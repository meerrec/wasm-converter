//! DOCX: парсинг `word/document.xml`, `numbering.xml`, `styles.xml`,
//! постраничная раскладка, line-breaking.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

use doc_converter_core::{Archive, Result};

/// Открыть DOCX из сырых байт.
///
/// # Errors
/// Если байты не OOXML-пакет или в нём нет `word/document.xml`.
pub fn open(bytes: Vec<u8>) -> Result<Document> {
    let mut archive = Archive::new(bytes)?;
    archive.validate_ooxml()?;
    let _document_xml = archive.read_string("word/document.xml")?;
    // TODO (Фаза 6): разобрать document.xml + styles + numbering + rels.
    Ok(Document { _private: () })
}

pub struct Document {
    _private: (),
}

impl Document {
    #[must_use]
    pub fn page_count(&self) -> u32 {
        // TODO (Фаза 6): посчитать страницы после pagination.
        1
    }
}
