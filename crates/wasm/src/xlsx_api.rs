//! WASM-обвязка XLSX: открыть книгу, померить лист, собрать кадр.
//!
//! Книга живёт в `thread_local!` воркера — ровно одна на поток: воркер
//! однопоточный и обслуживает одно окно. DisplayList собирается сразу в слот
//! ring'а, минуя JS: гонять сотни килобайт команд через `postMessage` значило
//! бы платить за копирование на каждом кадре.

use std::cell::Cell;
use std::cell::RefCell;

use doc_converter_core::wire::{
    BBoxWire, CellAddressWire, CellEditWire, CellRangeWire, HyperlinkInfoWire, ImageInfoWire,
    RenderOptionsWire, SelectionWire, SheetHitWire, SheetInfoWire,
};
use doc_converter_render::display_list::DisplayList;
use doc_converter_wasm_common::frame;
use doc_converter_xlsx::paint::{self, PaintOptions};
use doc_converter_xlsx::Viewport;
use doc_converter_xlsx::{CellRef, Workbook};
use serde::Serialize;
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
/// Если байты не OOXML-пакет или какая-то из частей не разбирается.
#[wasm_bindgen]
pub fn xlsx_open(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    CANCEL.with(|cancel| cancel.set(false));
    let book = doc_converter_xlsx::open(bytes).map_err(to_js)?;
    install_book(book)
}

