//! WASM-обвязка DOCX: открыть документ, померить страницы, собрать кадр.
//!
//! Раскладка считается один раз при открытии ([`docx_open`]): она дорогая —
//! каскад стилей и перенос строк по метрикам шрифта, — а документ после
//! открытия не меняется, и кадр любой страницы собирается из готового
//! [`PageLayout`]. Как и у XLSX, документ живёт в `thread_local!` воркера:
//! воркер однопоточный и обслуживает одно окно, а команды кадра уезжают прямо
//! в слот ring'а, минуя `postMessage`.
//!
//! Форма статистики сборки и разбор окна — общие с XLSX: payload'ы живут в
//! `doc_converter_core::wire` и генерируются в TS из одного источника.

use std::cell::Cell;
use std::cell::RefCell;

use doc_converter_core::wire::{ImageInfoWire, PageInfoWire};
use doc_converter_docx::layout::layout_document_with_cancel;
use doc_converter_docx::{Document, LayoutError, LayoutOptions, MediaPart, PageLayout};
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};
use doc_converter_render::viewport::Viewport;
use doc_converter_wasm_common::frame;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Открытый документ: `None`, пока ничего не открыто.
    static DOC: RefCell<Option<OpenDoc>> = const { RefCell::new(None) };
    /// Буфер кадра: переиспользуется, чтобы не аллоцировать на каждый кадр.
    static FRAME: RefCell<DisplayList> = RefCell::new(DisplayList::new());
    /// Байты кадра: сериализация тоже переиспользует буфер.
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Флаг отмены тяжёлой операции (ADR-0032): взводит сообщение `cancel`.
    static CANCEL: Cell<bool> = const { Cell::new(false) };
}

/// Взвести или снять флаг кооперативной отмены (ADR-0032).
///
/// Раскладка проверяет его между блоками тела. Флаг одноразовый: операция,
/// которая его заметила, гасит его сама, — иначе отмена, пришедшая после
/// операции, ждала бы следующую.
#[wasm_bindgen]
pub fn set_cancel(flag: bool) {
    CANCEL.with(|cancel| cancel.set(flag));
}

/// Открытый документ: разобранная модель, реестр шрифтов и готовая раскладка.
struct OpenDoc {
    /// Модель документа: кадру она не нужна — раскладка уже посчитана, — но
    /// [`docx_images`] и [`docx_image_bytes`] берут из неё медиачасти, а
    /// поиску по тексту она понадобится дальше.
    document: Document,
    /// Реестр, которым мерялась раскладка; остаётся, чтобы повторная раскладка
    /// (другой кегль, другой шрифт) не регистрировала шрифты заново.
    #[allow(dead_code)]
    fonts: FontRegistry,
    /// Раскладка страниц: из неё собирается каждый кадр.
    layout: PageLayout,
}

/// Открыть документ из байтов: разобрать, разложить по страницам, запомнить.
///
/// Раскладка (`LayoutOptions::default()`) считается здесь и один раз: ширина
/// полосы набора задана страницей документа, а не окном, поэтому окно и зум
/// на неё не влияют и приходят только с кадром. Предыдущий документ
/// закрывается.
///
/// # Errors
/// Если байты не OOXML-пакет, `word/document.xml` не разбирается или раскладка
/// упирается в [`doc_converter_docx::LayoutError`].
#[wasm_bindgen]
pub fn docx_open(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    let document = doc_converter_docx::open(bytes).map_err(to_js)?;
    install_document(document)
}

/// Раскладывает готовый документ, ставит его в `DOC` и сериализует ответ
/// открытия — общий хвост `docx_open` и `stream_open_docx` (ADR-0031).
pub(crate) fn install_document(document: Document) -> Result<JsValue, JsValue> {
    // Размер LRU-кэша метрик — общий `DEFAULT_FONT_CACHE` из
    // `doc_converter_render::font`.
    let mut fonts = FontRegistry::new(DEFAULT_FONT_CACHE);
    let laid_out = CANCEL.with(|cancelled| {
        layout_document_with_cancel(&document, &LayoutOptions::default(), &mut fonts, cancelled)
    });
    // Флаг одноразовый (ADR-0032): попытка раскладки его гасит — иначе
    // отменённое открытие отменяло бы и следующее.
    CANCEL.with(|cancelled| cancelled.set(false));
    let layout = laid_out.map_err(|e| {
        if matches!(e, LayoutError::Cancelled) {
            JsValue::from_str("[cancelled] operation cancelled")
        } else {
            to_js(e)
        }
    })?;
    let pages: Vec<PageInfoWire> = layout
        .pages
        .iter()
        .map(|page| PageInfoWire {
            width: page.width,
            height: page.height,
        })
        .collect();

    DOC.with(|doc| {
        *doc.borrow_mut() = Some(OpenDoc {
            document,
            fonts,
            layout,
        })
    });
    serde_wasm_bindgen::to_value(&pages).map_err(to_js)
}

