//! WASM-обвязка XLSX: открыть книгу, померить лист, собрать кадр.
//!
//! Книга живёт в `thread_local!` воркера — ровно одна на поток: воркер
//! однопоточный и обслуживает одно окно. DisplayList собирается сразу в слот
//! ring'а, минуя JS: гонять сотни килобайт команд через `postMessage` значило
//! бы платить за копирование на каждом кадре.

use std::cell::RefCell;

use doc_converter_render::display_list::DisplayList;
use doc_converter_render::sab::SabRing;
use doc_converter_xlsx::paint::{self, PaintOptions, Viewport};
use doc_converter_xlsx::Workbook;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

thread_local! {
    /// Открытая книга: `None`, пока ничего не открыто.
    static DOC: RefCell<Option<Workbook>> = const { RefCell::new(None) };
    /// Буфер кадра: переиспользуется, чтобы не аллоцировать на каждый кадр.
    static FRAME: RefCell<DisplayList> = RefCell::new(DisplayList::new());
}

/// Окно — то, что приходит из воркера.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportJs {
    #[serde(default)]
    scroll_x: f32,
    #[serde(default)]
    scroll_y: f32,
    width: f32,
    height: f32,
    #[serde(default = "one")]
    scale: f32,
}

/// Настройки рисования. Поля необязательны: интерфейс может прислать только
/// то, что меняет, а остальное остаётся как в Excel.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsJs {
    #[serde(default = "yes")]
    show_grid: bool,
    #[serde(default = "yes")]
    show_headers: bool,
    /// Тёмное оформление; `false` — светлое, как в Excel.
    #[serde(default)]
    dark: bool,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// Описание листа для интерфейса: имя, размеры и закрепления.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetInfo {
    name: String,
    index: usize,
    hidden: bool,
    /// Размер листа в пикселях раскладки — им распоряжается полоса прокрутки.
    width: f32,
    height: f32,
    frozen_cols: u32,
    frozen_rows: u32,
    /// Ширина и высота закреплённой части.
    frozen_width: f32,
    frozen_height: f32,
}

/// Что вышло из попытки собрать кадр.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStats {
    /// Кадр уложился в слот ring'а.
    written: bool,
    cmds: u32,
    build_ms: f64,
}

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Открыть книгу из байтов. Предыдущая книга закрывается.
///
/// # Errors
/// Если байты не OOXML-пакет или какая-то из частей не разбирается.
#[wasm_bindgen]
pub fn xlsx_open(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    let book = doc_converter_xlsx::open(bytes).map_err(to_js)?;
    let sheets: Vec<SheetInfo> = book
        .sheets()
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let layout = doc_converter_xlsx::SheetLayout::new(sheet);
            let pane = sheet.view.pane.filter(|pane| !pane.is_empty());
            SheetInfo {
                name: sheet.meta.name.clone(),
                index,
                hidden: sheet.meta.state != doc_converter_xlsx::SheetState::Visible,
                width: layout.total_width(),
                height: layout.total_height(),
                frozen_cols: pane.map_or(0, |pane| pane.cols),
                frozen_rows: pane.map_or(0, |pane| pane.rows),
                frozen_width: pane.map_or(0.0, |pane| layout.column_x(pane.cols)),
                frozen_height: pane.map_or(0.0, |pane| layout.row_y(pane.rows)),
            }
        })
        .collect();

    DOC.with(|doc| *doc.borrow_mut() = Some(book));
    serde_wasm_bindgen::to_value(&sheets).map_err(to_js)
}

/// Закрыть книгу и освободить память.
#[wasm_bindgen]
pub fn xlsx_close() {
    DOC.with(|doc| *doc.borrow_mut() = None);
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// Число открытых книг: 1 или 0. Нужно интерфейсу, чтобы не рисовать пустоту.
#[wasm_bindgen]
pub fn xlsx_is_open() -> bool {
    DOC.with(|doc| doc.borrow().is_some())
}

/// Картинка книги в том виде, в каком её видит интерфейс: id, MIME и размер.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    /// id, которым помечены картинки листов и команды `Image` в кадре.
    id: u32,
    /// MIME-тип байтов — интерфейс собирает из них `Blob`.
    mime: String,
    /// Длина байтов в [`xlsx_image_bytes`].
    byte_length: u32,
}

impl ImageInfo {
    fn of(image: &doc_converter_xlsx::WorkbookImage) -> Self {
        Self {
            id: image.id,
            mime: image.mime.clone(),
            byte_length: u32::try_from(image.bytes.len()).unwrap_or(u32::MAX),
        }
    }
}

/// Картинки открытой книги: id, MIME-тип и длина байтов.
///
/// Байты в список не входят: их забирает [`xlsx_image_bytes`] по одному id,
/// чтобы не копировать разом всю media книги.
///
/// # Errors
/// Если книга не открыта.
#[wasm_bindgen]
pub fn xlsx_images() -> Result<JsValue, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let images: Vec<ImageInfo> = book.images().iter().map(ImageInfo::of).collect();
        serde_wasm_bindgen::to_value(&images).map_err(to_js)
    })
}

