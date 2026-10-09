//! Минимальный OPC-пакет в памяти для docx-целей.
//!
//! Разбор отдельной части наружу не отдаётся: публичен только разбор пакета
//! целиком. Поэтому цель собирает валидную оболочку DOCX и подменяет в ней
//! одну часть байтами фаззера — до этой части разбор доходит, а на остальных
//! не спотыкается раньше времени.
//!
//! Записи кладутся методом `Stored`: сжатие — это работа на каждой итерации,
//! а частей здесь меньше килобайта.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// `[Content_Types].xml`: без него разбор пакет не открывает.
pub const CONTENT_TYPES_XML: &[u8] = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;

/// `_rels/.rels`: связь `officeDocument` — из неё берётся имя главной части.
pub const ROOT_RELS_XML: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

/// Связи главной части: стили и нумерация — те части, которые цель подменяет.
pub const DOCUMENT_RELS_XML: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#;

/// Главная часть с одним абзацем.
pub const DOCUMENT_XML: &[u8] = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello, World!</w:t></w:r></w:p></w:body></w:document>"#;

/// Пустая таблица стилей: корень `w:styles` обязателен, содержимое — нет.
pub const STYLES_XML: &[u8] =
    br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#;

/// Пустая таблица нумерации: документ без нумерации — обычное дело.
pub const NUMBERING_XML: &[u8] =
    br#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#;

/// Пакет с полным набором частей, где `part` заменена байтами фаззера.
///
/// Имя части, которой нет в шаблоне, ничего не подменяет: вызывающие берут
/// имена из констант выше.
#[must_use]
pub fn package_with(part: &str, data: &[u8]) -> Vec<u8> {
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", CONTENT_TYPES_XML),
        ("_rels/.rels", ROOT_RELS_XML),
        ("word/document.xml", DOCUMENT_XML),
        ("word/styles.xml", STYLES_XML),
        ("word/numbering.xml", NUMBERING_XML),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS_XML),
    ];
    for slot in &mut parts {
        if slot.0 == part {
            slot.1 = data;
        }
    }
    write_package(&parts)
}

/// Записать ZIP с несжатыми частями.
///
/// Паника здесь — не находка: и имена, и структуру архива строит сама цель,
/// фаззерных байтов в них нет.
#[must_use]
fn write_package(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        writer
            .start_file(*name, options)
            .expect("starting a stored entry in a Vec cannot fail");
        writer
            .write_all(data)
            .expect("writing into a Vec cannot fail");
    }
    writer
        .finish()
        .expect("finishing a ZIP over a Vec cannot fail")
        .into_inner()
}