/// Закрыть документ и освободить память.
#[wasm_bindgen]
pub fn docx_close() {
    CANCEL.with(|cancel| cancel.set(false));
    DOC.with(|doc| *doc.borrow_mut() = None);
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// Открыт ли документ. Нужно интерфейсу, чтобы не рисовать пустоту.
#[wasm_bindgen]
pub fn docx_is_open() -> bool {
    DOC.with(|doc| doc.borrow().is_some())
}

/// Картинка документа в том виде, в каком её видит интерфейс: id, MIME и размер.
fn image_info(part: &MediaPart) -> ImageInfoWire {
    ImageInfoWire {
        id: part.id,
        mime: part.mime.clone(),
        byte_length: u32::try_from(part.bytes.len()).unwrap_or(u32::MAX),
    }
}

/// Картинки открытого документа: id, MIME-тип и длина байтов.
///
/// Байты в список не входят: их забирает [`docx_image_bytes`] по одному id,
/// чтобы не копировать разом всю media документа. В списке только части, на
/// которые ссылается хотя бы один рисунок: остальные разбор не читает.
///
/// # Errors
/// Если документ не открыт.
#[wasm_bindgen]
pub fn docx_images() -> Result<JsValue, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let open = doc.as_ref().ok_or_else(|| to_js("no document is open"))?;
        let images: Vec<ImageInfoWire> = open.document.media().iter().map(image_info).collect();
        serde_wasm_bindgen::to_value(&images).map_err(to_js)
    })
}

/// Байты картинки по её id.
///
/// Возвращается `Uint8Array` — копия байтов из памяти wasm; буфер принадлежит
/// интерфейсу, и дальше его можно отдать `postMessage` с transferables.
///
/// # Errors
/// Если документ не открыт или картинки с таким id нет.
#[wasm_bindgen]
pub fn docx_image_bytes(id: u32) -> Result<js_sys::Uint8Array, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let open = doc.as_ref().ok_or_else(|| to_js("no document is open"))?;
        let part = open
            .document
            .media_part(id)
            .ok_or_else(|| to_js(format!("image {id} does not exist")))?;
        Ok(js_sys::Uint8Array::from(part.bytes.as_slice()))
    })
}

/// Собрать кадр стопки страниц в `frame` и вернуть число команд.
///
/// Кадр несёт страницы вокруг окна, а не одну текущую: прокрутка отсчитывается
/// от верха стопки, страницы стоят друг под другом через `gap` (см.
/// [`doc_converter_docx::render::build_stack`]). Пустой документ даёт пустой
/// кадр, а не ошибку. Сериализация и транспорт общие с XLSX/PPTX —
/// `doc_converter_wasm_common::frame`, — сюда входит только сборка формата.
///
/// # Errors
/// Если документ не открыт.
fn build_frame(viewport: Viewport, gap: f32, frame: &mut DisplayList) -> Result<u32, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let open = doc.as_ref().ok_or_else(|| to_js("no document is open"))?;
        doc_converter_docx::render::build_stack(&open.layout, viewport, gap, frame);
        Ok(u32::try_from(frame.len()).unwrap_or(u32::MAX))
    })
}

/// Собрать кадр стопки страниц сразу в слот ring'а.
///
/// Возвращает `written: false`, если свободного слота не было: кадр теряется,
/// но следующий за ним рисуется — так и задумано, догонять прокрутку
/// устаревшими кадрами незачем. Второй случай `written: false` — кадр больше
/// слота: `bytes` скажет, насколько.
///
/// # Errors
/// Если документ не открыт, окно описано неверно или SAB не тот.
#[wasm_bindgen]
pub fn docx_build_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
    viewport: JsValue,
    gap: f32,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let stats = frame::build_sab(sab, slot_capacity, &FRAME, &SCRATCH, |frame| {
        build_frame(viewport, gap, frame)
    })?;
    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Собрать кадр стопки страниц в буфер вызывающего — путь без `SharedArrayBuffer`.
///
/// Байты пишутся в готовый массив: воркер держит два и заполняет то один, то
/// другой, поэтому на кадр нет ни аллокации под сам кадр, ни копии из памяти
/// wasm в JS (копия в память painter'а неизбежна: он живёт в другом модуле).
/// `written: false` вместе с `bytes` больше длины буфера означает, что буфер
/// мал: воркер вырастит его и повторит сборку.
///
/// # Errors
/// Если документ не открыт или окно описано неверно.
#[wasm_bindgen]
pub fn docx_build_display_list_into(
    out: &js_sys::Uint8Array,
    viewport: JsValue,
    gap: f32,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let stats = frame::build_into(out, &FRAME, &SCRATCH, |frame| {
        build_frame(viewport, gap, frame)
    })?;
    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Открыть документ из байтов, отдав их без копирования.
///
/// # Errors
/// То же, что у [`docx_open`].
#[wasm_bindgen]
pub fn docx_open_bytes(bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
    docx_open(bytes.to_vec())
}

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
///
/// Общая с [`crate::export_api`]: у обоих путей ошибки одного вида.
pub(crate) fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Доступ к раскладке открытого документа — для [`crate::text_api`].
///
/// `None` — документ не открыт. Замыкание выполняется под заимствованием
/// `DOC`: вызывающий не должен сам трогать `DOC` внутри.
pub(crate) fn with_layout<T>(read: impl FnOnce(&PageLayout) -> T) -> Option<T> {
    DOC.with(|doc| doc.borrow().as_ref().map(|doc| read(&doc.layout)))
}

/// Перевести окно из JS в представление рендера. Форма окна общая с XLSX/PPTX,
/// разбор — общий в `doc_converter_wasm_common::frame`.
///
/// У DOCX `scrollY` отсчитывается от верха стопки страниц, а не от верха
/// текущей: страницы стоят друг под другом, и кадр несёт те из них, что видны.
fn viewport_of(value: JsValue) -> Result<Viewport, JsValue> {
    frame::viewport_of(value)
}