/// Ставит готовую книгу в `DOC` и сериализует ответ открытия — общий хвост
/// `xlsx_open` и `stream_open_xlsx` (ADR-0031).
pub(crate) fn install_book(book: doc_converter_xlsx::Workbook) -> Result<JsValue, JsValue> {
    let sheets: Vec<SheetInfoWire> = book
        .sheets()
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let layout = doc_converter_xlsx::SheetLayout::full(sheet);
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
pub fn xlsx_close() {
    CANCEL.with(|cancel| cancel.set(false));
    DOC.with(|doc| *doc.borrow_mut() = None);
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// Число открытых книг: 1 или 0. Нужно интерфейсу, чтобы не рисовать пустоту.
#[wasm_bindgen]
pub fn xlsx_is_open() -> bool {
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

/// Перевести окно из JS в представление рендера. Форма окна общая с DOCX,
/// разбор — общий в `doc_converter_wasm_common::frame`.
pub(crate) fn viewport_of(value: JsValue) -> Result<Viewport, JsValue> {
    frame::viewport_of(value)
}

fn options_of(
    value: JsValue,
) -> Result<(PaintOptions, Option<doc_converter_xlsx::Selection>), JsValue> {
    let js: RenderOptionsWire = serde_wasm_bindgen::from_value(value).map_err(to_js)?;
    let selection = js.selection.map(|selection| doc_converter_xlsx::Selection {
        active: address_of(selection.active),
        ranges: selection.ranges.into_iter().map(range_of).collect(),
    });
    Ok((
        PaintOptions {
            show_grid: js.show_grid,
            show_headers: js.show_headers,
            dark: js.dark,
            ..PaintOptions::default()
        },
        selection,
    ))
}

/// Собрать кадр листа в `frame` и вернуть число команд.
///
/// CANCEL-обвязка (ADR-0032) и маппинг ошибок формата остаются здесь — у
/// DOCX/PPTX свои типы ошибок, — а сериализация и транспорт общие у всех
/// форматов и живут в `doc_converter_wasm_common::frame`. Слот ring'а и буфер
/// JS отличаются только тем, куда байты деваются, — решение о транспорте не
/// должно разъезжаться с решением о содержимом кадра.
///
/// # Errors
/// Если книга не открыта или листа с таким номером нет.
fn build_frame(
    sheet: usize,
    viewport: Viewport,
    options: &PaintOptions,
    selection: Option<&doc_converter_xlsx::Selection>,
    frame: &mut DisplayList,
) -> Result<u32, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;

        let built = CANCEL.with(|cancelled| {
            paint::build_with_selection(book, sheet, viewport, options, selection, frame, cancelled)
        });
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
/// слота: `bytes` в статистике скажет, насколько, а по нему воркер решит,
/// растить ли ring.
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
    let (options, selection) = options_of(options)?;
    let stats = frame::build_sab(sab, slot_capacity, &FRAME, &SCRATCH, |frame| {
        build_frame(sheet, viewport, &options, selection.as_ref(), frame)
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
pub fn xlsx_build_display_list_into(
    out: &js_sys::Uint8Array,
    sheet: usize,
    viewport: JsValue,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let (options, selection) = options_of(options)?;
    let stats = frame::build_into(out, &FRAME, &SCRATCH, |frame| {
        build_frame(sheet, viewport, &options, selection.as_ref(), frame)
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

/// Зона листа под точкой: ячейка, заголовок или общий угол.
///
/// # Errors
/// Если книга не открыта, листа нет или окно описано неверно.
#[wasm_bindgen]
pub fn xlsx_sheet_hit_test(
    sheet: usize,
    viewport: JsValue,
    options: JsValue,
    x: f32,
    y: f32,
) -> Result<JsValue, JsValue> {
    let viewport = viewport_of(viewport)?;
    let options: RenderOptionsWire = serde_wasm_bindgen::from_value(options).map_err(to_js)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let hit = match paint::sheet_hit_test(sheet, viewport, options.show_headers, x, y) {
            paint::SheetHit::Cell(cell) => SheetHitWire::Cell {
                row: cell.row,
                col: cell.col,
            },
            paint::SheetHit::RowHeader { row } => SheetHitWire::RowHeader { row },
            paint::SheetHit::ColumnHeader { col } => SheetHitWire::ColumnHeader { col },
            paint::SheetHit::SelectAll => SheetHitWire::SelectAll,
            paint::SheetHit::Outside => SheetHitWire::Outside,
        };
        serde_wasm_bindgen::to_value(&hit).map_err(to_js)
    })
}

fn address_of(value: CellAddressWire) -> CellRef {
    CellRef::new(
        value.row.min(doc_converter_xlsx::cellref::MAX_ROW),
        value.col.min(doc_converter_xlsx::cellref::MAX_COL),
    )
}

fn address_wire(value: CellRef) -> CellAddressWire {
    CellAddressWire {
        row: value.row,
        col: value.col,
    }
}

fn range_of(value: CellRangeWire) -> doc_converter_xlsx::Range {
    doc_converter_xlsx::Range {
        first: address_of(value.start),
        last: address_of(value.end),
    }
    .normalized()
}

fn range_wire(value: doc_converter_xlsx::Range) -> CellRangeWire {
    let value = value.normalized();
    CellRangeWire {
        start: address_wire(value.first),
        end: address_wire(value.last),
    }
}

/// Нормализовать выделение по объединённым ячейкам текущей книги.
///
/// # Errors
/// Если книга не открыта, листа нет или снимок имеет неверную форму.
#[wasm_bindgen]
pub fn xlsx_normalize_selection(sheet: usize, selection: JsValue) -> Result<JsValue, JsValue> {
    let selection: SelectionWire = serde_wasm_bindgen::from_value(selection).map_err(to_js)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let normalized = doc_converter_xlsx::selection::normalize(
            sheet,
            &doc_converter_xlsx::Selection {
                active: address_of(selection.active),
                ranges: selection.ranges.into_iter().map(range_of).collect(),
            },
        );
        let result = SelectionWire {
            active: address_wire(normalized.active),
            ranges: normalized.ranges.into_iter().map(range_wire).collect(),
        };
        serde_wasm_bindgen::to_value(&result).map_err(to_js)
    })
}

/// Адрес назначения клавиатурной навигации по модели книги.
///
/// # Errors
/// Если книга не открыта, листа нет или команда неизвестна.
#[wasm_bindgen]
pub fn xlsx_navigate_selection(
    sheet: usize,
    active: JsValue,
    command: String,
) -> Result<JsValue, JsValue> {
    use doc_converter_xlsx::NavigationCommand as Command;
    let active: CellAddressWire = serde_wasm_bindgen::from_value(active).map_err(to_js)?;
    let command = if let Some(rest) = command.strip_prefix("steps-") {
        let Some((direction, count)) = rest.rsplit_once('-') else {
            return Err(to_js(format!("unknown navigation command `{command}`")));
        };
        let count = count
            .parse::<u32>()
            .map_err(|_| to_js(format!("unknown navigation command `{command}`")))?
            .clamp(1, 1_000);
        match direction {
            "left" => Command::StepLeft(count),
            "right" => Command::StepRight(count),
            "up" => Command::StepUp(count),
            "down" => Command::StepDown(count),
            _ => return Err(to_js(format!("unknown navigation command `{command}`"))),
        }
    } else {
        match command.as_str() {
            "left" => Command::Left,
            "right" => Command::Right,
            "up" => Command::Up,
            "down" => Command::Down,
            "ctrl-left" => Command::CtrlLeft,
            "ctrl-right" => Command::CtrlRight,
            "ctrl-up" => Command::CtrlUp,
            "ctrl-down" => Command::CtrlDown,
            "home" => Command::Home,
            "ctrl-home" => Command::CtrlHome,
            "ctrl-end" => Command::CtrlEnd,
            _ => return Err(to_js(format!("unknown navigation command `{command}`"))),
        }
    };
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let target = doc_converter_xlsx::selection::navigate(sheet, address_of(active), command);
        serde_wasm_bindgen::to_value(&address_wire(target)).map_err(to_js)
    })
}

/// Переход на один видимый экран по фактическим размерам строк и столбцов.
///
/// # Errors
/// Если книга не открыта, листа нет или окно описано неверно.
#[wasm_bindgen]
pub fn xlsx_page_selection(
    sheet: usize,
    active: JsValue,
    viewport: JsValue,
    options: JsValue,
    horizontal: bool,
    backward: bool,
) -> Result<JsValue, JsValue> {
    let active: CellAddressWire = serde_wasm_bindgen::from_value(active).map_err(to_js)?;
    let viewport = viewport_of(viewport)?;
    let options: RenderOptionsWire = serde_wasm_bindgen::from_value(options).map_err(to_js)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let target = doc_converter_xlsx::selection::page_navigate(
            sheet,
            address_of(active),
            viewport,
            options.show_headers,
            horizontal,
            backward,
        );
        serde_wasm_bindgen::to_value(&address_wire(target)).map_err(to_js)
    })
}

