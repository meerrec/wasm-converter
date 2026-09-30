//! XLSX: разбор `workbook.xml` + `sheet*.xml` + `sharedStrings` + styles,
//! построение `DisplayList` для видимой области.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod cellref;
pub mod error;
pub mod model;
pub mod strings;
pub mod workbook;

pub use error::{Result, XlsxError};
pub use model::{
    Cell, CellError, CellFormat, CellValue, SheetState, StyleTable, Workbook, Worksheet,
    WorksheetBuilder, WorksheetMeta,
};
pub use strings::SharedStrings;
pub use workbook::WorkbookMeta;

use doc_converter_core::rels::{rels_part, RelMap};
use doc_converter_core::Archive;

/// Часть с каталогом листов.
pub const WORKBOOK_PART: &str = "xl/workbook.xml";

/// Общая таблица строк. В книге без строк этой части нет вовсе, и это норма.
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// Открыть XLSX из сырых байт.
///
/// Читает каталог листов и общую таблицу строк. Содержимое листов и стили —
/// `TODO (Фаза 3)`, до них книга отдаёт пустые `StyleTable` и листы без ячеек.
///
/// # Errors
/// Если байты не OOXML-пакет, в нём нет `xl/workbook.xml` или `workbook.xml`
/// не разбирается.
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes)?;
    archive.validate_ooxml()?;

    let rels = RelMap::parse(&archive.read(&rels_part(WORKBOOK_PART))?)?;
    let meta = WorkbookMeta::parse(&archive.read(WORKBOOK_PART)?, &rels, WORKBOOK_PART)?;

    let shared_strings = if archive.contains(SHARED_STRINGS_PART) {
        SharedStrings::parse(&archive.read(SHARED_STRINGS_PART)?, SHARED_STRINGS_PART)?
    } else {
        SharedStrings::default()
    };

    // TODO (Фаза 3): `xl/styles.xml` → `StyleTable` и содержимое листов через
    // `WorksheetBuilder` (`worksheet.rs`).
    Ok(Workbook::new(
        meta.sheets,
        shared_strings,
        StyleTable::default(),
        meta.date1904,
    ))
}
