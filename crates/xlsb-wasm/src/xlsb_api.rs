//! WASM-обвязка XLSB: открыть книгу, померить лист, собрать кадр.
//!
//! Книга живёт в `thread_local!` воркера — ровно одна на поток: воркер
//! однопоточный и обслуживает одно окно. DisplayList собирается сразу в слот
//! ring'а, минуя JS: гонять сотни килобайт команд через `postMessage` значило
//! бы платить за копирование на каждом кадре. Кадр рисует общий
//! `doc_converter_xlsx::paint`: XLSB доводится до той же модели книги, что и
//! XLSX, поэтому отдельной ветки рендера на формат нет.
//!
//! Форма статистики сборки и разбор окна — общие с XLSX/DOCX/PPTX: payload'ы
//! живут в `doc_converter_core::wire` и генерируются в TS из одного источника.

use std::cell::Cell;
use std::cell::RefCell;

use doc_converter_core::wire::{ImageInfoWire, RenderOptionsWire, SheetInfoWire};
use doc_converter_render::display_list::DisplayList;
use doc_converter_wasm_common::frame;
use doc_converter_xlsx::paint::{self, PaintOptions};
use doc_converter_xlsx::Viewport;
use doc_converter_xlsx::Workbook;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Открытая книга: `None`, пока ничего не открыто.
    static DOC: RefCell<Option<Workbook>> = const { RefCell::new(None) };
    /// Буфер кадра: переиспользуется, чтобы не аллоцировать на каждый кадр.
    static FRAME: RefCell<DisplayList> = RefCell::new(DisplayList::new());
    /// Байты кадра: сериализация тоже переиспользует буфер — на кадр глубиной
    /// в сотни килобайт `to_bytes()` был бы лишней аллокацией и копией.
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Флаг отмены тяжёлой операции (ADR-0032): взводит сообщение `cancel`.
    static CANCEL: Cell<bool> = const { Cell::new(false) };
}

/// Взвести или снять флаг кооперативной отмены (ADR-0032).
///
/// Сборка кадра проверяет его между полосами видимых строк. Флаг одноразовый:
/// операция, которая его заметила, гасит его сама, — иначе отмена, пришедшая
/// после операции, ждала бы следующую.
#[wasm_bindgen]
pub fn set_cancel(flag: bool) {
    CANCEL.with(|cancel| cancel.set(flag));
}

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
pub(crate) fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Открыть книгу из байтов. Предыдущая книга закрывается.
///
/// # Errors
/// Если байты не OOXML-пакет или какая-то из бинарных частей не разбирается.
#[wasm_bindgen]
pub fn xlsb_open(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    CANCEL.with(|cancel| cancel.set(false));
    let book = doc_converter_xlsb::open(bytes).map_err(to_js)?;
    install_book(book)
}

/// Ставит готовую книгу в `DOC` и сериализует ответ открытия — общий хвост
/// `xlsb_open` и `stream_open_xlsb` (ADR-0031).
pub(crate) fn install_book(book: Workbook) -> Result<JsValue, JsValue> {
    let sheets: Vec<SheetInfoWire> = book
        .sheets()
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let layout = doc_converter_xlsx::SheetLayout::new(sheet);
            let pane = sheet.view.pane.filter(|pane| !pane.is_empty());
            SheetInfoWire {
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
pub fn xlsb_close() {
    CANCEL.with(|cancel| cancel.set(false));
    DOC.with(|doc| *doc.borrow_mut() = None);
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// Открыта ли книга. Нужно интерфейсу, чтобы не рисовать пустоту.
#[wasm_bindgen]
pub fn xlsb_is_open() -> bool {
    DOC.with(|doc| doc.borrow().is_some())
}

/// Картинка книги в том виде, в каком её видит интерфейс: id, MIME и размер.
fn image_info(image: &doc_converter_xlsx::WorkbookImage) -> ImageInfoWire {
    ImageInfoWire {
        id: image.id,
        mime: image.mime.clone(),
        byte_length: u32::try_from(image.bytes.len()).unwrap_or(u32::MAX),
    }
}

/// Картинки открытой книги: id, MIME-тип и длина байтов.
///
/// Байты в список не входят: их забирает [`xlsb_image_bytes`] по одному id,
/// чтобы не копировать разом всю media книги. Первый срез XLSB картинок не
/// читает, поэтому список обычно пуст.
///
/// # Errors
/// Если книга не открыта.
#[wasm_bindgen]
pub fn xlsb_images() -> Result<JsValue, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let images: Vec<ImageInfoWire> = book.images().iter().map(image_info).collect();
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
pub fn xlsb_image_bytes(id: u32) -> Result<js_sys::Uint8Array, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let image = book
            .image(id)
            .ok_or_else(|| to_js(format!("image {id} does not exist")))?;
        Ok(js_sys::Uint8Array::from(image.bytes.as_slice()))
    })
}

