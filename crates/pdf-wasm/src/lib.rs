//! Отдельный wasm-модуль экспорта в PDF: лист XLSX и документ DOCX.
//!
//! Тянет `doc-converter-pdf` (экспортёр PDF), поэтому в основной модуль не
//! входит: просмотрщику он нужен только по клику «Экспорт в PDF», и воркер
//! подгружает его по требованию. Модуль самодостаточен — никакого
//! `thread_local`: книгу и документ он получает байтами и разбирает сам.
//!
//! Как и `crates/wasm`, крейт собирается только под `wasm32`: на нативном
//! таргете он пуст, и `cargo clippy --workspace --all-targets` проходит без
//! выбора цели.

#![cfg(target_arch = "wasm32")]

use doc_converter_core::wire::{
    CellEditWire, DocxRunEditWire, OrientationWire, PageSizeWire, PdfOptionsWire,
};
use doc_converter_pdf::{Margins, PageOrientation, PageSize, PdfExporter, PdfOptions};
use doc_converter_xlsx::CellRef;
use wasm_bindgen::prelude::*;

/// Дополнить умолчания PDF-крейта тем, что выбрал интерфейс.
fn into_options(options: PdfOptionsWire) -> PdfOptions {
    let mut result = PdfOptions::default();
    if let Some(size) = options.page_size {
        result.page.size = match size {
            PageSizeWire::A4 => PageSize::A4,
            PageSizeWire::A3 => PageSize::A3,
            PageSizeWire::Letter => PageSize::Letter,
            PageSizeWire::Legal => PageSize::Legal,
        };
    }
    if let Some(orientation) = options.orientation {
        result.page.orientation = match orientation {
            OrientationWire::Portrait => PageOrientation::Portrait,
            OrientationWire::Landscape => PageOrientation::Landscape,
        };
    }
    if let Some(margins) = options.margins {
        result.page.margins = Some(Margins {
            top_mm: margins.top,
            right_mm: margins.right,
            bottom_mm: margins.bottom,
            left_mm: margins.left,
        });
    }
    if let Some(scale) = options.scale {
        result.page.scale = scale;
    }
    // Явные числа сильнее флага: `fitToWidth: 0` — «без ограничения по
    // ширине», и булев `fitToPage` не должен вернуть единицу обратно.
    if let Some(width) = options.fit_to_width {
        result.page.fit_to_width = fit_pages(width);
    }
    if let Some(height) = options.fit_to_height {
        result.page.fit_to_height = fit_pages(height);
    }
    if options.fit_to_page == Some(true) {
        if options.fit_to_width.is_none() {
            result.page.fit_to_width = Some(1);
        }
        if options.fit_to_height.is_none() {
            result.page.fit_to_height = Some(1);
        }
    }
    result.sheet_index = options.sheet_index;
    result
}

/// Число страниц `fitTo*`: `0` — «сколько потребуется», то есть без границы.
fn fit_pages(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Настройки из JS или умолчания, если их не передали.
fn parse_options(options: JsValue) -> Result<PdfOptions, JsValue> {
    if options.is_undefined() || options.is_null() {
        return Ok(PdfOptions::default());
    }
    serde_wasm_bindgen::from_value::<PdfOptionsWire>(options)
        .map(into_options)
        .map_err(to_js)
}

/// Экспортировать лист книги в PDF. Книга приходит байтами XLSX и
/// разбирается здесь же: у модуля нет общего с просмотрщиком состояния.
///
/// `sheet` — индекс листа в книге; `options.sheetIndex` перекрывает его,
/// если задан.
///
/// # Errors
/// Если байты не OOXML-пакет, листа с таким индексом нет или PDF не собрался.
#[wasm_bindgen]
pub fn export_pdf(bytes: &[u8], sheet: usize, options: JsValue) -> Result<Vec<u8>, JsValue> {
    export_pdf_with(bytes, sheet, options, JsValue::UNDEFINED)
}

/// [`export_pdf`] с правками интерфейса: `edits` — снимок `xlsx_edits` из
/// основного модуля просмотрщика.
///
/// # Errors
/// Те же, что у [`export_pdf`], плюс неразборчивый снимок правок.
#[wasm_bindgen]
pub fn export_pdf_edits(
    bytes: &[u8],
    sheet: usize,
    options: JsValue,
    edits: JsValue,
) -> Result<Vec<u8>, JsValue> {
    export_pdf_with(bytes, sheet, options, edits)
}

/// Общий хвост экспорта листа: правки прикладываются к разобранной книге до
/// экспортёра.
fn export_pdf_with(
    bytes: &[u8],
    sheet: usize,
    options: JsValue,
    edits: JsValue,
) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let sheet = options.sheet_index.unwrap_or(sheet);
    let book = doc_converter_xlsx::open(bytes.to_vec()).map_err(to_js)?;
    apply_edits(&book, edits)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_xlsx_sheet(&book, sheet).map_err(to_js)
}