/// Байты картинки по её id.
///
/// Возвращается `Uint8Array` — копия байтов из памяти wasm; буфер принадлежит
/// интерфейсу, и дальше его можно отдать `postMessage` с transferables.
///
/// # Errors
/// Если книга не открыта или картинки с таким id нет.
#[wasm_bindgen]
pub fn xlsx_image_bytes(id: u32) -> Result<js_sys::Uint8Array, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let image = book
            .image(id)
            .ok_or_else(|| to_js(format!("image {id} does not exist")))?;
        Ok(js_sys::Uint8Array::from(image.bytes.as_slice()))
    })
}

/// Перевести окно из JS в представление рендера.
fn viewport_of(value: JsValue) -> Result<Viewport, JsValue> {
    let js: ViewportJs = serde_wasm_bindgen::from_value(value).map_err(to_js)?;
    Ok(Viewport {
        scroll_x: js.scroll_x,
        scroll_y: js.scroll_y,
        width: js.width,
        height: js.height,
        scale: js.scale,
    })
}

fn options_of(value: JsValue) -> Result<PaintOptions, JsValue> {
    let js: OptionsJs = serde_wasm_bindgen::from_value(value).map_err(to_js)?;
    Ok(PaintOptions {
        show_grid: js.show_grid,
        show_headers: js.show_headers,
        dark: js.dark,
        ..PaintOptions::default()
    })
}

/// Собрать кадр листа в буфер и вернуть его байты.
///
/// Нужен там, где `SharedArrayBuffer` недоступен: при отсутствии
/// COOP/COEP-заголовков кадр уезжает в воркер копией.
///
/// # Errors
/// Если книга не открыта, листа с таким номером нет или окно описано неверно.
#[wasm_bindgen]
pub fn xlsx_build_display_list(
    sheet: usize,
    viewport: JsValue,
    options: JsValue,
) -> Result<Vec<u8>, JsValue> {
    let viewport = viewport_of(viewport)?;
    let options = options_of(options)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;

        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            paint::build(book, sheet, viewport, &options, &mut frame);
            Ok(frame.to_bytes())
        })
    })
}

/// Собрать кадр листа сразу в слот ring'а.
///
/// Возвращает `written: false`, если свободного слота не было: кадр теряется,
/// но следующий за ним рисуется — так и задумано, догонять прокрутку
/// устаревшими кадрами незачем.
///
/// # Errors
/// Если книга не открыта, листа нет, окно описано неверно или SAB не тот.
#[wasm_bindgen]
pub fn xlsx_build_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
    sheet: usize,
    viewport: JsValue,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let options = options_of(options)?;
    let started = now();

    let stats = DOC.with(|doc| -> Result<BuildStats, JsValue> {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;

        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            paint::build(book, sheet, viewport, &options, &mut frame);
            let cmds = u32::try_from(frame.len()).unwrap_or(u32::MAX);

            let mut ring = SabRing::from_sab(sab, slot_capacity as usize)?;
            let written = ring.try_write(&frame.to_bytes())?;
            Ok(BuildStats {
                written,
                cmds,
                build_ms: now() - started,
            })
        })
    })?;

    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Ячейка под точкой окна — для подсказки адреса и будущего выделения.
///
/// # Errors
/// Если книга не открыта, листа нет или точка описана неверно.
#[wasm_bindgen]
pub fn xlsx_hit_test(sheet: usize, viewport: JsValue, x: f32, y: f32) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let cell = paint::hit_test(sheet, viewport, x, y);
        serde_wasm_bindgen::to_value(&(cell.row, cell.col)).map_err(to_js)
    })
}

/// Гиперссылка в том виде, в каком её видит интерфейс.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HyperlinkInfo {
    /// Адрес перехода: URL, почта, путь к файлу или ссылка внутри книги.
    target: String,
    /// Подпись, которую показывает Excel (`display`).
    display: Option<String>,
    /// Всплывающая подсказка (`tooltip`).
    tooltip: Option<String>,
}

impl HyperlinkInfo {
    /// Описание ссылки для интерфейса; `None` — цель не разрешилась
    /// (`HyperlinkTarget::Broken`), переходить некуда.
    fn of(link: &doc_converter_xlsx::Hyperlink) -> Option<Self> {
        Some(Self {
            target: link.target.address()?.to_owned(),
            display: link.display.clone(),
            tooltip: link.tooltip.clone(),
        })
    }
}

/// Гиперссылка под точкой окна: `target`, `display`, `tooltip` или `null`.
///
/// Отдельный экспорт, а не расширение [`xlsx_hit_test`]: у того уже есть
/// потребители, которым нужна пара координат ячейки, и смена формы ответа
/// сломала бы их. Геометрия общая — [`paint::hit_test`], — так что попадание
/// здесь ровно то же, что и у `xlsx_hit_test`.
///
/// # Errors
/// Если книга не открыта, листа нет или точка описана неверно.
#[wasm_bindgen]
pub fn xlsx_hyperlink_at(
    sheet: usize,
    viewport: JsValue,
    x: f32,
    y: f32,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let link = sheet
            .hyperlink_at(paint::hit_test(sheet, viewport, x, y))
            .and_then(HyperlinkInfo::of);
        // `None` уезжает в JS как `null`: отсутствие ссылки — не ошибка.
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true);
        link.serialize(&serializer).map_err(to_js)
    })
}

/// Открыть книгу из байтов, отдав их без копирования.
///
/// # Errors
/// То же, что у [`xlsx_open`].
#[wasm_bindgen]
pub fn xlsx_open_bytes(bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
    xlsx_open(bytes.to_vec())
}

fn now() -> f64 {
    js_sys::Date::now()
}
