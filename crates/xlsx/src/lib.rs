//! XLSX: разбор `workbook.xml` + `sheet*.xml` + `sharedStrings` + styles,
//! построение `DisplayList` для видимой области.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod cellref;
pub mod error;
pub mod model;
pub mod strings;

pub use error::{Result, XlsxError};
pub use model::{
    Cell, CellError, CellFormat, CellValue, SheetState, StyleTable, Workbook, Worksheet,
    WorksheetBuilder, WorksheetMeta,
};
pub use strings::SharedStrings;

use doc_converter_core::Archive;

/// Открыть XLSX из сырых байт.
///
/// # Errors
/// Если байты не OOXML-пакет или в нём нет `xl/workbook.xml`.
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes)?;
    archive.validate_ooxml()?;
    archive.read_string("xl/workbook.xml")?;
    // TODO (Фаза 3): потоковый разбор `workbook.xml` → `WorksheetMeta`,
    // разрешение частей листов через `_rels/workbook.xml.rels` и наполнение
    // листов через `WorksheetBuilder`.
    Ok(Workbook::default())
}