/// Экспортировать книгу целиком в один PDF: листы идут подряд, закладка —
/// на первый лист каждого.
///
/// Отличие от [`export_pdf`] только в том, какие листы попадают в документ:
/// здесь — все, в порядке книги. `options.sheetIndex` в этом пути не читается:
/// он выбирает один лист, а книга — это все сразу.
///
/// # Errors
/// Если байты не OOXML-пакет, в книге нет листов или PDF не собрался.
#[wasm_bindgen]
pub fn export_pdf_book(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    export_pdf_book_with(bytes, options, JsValue::UNDEFINED)
}

/// [`export_pdf_book`] с правками интерфейса: `edits` — снимок `xlsx_edits`.
///
/// # Errors
/// Те же, что у [`export_pdf_book`], плюс неразборчивый снимок правок.
#[wasm_bindgen]
pub fn export_pdf_book_edits(
    bytes: &[u8],
    options: JsValue,
    edits: JsValue,
) -> Result<Vec<u8>, JsValue> {
    export_pdf_book_with(bytes, options, edits)
}

/// Общий хвост экспорта книги: правки прикладываются до экспортёра.
fn export_pdf_book_with(
    bytes: &[u8],
    options: JsValue,
    edits: JsValue,
) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let book = doc_converter_xlsx::open(bytes.to_vec()).map_err(to_js)?;
    apply_edits(&book, edits)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_xlsx_book(&book).map_err(to_js)
}

/// Экспортировать книгу XLSB в PDF: листы идут подряд, закладка — на первый
/// лист каждого. Книга приходит байтами XLSB и разбирается здесь же: у модуля
/// нет общего с просмотрщиком состояния, а модель — та же, что у XLSX
/// ([`doc_converter_xlsb::open`] доводит бинарные части до книги XLSX).
///
/// # Errors
/// Если байты не XLSB-пакет (не ZIP, нет `xl/workbook.bin`), в книге лист
/// неподдержанного типа или PDF не собрался.
#[wasm_bindgen]
pub fn export_xlsb_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let book = doc_converter_xlsb::open(bytes.to_vec()).map_err(to_js)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_xlsx_book(&book).map_err(to_js)
}

/// Экспортировать книгу ODS в PDF: листы идут подряд, закладка — на первый
/// лист каждого. Книга приходит байтами ODS и разбирается здесь же: модель —
/// та же, что у XLSX ([`doc_converter_odf::ods::open`] доводит `content.xml` до
/// книги XLSX).
///
/// # Errors
/// Если байты не ODS-пакет (`mimetype` не электронная таблица, нет
/// `content.xml`) или PDF не собрался.
#[wasm_bindgen]
pub fn export_ods_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let book = doc_converter_odf::ods::open(bytes.to_vec()).map_err(to_js)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_xlsx_book(&book).map_err(to_js)
}