/// Область данных вокруг активной ячейки для Ctrl+A либо `null`.
///
/// # Errors
/// Если книга не открыта или листа нет.
#[wasm_bindgen]
pub fn xlsx_current_region(sheet: usize, active: JsValue) -> Result<JsValue, JsValue> {
    let active: CellAddressWire = serde_wasm_bindgen::from_value(active).map_err(to_js)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let region = doc_converter_xlsx::selection::current_region(sheet, address_of(active))
            .map(range_wire);
        serde_wasm_bindgen::to_value(&region).map_err(to_js)
    })
}

/// Границы ячейки или её объединения в пикселях полной раскладки.
///
/// # Errors
/// Если книга не открыта, листа нет или адрес имеет неверную форму.
#[wasm_bindgen]
pub fn xlsx_cell_rect(sheet: usize, active: JsValue) -> Result<JsValue, JsValue> {
    let active: CellAddressWire = serde_wasm_bindgen::from_value(active).map_err(to_js)?;
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let sheet = book
            .sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        let rect = doc_converter_xlsx::selection::cell_rect(sheet, address_of(active));
        let result = BBoxWire {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        };
        serde_wasm_bindgen::to_value(&result).map_err(to_js)
    })
}

/// Описание ссылки для интерфейса; `None` — цель не разрешилась
/// (`HyperlinkTarget::Broken`), переходить некуда.
fn hyperlink_info(link: &doc_converter_xlsx::Hyperlink) -> Option<HyperlinkInfoWire> {
    Some(HyperlinkInfoWire {
        target: link.target.address()?.to_owned(),
        display: link.display.clone(),
        tooltip: link.tooltip.clone(),
    })
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
            .and_then(hyperlink_info);
        // `None` уезжает в JS как `null`: отсутствие ссылки — не ошибка.
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true);
        link.serialize(&serializer).map_err(to_js)
    })
}

/// Заменить показ ячейки сырым текстом, как введён интерфейс.
///
/// Правка живёт до следующего открытия книги: `xlsx_open` и
/// `stream_open_xlsx` заводят новую книгу с пустой картой правок. Пустая
/// строка — тоже правка: она гасит текст ячейки. Кадры интерфейс перерисует
/// сам; кэши кадра сбрасываются здесь же.
///
/// # Errors
/// Если книга не открыта или листа с таким номером нет.
#[wasm_bindgen]
pub fn xlsx_set_cell_text(sheet: usize, row: u32, col: u32, text: String) -> Result<(), JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        book.sheets()
            .get(sheet)
            .ok_or_else(|| to_js(format!("sheet {sheet} does not exist")))?;
        book.set_cell_text(sheet, CellRef::new(row, col), text);
        // Правки редки: полный сброс дешевле, чем знать, какой из кэшей кадра
        // начнёт зависеть от текста значений.
        paint::reset_caches();
        Ok(())
    })
}

/// Снимок правок открытой книги — для экспортёра PDF, который разбирает байты
/// заново. Порядок детерминирован: лист, строка, столбец.
///
/// # Errors
/// Если книга не открыта.
#[wasm_bindgen]
pub fn xlsx_edits() -> Result<JsValue, JsValue> {
    DOC.with(|doc| {
        let doc = doc.borrow();
        let book = doc.as_ref().ok_or_else(|| to_js("no workbook is open"))?;
        let edits: Vec<CellEditWire> = book
            .cell_edits()
            .into_iter()
            .map(|(sheet, cell, text)| CellEditWire {
                sheet: u32::try_from(sheet).unwrap_or(u32::MAX),
                row: cell.row,
                col: cell.col,
                text,
            })
            .collect();
        serde_wasm_bindgen::to_value(&edits).map_err(to_js)
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

// Экспорт в PDF живёт в отдельном модуле (`crates/pdf-wasm`): экспортёр PDF
// тянет за собой lopdf/skrifa и раздувает бандл в разы, а нужен он только по
// клику «Экспорт в PDF». Воркер грузит его лениво.
