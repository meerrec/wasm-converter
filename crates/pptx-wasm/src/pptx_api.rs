//! WASM-обвязка PPTX: открыть презентацию, померить слайды, собрать кадр.
//!
//! Раскладка считается один раз при открытии ([`pptx_open`]): слайд — одна
//! страница рендера, размер задан презентацией (`p:sldSz`), а не окном, поэтому
//! окно и зум приходят только с кадром. Как и у DOCX, презентация живёт в
//! `thread_local!` воркера: воркер однопоточный и обслуживает одно окно, а
//! команды кадра уезжают прямо в слот ring'а, минуя `postMessage`.
//!
//! Форма статистики сборки и разбор окна — общие с XLSX/DOCX: payload'ы живут
//! в `doc_converter_core::wire` и генерируются в TS из одного источника.

use std::cell::Cell;
use std::cell::RefCell;

use doc_converter_core::wire::{BuildStatsWire, ImageInfoWire, SlideInfoWire, ViewportWire};
use doc_converter_pptx::model::Presentation;
use doc_converter_pptx_render::{layout_slide, SlideLayout};
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::sab::SabRing;
use doc_converter_render::viewport::Viewport;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Открытая презентация: `None`, пока ничего не открыто.
    static PRES: RefCell<Option<OpenPres>> = const { RefCell::new(None) };
    /// Буфер кадра: переиспользуется, чтобы не аллоцировать на каждый кадр.
    static FRAME: RefCell<DisplayList> = RefCell::new(DisplayList::new());
    /// Байты кадра: сериализация тоже переиспользует буфер.
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Флаг отмены тяжёлой операции (ADR-0032): взводит сообщение `cancel`.
    static CANCEL: Cell<bool> = const { Cell::new(false) };
}

/// Взвести или снять флаг кооперативной отмены (ADR-0032).
///
/// Раскладка проверяет его между слайдами. Флаг одноразовый: операция,
/// которая его заметила, гасит его сама, — иначе отмена, пришедшая после
/// операции, ждала бы следующую.
#[wasm_bindgen]
pub fn set_cancel(flag: bool) {
    CANCEL.with(|cancel| cancel.set(flag));
}

/// Открытая презентация: разобранная модель и готовая раскладка слайдов.
struct OpenPres {
    /// Модель презентации: кадру она не нужна — раскладка уже посчитана, — но
    /// [`pptx_images`] и [`pptx_image_bytes`] берут из неё media-части.
    presentation: Presentation,
    /// Раскладка слайдов: из неё собирается каждый кадр.
    layouts: Vec<SlideLayout>,
}

/// Открыть презентацию из байтов: разобрать, разложить слайды, запомнить.
///
/// Раскладка считается здесь и один раз: размер слайда задан презентацией, а
/// не окном. Предыдущая презентация закрывается.
///
/// # Errors
/// Если байты не OOXML-пакет, `ppt/presentation.xml` не разбирается, в
/// презентации нет слайдов или раскладка упирается в [`Error`].
#[wasm_bindgen]
pub fn pptx_open(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    let presentation = doc_converter_pptx::open(bytes).map_err(to_js)?;
    install_presentation(presentation)
}

/// Раскладывает готовую презентацию, ставит её в `PRES` и сериализует ответ
/// открытия — общий хвост `pptx_open` и `stream_open_pptx` (ADR-0031).
pub(crate) fn install_presentation(presentation: Presentation) -> Result<JsValue, JsValue> {
    let mut layouts = Vec::with_capacity(presentation.slide_count());
    for index in 0..presentation.slide_count() {
        // Кооперативная отмена между слайдами (ADR-0032).
        if CANCEL.with(|cancelled| cancelled.get()) {
            // Флаг одноразовый: попытка раскладки его гасит — иначе отменённое
            // открытие отменяло бы и следующее.
            CANCEL.with(|cancelled| cancelled.set(false));
            return Err(JsValue::from_str("[cancelled] operation cancelled"));
        }
        layouts.push(layout_slide(&presentation, index).map_err(to_js)?);
    }
    let slides: Vec<SlideInfoWire> = layouts
        .iter()
        .enumerate()
        .map(|(index, layout)| SlideInfoWire {
            index,
            name: presentation.slide(index).map_or_else(
                || format!("Slide {}", index + 1),
                |slide| slide.name.clone(),
            ),
            width: layout.width,
            height: layout.height,
        })
        .collect();

    PRES.with(|pres| {
        *pres.borrow_mut() = Some(OpenPres {
            presentation,
            layouts,
        })
    });
    serde_wasm_bindgen::to_value(&slides).map_err(to_js)
}