/// Приложить снимок правок интерфейса к разобранной книге.
///
/// Пустой снимок (`null`/`undefined`) — обычный экспорт без правок: путь
/// остаётся байт-в-байт прежним.
fn apply_edits(book: &doc_converter_xlsx::Workbook, edits: JsValue) -> Result<(), JsValue> {
    if edits.is_undefined() || edits.is_null() {
        return Ok(());
    }
    let edits: Vec<CellEditWire> = serde_wasm_bindgen::from_value(edits).map_err(to_js)?;
    for edit in edits {
        book.set_cell_text(
            usize::try_from(edit.sheet).unwrap_or(usize::MAX),
            CellRef::new(edit.row, edit.col),
            edit.text,
        );
    }
    Ok(())
}

/// Экспортировать документ DOCX в PDF. Документ приходит байтами OOXML и
/// разбирается здесь же: у модуля нет общего с просмотрщиком состояния.
///
/// Опции — тот же [`PdfOptionsWire`], что у XLSX, но читаются не все: страницы
/// PDF — это страницы раскладки DOCX, поэтому размер, поля и ориентация
/// берутся из `w:sectPr`, а `pageSize`, `orientation`, `margins`, `scale` и
/// `sheetIndex` не читаются. Действуют метаданные (`title`, `author`,
/// `subject`, `keywords`) и `compress`.
///
/// # Errors
/// Если байты не OOXML-пакет, `word/document.xml` не разбирается, раскладка
/// документа не собралась или PDF не собрался.
#[wasm_bindgen]
pub fn export_docx_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let document = doc_converter_docx::open(bytes.to_vec()).map_err(to_js)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_docx(&document).map_err(to_js)
}

/// Экспортировать документ ODT в PDF. Документ приходит байтами ODF и
/// разбирается здесь же: модель — та же, что у DOCX
/// ([`doc_converter_odf::odt::open`] доводит `content.xml` до документа DOCX),
/// поэтому раскладку и страницы даёт DOCX-семейство (ADR-0046).
///
/// Опции — тот же [`PdfOptionsWire`], что у XLSX/DOCX, но читаются не все:
/// размер, поля и ориентация берутся из раскладки документа, а `pageSize`,
/// `orientation`, `margins`, `scale` и `sheetIndex` не читаются.
///
/// # Errors
/// Если байты не ODT-пакет (`mimetype` не текстовый документ, нет
/// `content.xml`), раскладка документа не собралась или PDF не собрался.
#[wasm_bindgen]
pub fn export_odt_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let document = doc_converter_odf::odt::open(bytes.to_vec()).map_err(to_js)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_docx(&document).map_err(to_js)
}

/// [`export_docx_pdf`] с правками интерфейса: `edits` — снимок `docx_edits` из
/// просмотрщика. Правки прикладываются к заново разобранному документу до
/// раскладки, поэтому перераскладка PDF видит тот же текст, что и кадр.
///
/// # Errors
/// Как у [`export_docx_pdf`], плюс неверный адрес run'а в снимке.
#[wasm_bindgen]
pub fn export_docx_pdf_edits(
    bytes: &[u8],
    options: JsValue,
    edits: JsValue,
) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let document = doc_converter_docx::open(bytes.to_vec()).map_err(to_js)?;
    apply_docx_edits(&document, edits)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_docx(&document).map_err(to_js)
}

/// Приложить снимок правок интерфейса к заново разобранному документу.
fn apply_docx_edits(
    document: &doc_converter_docx::Document,
    edits: JsValue,
) -> Result<(), JsValue> {
    if edits.is_undefined() || edits.is_null() {
        return Ok(());
    }
    let edits: Vec<DocxRunEditWire> = serde_wasm_bindgen::from_value(edits).map_err(to_js)?;
    for edit in edits {
        document
            .set_run_text(edit.paragraph, edit.run, edit.text)
            .map_err(to_js)?;
    }
    Ok(())
}
