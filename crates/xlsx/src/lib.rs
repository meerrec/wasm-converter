//! XLSX: разбор `workbook.xml` + `sheet*.xml` + `sharedStrings` + styles,
//! построение `DisplayList` для видимой области.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod cellref;
pub mod error;
pub mod model;
pub mod strings;
pub mod styles;
pub mod workbook;
pub mod worksheet;

/// Помощники чтения XML — деталь реализации, наружу не выходят.
mod xml;

pub use error::{Result, XlsxError};
pub use model::{
    Border, BorderSide, BorderStyle, Cell, CellError, CellFormat, CellValue, Color, Fill,
    FillPattern, Font, Sheet, SheetState, StyleTable, Workbook, Worksheet, WorksheetBuilder,
    WorksheetMeta,
};
pub use strings::SharedStrings;
pub use workbook::WorkbookMeta;

use doc_converter_core::rels::{rels_part, RelMap};
use doc_converter_core::Archive;

/// Часть с каталогом листов.
pub const WORKBOOK_PART: &str = "xl/workbook.xml";

/// Общая таблица строк. В книге без строк этой части нет вовсе, и это норма.
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// Таблица стилей.
pub const STYLES_PART: &str = "xl/styles.xml";

/// Открыть XLSX из сырых байт.
///
/// Читает каталог листов, общую таблицу строк, стили и содержимое всех листов.
///
/// Листы разбираются сразу: рендер и экспорт работают по модели, а ленивый
/// разбор потребовал бы держать открытый архив внутри книги.
///
/// # Errors
/// Если байты не OOXML-пакет, в нём нет `xl/workbook.xml` или какая-то из
/// частей не разбирается.
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes)?;
    archive.validate_ooxml()?;

    let rels = RelMap::parse(&archive.read(&rels_part(WORKBOOK_PART))?)?;
    let catalog = WorkbookMeta::parse(&archive.read(WORKBOOK_PART)?, &rels, WORKBOOK_PART)?;

    let shared_strings = if archive.contains(SHARED_STRINGS_PART) {
        SharedStrings::parse(&archive.read(SHARED_STRINGS_PART)?, SHARED_STRINGS_PART)?
    } else {
        SharedStrings::default()
    };

    let style_table = if archive.contains(STYLES_PART) {
        styles::parse(&archive.read(STYLES_PART)?, STYLES_PART)?
    } else {
        StyleTable::default()
    };

    let mut sheets = Vec::with_capacity(catalog.sheets.len());
    for meta in catalog.sheets {
        let data = worksheet::parse(&archive.read(&meta.part)?, &meta.part)?;
        sheets.push(Sheet { meta, data });
    }

    Ok(Workbook::new(
        sheets,
        shared_strings,
        style_table,
        catalog.date1904,
    ))
}