/// Закрыть презентацию и освободить память.
#[wasm_bindgen]
pub fn pptx_close() {
    CANCEL.with(|cancel| cancel.set(false));
    PRES.with(|pres| *pres.borrow_mut() = None);
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// Открыта ли презентация. Нужно интерфейсу, чтобы не рисовать пустоту.
#[wasm_bindgen]
pub fn pptx_is_open() -> bool {
    PRES.with(|pres| pres.borrow().is_some())
}

/// Картинка презентации в том виде, в каком её видит интерфейс: id, MIME и
/// размер.
fn image_info(part: &doc_converter_pptx::model::ImagePart) -> ImageInfoWire {
    ImageInfoWire {
        id: part.id,
        mime: part.mime.clone(),
        byte_length: u32::try_from(part.bytes.len()).unwrap_or(u32::MAX),
    }
}

/// Картинки открытой презентации: id, MIME-тип и длина байтов.
///
/// Байты в список не входят: их забирает [`pptx_image_bytes`] по одному id,
/// чтобы не копировать разом всю media презентации. В списке только части, на
/// которые ссылается хотя бы один `p:pic`: остальные разбор не читает.
///
/// # Errors
/// Если презентация не открыта.
#[wasm_bindgen]
pub fn pptx_images() -> Result<JsValue, JsValue> {
    PRES.with(|pres| {
        let pres = pres.borrow();
        let open = pres
            .as_ref()
            .ok_or_else(|| to_js("no presentation is open"))?;
        let images: Vec<ImageInfoWire> = open.presentation.images.iter().map(image_info).collect();
        serde_wasm_bindgen::to_value(&images).map_err(to_js)
    })
}

/// Байты картинки по её id.
///
/// Возвращается `Uint8Array` — копия байтов из памяти wasm; буфер принадлежит
/// интерфейсу, и дальше его можно отдать `postMessage` с transferables.
///
/// # Errors
/// Если презентация не открыта или картинки с таким id нет.
#[wasm_bindgen]
pub fn pptx_image_bytes(id: u32) -> Result<js_sys::Uint8Array, JsValue> {
    PRES.with(|pres| {
        let pres = pres.borrow();
        let open = pres
            .as_ref()
            .ok_or_else(|| to_js("no presentation is open"))?;
        let part = open
            .presentation
            .image(id)
            .ok_or_else(|| to_js(format!("image {id} does not exist")))?;
        Ok(js_sys::Uint8Array::from(part.bytes.as_slice()))
    })
}

/// Собрать кадр слайда и отдать его байты транспорту вызывающего.
///
/// Слайд — одна страница рендера: кадр несёт сам слайд, а не стопку, поэтому
/// `viewport.x`/`viewport.y` — прокрутка внутри слайда (обычно нулевая), а не
/// смещение по стопке. Сборка и сериализация у обоих путей (SAB и без него)
/// общие — разница только в том, куда байты деваются.
///
/// # Errors
/// Если презентация не открыта, слайда с таким индексом нет, окно описано
/// неверно или запись не удалась.
fn with_frame_bytes(
    slide: u32,
    viewport: Viewport,
    write: impl FnOnce(&[u8]) -> Result<bool, JsValue>,
) -> Result<BuildStatsWire, JsValue> {
    let started = js_sys::Date::now();
    PRES.with(|pres| -> Result<BuildStatsWire, JsValue> {
        let pres = pres.borrow();
        let open = pres
            .as_ref()
            .ok_or_else(|| to_js("no presentation is open"))?;
        let index = usize::try_from(slide).map_err(|_| to_js("slide index out of range"))?;
        let layout = open
            .layouts
            .get(index)
            .ok_or_else(|| to_js(format!("slide index {index} is out of range")))?;

        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            doc_converter_pptx_render::render::build(layout, &viewport, &mut frame);
            let cmds = u32::try_from(frame.len()).unwrap_or(u32::MAX);

            SCRATCH.with(|scratch| {
                let mut scratch = scratch.borrow_mut();
                frame.to_bytes_into(&mut scratch);
                let bytes = u32::try_from(scratch.len()).unwrap_or(u32::MAX);
                Ok(BuildStatsWire {
                    written: write(&scratch)?,
                    cmds,
                    bytes,
                    build_ms: js_sys::Date::now() - started,
                })
            })
        })
    })
}

