//! XLSX: разбор `workbook.xml` + `sheet*.xml` + `sharedStrings` + styles,
//! построение `DisplayList` для видимой области.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod cellref;

use doc_converter_core::{Archive, Result};

/// Открыть XLSX из сырых байт.
///
/// # Errors
/// Если байты не OOXML-пакет или в нём нет `xl/workbook.xml`.
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes)?;
    archive.validate_ooxml()?;
    let wb_xml = archive.read_string("xl/workbook.xml")?;
    // TODO (Фаза 3): потоковый разбор workbook.xml, каталог листов.
    Ok(Workbook {
        _private: (),
        _wb_xml: wb_xml,
    })
}

pub struct Workbook {
    _private: (),
    _wb_xml: String,
}

impl Workbook {
    #[must_use]
    pub fn sheet_names(&self) -> Vec<String> {
        // TODO (Фаза 3): извлечь <sheet name="…"/> из workbook.xml.
        vec!["Sheet1".to_string()]
    }
}
