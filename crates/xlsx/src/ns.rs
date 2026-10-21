//! Namespace'ы OOXML: Transitional и Strict (ADR-0034).
//!
//! Парсеры частей сравнивают локальные имена элементов и атрибутов, а не
//! namespace: `<worksheet>` и его Strict-аналог дают одно и то же имя, поэтому
//! модель от варианта не зависит. Вариант нужен ровно в точке входа — по
//! namespace корня `xl/workbook.xml` выбирается таблица [`NsTable`], а пакет с
//! незнакомым namespace отвергается.
//!
//! Вариант определяется по корню главной части, а не по `[Content_Types].xml`:
//! content types у Strict и Transitional совпадают, и признака варианта там нет.

use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesStart, Event};

use crate::error::{Result, XlsxError};

/// Таблица namespace'ов одного варианта OOXML (ADR-0017, ADR-0034).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NsTable {
    /// Основной namespace `SpreadsheetML`.
    pub spreadsheetml: &'static str,
    /// Основной namespace `DrawingML` (`a:`).
    pub drawingml: &'static str,
    /// Namespace типов связей (`r:`).
    pub relationships: &'static str,
}

/// Transitional OOXML — вариант, который Excel пишет по умолчанию.
pub const TRANSITIONAL: NsTable = NsTable {
    spreadsheetml: "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    drawingml: "http://schemas.openxmlformats.org/drawingml/2006/main",
    relationships: "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
};

/// Strict OOXML (ISO/IEC 29500-1 Strict).
pub const STRICT: NsTable = NsTable {
    spreadsheetml: "http://purl.oclc.org/ooxml/spreadsheetml/main",
    drawingml: "http://purl.oclc.org/ooxml/drawingml/main",
    relationships: "http://purl.oclc.org/ooxml/officeDocument/relationships",
};

/// Вариант OOXML, определённый по корню главной части.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OoxmlVariant {
    /// Transitional (ECMA-376) — вариант по умолчанию у большинства писателей.
    Transitional,
    /// Strict (ISO/IEC 29500-1 Strict).
    Strict,
}

impl OoxmlVariant {
    /// Таблица namespace'ов варианта.
    #[must_use]
    pub const fn table(self) -> &'static NsTable {
        match self {
            Self::Transitional => &TRANSITIONAL,
            Self::Strict => &STRICT,
        }
    }
}

/// Определить вариант OOXML по namespace корневого элемента части.
///
/// Корень ищется в уже прочитанных байтах части: второй раз `xl/workbook.xml`
/// не читается. Объявление `xmlns` разрешается по префиксу самого корня
/// (беспрефиксный элемент → `xmlns`, иначе `xmlns:<префикс>`).
///
/// # Errors
/// [`XlsxError::UnknownVariant`] — namespace корня не Transitional и не Strict;
/// [`XlsxError::Core`] — XML части не разбирается; [`XlsxError::Malformed`] —
/// корня нет или у него не объявлен namespace.
#[must_use = "вариант OOXML — результат проверки пакета"]
pub fn detect(xml: &[u8], part: &str) -> Result<OoxmlVariant> {
    let mut reader = XmlReader::preserving(xml, part);
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(XlsxError::malformed(part, "no root element"));
        };
        let (Event::Start(root) | Event::Empty(root)) = event else {
            continue;
        };
        return variant_of_root(&root, part);
    }
}

/// Вариант по корневому элементу: разрешить префикс и сравнить namespace.
fn variant_of_root(root: &BytesStart<'_>, part: &str) -> Result<OoxmlVariant> {
    let name = root.name();
    let name = name.as_ref();
    let (prefix, named) = match name.iter().position(|byte| *byte == b':') {
        Some(colon) => (&name[..colon], true),
        None => (name, false),
    };

    for attribute in root.attributes() {
        let attribute = attribute
            .map_err(|e| XlsxError::malformed(part, format!("malformed root attributes: {e}")))?;
        let key = attribute.key.as_ref();
        if key == b"xmlns".as_slice() {
            if !named {
                return classify(attribute.value.as_ref(), part);
            }
        } else if let Some(declared) = key.strip_prefix(b"xmlns:".as_slice()) {
            if named && declared == prefix {
                return classify(attribute.value.as_ref(), part);
            }
        }
    }

    Err(XlsxError::malformed(
        part,
        "the root element has no namespace declaration",
    ))
}

/// Сопоставить namespace корня с вариантом.
fn classify(namespace: &[u8], part: &str) -> Result<OoxmlVariant> {
    if namespace == TRANSITIONAL.spreadsheetml.as_bytes() {
        Ok(OoxmlVariant::Transitional)
    } else if namespace == STRICT.spreadsheetml.as_bytes() {
        Ok(OoxmlVariant::Strict)
    } else {
        Err(XlsxError::UnknownVariant {
            part: part.to_owned(),
            namespace: String::from_utf8_lossy(namespace).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSITIONAL_ROOT: &[u8] =
        br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#;
    const STRICT_ROOT: &[u8] =
        br#"<workbook xmlns="http://purl.oclc.org/ooxml/spreadsheetml/main"/>"#;

    #[test]
    fn transitional_root_is_detected() {
        assert_eq!(
            detect(TRANSITIONAL_ROOT, "xl/workbook.xml").unwrap(),
            OoxmlVariant::Transitional
        );
    }

    #[test]
    fn strict_root_is_detected() {
        assert_eq!(
            detect(STRICT_ROOT, "xl/workbook.xml").unwrap(),
            OoxmlVariant::Strict
        );
    }

    #[test]
    fn prefixed_root_is_detected() {
        let xml = br#"<x:workbook xmlns:x="http://purl.oclc.org/ooxml/spreadsheetml/main"/>"#;
        assert_eq!(
            detect(xml, "xl/workbook.xml").unwrap(),
            OoxmlVariant::Strict
        );
    }

    #[test]
    fn declaration_before_the_root_is_skipped() {
        let xml = br#"<?xml version="1.0"?><!-- comment --><workbook xmlns="http://purl.oclc.org/ooxml/spreadsheetml/main"/>"#;
        assert_eq!(
            detect(xml, "xl/workbook.xml").unwrap(),
            OoxmlVariant::Strict
        );
    }

    #[test]
    fn unknown_namespace_is_rejected() {
        let xml = br#"<workbook xmlns="http://example.com/spreadsheetml"/>"#;
        let err = detect(xml, "xl/workbook.xml").expect_err("незнакомый namespace");
        assert!(
            matches!(&err, XlsxError::UnknownVariant { namespace, .. } if namespace == "http://example.com/spreadsheetml"),
            "{err}"
        );
    }

    #[test]
    fn missing_declaration_is_rejected() {
        let err = detect(b"<workbook/>", "xl/workbook.xml").expect_err("нет xmlns");
        assert!(matches!(err, XlsxError::Malformed { .. }), "{err}");
    }

    #[test]
    fn variant_tables_carry_the_expected_namespaces() {
        assert_eq!(
            OoxmlVariant::Transitional.table().spreadsheetml,
            TRANSITIONAL.spreadsheetml
        );
        assert_eq!(
            OoxmlVariant::Strict.table().relationships,
            "http://purl.oclc.org/ooxml/officeDocument/relationships"
        );
    }
}