/// Собрать кадр слайда и вернуть его байты.
///
/// Годится тому, кто рисует кадр в другом модуле wasm: кадр самодостаточен.
/// Просмотрщик ходит через [`pptx_build_display_list_into`] — там буфер
/// вызывающего переиспользуется, а тут кадр каждый раз новый массив.
///
/// # Errors
/// Если презентация не открыта, слайда с таким индексом нет или окно описано
/// неверно.
#[wasm_bindgen]
pub fn pptx_build_display_list(viewport: JsValue, slide: u32) -> Result<Vec<u8>, JsValue> {
    let viewport = viewport_of(viewport)?;
    let mut out = Vec::new();
    with_frame_bytes(slide, viewport, |bytes| {
        out.extend_from_slice(bytes);
        Ok(true)
    })?;
    Ok(out)
}

/// Собрать кадр слайда сразу в слот ring'а.
///
/// Возвращает `written: false`, если свободного слота не было: кадр теряется,
/// но следующий за ним рисуется — так и задумано, догонять прокрутку
/// устаревшими кадрами незачем. Второй случай `written: false` — кадр больше
/// слота: `bytes` скажет, насколько.
///
/// # Errors
/// Если презентация не открыта, слайда с таким индексом нет, окно описано
/// неверно или SAB не тот.
#[wasm_bindgen]
pub fn pptx_build_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
    viewport: JsValue,
    slide: u32,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let stats = with_frame_bytes(slide, viewport, |bytes| {
        let mut ring = SabRing::from_sab(sab, slot_capacity as usize)?;
        ring.try_write(bytes)
    })?;
    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Собрать кадр слайда в буфер вызывающего — путь без `SharedArrayBuffer`.
///
/// Байты пишутся в готовый массив: воркер держит два и заполняет то один, то
/// другой, поэтому на кадр нет ни аллокации под сам кадр, ни копии из памяти
/// wasm в JS (копия в память painter'а неизбежна: он живёт в другом модуле).
/// `written: false` вместе с `bytes` больше длины буфера означает, что буфер
/// мал: воркер вырастит его и повторит сборку.
///
/// # Errors
/// Если презентация не открыта, слайда с таким индексом нет или окно описано
/// неверно.
#[wasm_bindgen]
pub fn pptx_build_display_list_into(
    out: &js_sys::Uint8Array,
    viewport: JsValue,
    slide: u32,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let stats = with_frame_bytes(slide, viewport, |bytes| {
        let length = bytes.len() as u32;
        if length > out.length() {
            return Ok(false);
        }
        out.subarray(0, length).copy_from(bytes);
        Ok(true)
    })?;
    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Открыть презентацию из байтов, отдав их без копирования.
///
/// # Errors
/// То же, что у [`pptx_open`].
#[wasm_bindgen]
pub fn pptx_open_bytes(bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
    pptx_open(bytes.to_vec())
}

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
///
/// Общая с [`crate::export_api`]: у обоих путей ошибки одного вида.
pub(crate) fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Перевести окно из JS в представление рендера.
///
/// У PPTX `scrollX`/`scrollY` отсчитываются от левого верхнего угла слайда;
/// при показе слайда целиком они нулевые.
fn viewport_of(value: JsValue) -> Result<Viewport, JsValue> {
    let js: ViewportWire = serde_wasm_bindgen::from_value(value).map_err(to_js)?;
    Ok(Viewport::new(
        js.scroll_x,
        js.scroll_y,
        js.width,
        js.height,
        js.scale,
    ))
}
