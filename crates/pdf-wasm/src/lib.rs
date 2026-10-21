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

use doc_converter_core::wire::{OrientationWire, PageSizeWire, PdfOptionsWire};
use doc_converter_pdf::{Margins, PageOrientation, PageSize, PdfExporter, PdfOptions};
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
    result.sheet_index = options.sheet_index;
    result
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
    let options = parse_options(options)?;
    let sheet = options.sheet_index.unwrap_or(sheet);
    let book = doc_converter_xlsx::open(bytes.to_vec()).map_err(to_js)?;
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
    let options = parse_options(options)?;
    let book = doc_converter_xlsx::open(bytes.to_vec()).map_err(to_js)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_xlsx_book(&book).map_err(to_js)
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

/// Экспортировать презентацию PPTX в PDF: слайд = страница, размер страницы
/// из `p:sldSz`. Презентация приходит байтами OOXML и разбирается здесь же: у
/// модуля нет общего с просмотрщиком состояния.
///
/// Опции принимаются для симметрии протокола с XLSX/DOCX, но не читаются:
/// размер, поля, ориентация и масштаб страницы задаёт сама презентация
/// (`p:sldSz`), а метаданных в модели PPTX нет. Действует `compress`.
///
/// # Errors
/// Если байты не OOXML-пакет, `ppt/presentation.xml` не разбирается, в
/// презентации нет слайдов или PDF не собрался.
#[wasm_bindgen]
pub fn export_pptx_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let presentation = doc_converter_pptx::open(bytes.to_vec()).map_err(to_js)?;
    doc_converter_pptx_pdf::export(&presentation, &options).map_err(to_js)
}