/// Перевести окно из JS в представление рендера. Форма окна общая с XLSX,
/// разбор — общий в `doc_converter_wasm_common::frame`.
fn viewport_of(value: JsValue) -> Result<Viewport, JsValue> {
    frame::viewport_of(value)
}

fn options_of(value: JsValue) -> Result<PaintOptions, JsValue> {
    let js: RenderOptionsWire = serde_wasm_bindgen::from_value(value).map_err(to_js)?;
    Ok(PaintOptions {
        show_grid: js.show_grid,
        show_headers: js.show_headers,
        dark: js.dark,
        ..PaintOptions::default()
    })
}

/// Собрать кадр листа в `frame` и вернуть число команд.
///
/// CANCEL-обвязка (ADR-0032) и маппинг ошибок формата остаются здесь, а
/// сериализация и транспорт общие у всех форматов и живут в
/// `doc_converter_wasm_common::frame`.
///
/// # Errors
/// Если книга не открыта или листа с таким номером нет.
fn build_frame(
    sheet: usize,
    viewport: Viewport,
    options: &PaintOptions,
    frame: &mut DisplayList,
) -> Result<u32, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;

        let built =
            CANCEL.with(|cancelled| paint::build(book, sheet, viewport, options, frame, cancelled));
        // Флаг одноразовый (ADR-0032): попытка сборки его гасит — иначе
        // отменённый кадр отменял бы и следующий.
        CANCEL.with(|cancelled| cancelled.set(false));
        built.map_err(|e| {
            if matches!(e, doc_converter_xlsx::XlsxError::Cancelled) {
                JsValue::from_str("[cancelled] operation cancelled")
            } else {
                to_js(e)
            }
        })?;
        Ok(u32::try_from(frame.len()).unwrap_or(u32::MAX))
    })
}

/// Собрать кадр листа сразу в слот ring'а.
///
/// Возвращает `written: false`, если свободного слота не было: кадр теряется,
/// но следующий за ним рисуется — так и задумано, догонять прокрутку
/// устаревшими кадрами незачем. Второй случай `written: false` — кадр больше
/// слота: `bytes` в статистике скажет, насколько.
///
/// # Errors
/// Если книга не открыта, листа нет, окно описано неверно или SAB не тот.
#[wasm_bindgen]
pub fn xlsb_build_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
    sheet: usize,
    viewport: JsValue,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let options = options_of(options)?;
    let stats = frame::build_sab(sab, slot_capacity, &FRAME, &SCRATCH, |frame| {
        build_frame(sheet, viewport, &options, frame)
    })?;
    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Собрать кадр листа в буфер вызывающего — путь без `SharedArrayBuffer`.
///
/// Буфер переиспользуется: воркер держит два и заполняет то один, то другой,
/// поэтому на кадр нет ни аллокации, ни копии из памяти wasm в JS — только
/// запись в готовый массив. `written: false` вместе с `bytes` больше длины
/// буфера означает, что буфер мал: воркер вырастит его и повторит сборку.
///
/// # Errors
/// Если книга не открыта, листа нет или окно описано неверно.
#[wasm_bindgen]
pub fn xlsb_build_display_list_into(
    out: &js_sys::Uint8Array,
    sheet: usize,
    viewport: JsValue,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let options = options_of(options)?;
    let stats = frame::build_into(out, &FRAME, &SCRATCH, |frame| {
        build_frame(sheet, viewport, &options, frame)
    })?;
    serde_wasm_bindgen::to_value(&stats).map_err(to_js)
}

/// Открыть книгу из байтов, отдав их без копирования.
///
/// # Errors
/// То же, что у [`xlsb_open`].
#[wasm_bindgen]
pub fn xlsb_open_bytes(bytes: js_sys::Uint8Array) -> Result<JsValue, JsValue> {
    xlsb_open(bytes.to_vec())
}
