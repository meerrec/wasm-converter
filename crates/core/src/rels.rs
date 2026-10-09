//! `_rels/*.rels` — карта relationships.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use quick_xml::XmlVersion;

use crate::error::{Error, Result};
use crate::xml::XmlReader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub target_mode: Option<String>,
}

#[derive(Debug, Default)]
pub struct RelMap {
    pub items: HashMap<String, Relationship>,
}

impl RelMap {
    /// Разбирает `_rels/*.rels`.
    ///
    /// # Errors
    /// Если XML некорректен или `<Relationship/>` неполный.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = XmlReader::new(bytes, "_rels");
        let mut items = HashMap::new();
        while let Some(ev) = r.next_significant()? {
            if let quick_xml::events::Event::Empty(e) | quick_xml::events::Event::Start(e) = ev {
                if e.name().as_ref() == b"Relationship" {
                    let mut id = None;
                    let mut rel_type = None;
                    let mut target = None;
                    let mut target_mode = None;
                    for attr in e.attributes() {
                        let attr =
                            attr.map_err(|e| Error::Malformed(format!("bad attribute: {e}")))?;
                        let key = attr.key.as_ref();
                        // `normalized_value` разворачивает XML-сущности: цель
                        // ссылки с `&amp;` в запросе иначе осталась бы с
                        // амперсандом в эскейпленной форме.
                        let val = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|e| Error::Malformed(format!("bad attribute value: {e}")))?
                            .into_owned();
                        match key {
                            b"Id" => id = Some(val),
                            b"Type" => rel_type = Some(val),
                            b"Target" => target = Some(val),
                            b"TargetMode" => target_mode = Some(val),
                            _ => {}
                        }
                    }
                    match (id, rel_type, target) {
                        (Some(id), Some(rel_type), Some(target)) => {
                            items.insert(
                                id.clone(),
                                Relationship {
                                    id,
                                    rel_type,
                                    target,
                                    target_mode,
                                },
                            );
                        }
                        _ => return Err(Error::Malformed("incomplete <Relationship/>".into())),
                    }
                }
            }
        }
        Ok(Self { items })
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.get(id)
    }
}

impl Relationship {
    /// Путь части внутри пакета, на которую указывает связь.
    ///
    /// `Target` записан относительно каталога части-источника
    /// (`worksheets/sheet1.xml` для `xl/workbook.xml`), но встречаются и
    /// абсолютные цели (`/xl/worksheets/sheet1.xml`), и выходы наверх (`../`).
    /// `None` — цель внешняя: это ссылка, а не часть пакета.
    ///
    /// `Target` приходит percent-закодированным (`media/image%201.png`) —
    /// в ZIP часть записана уже раскодированной (`media/image 1.png`),
    /// поэтому декодирование идёт последним шагом.
    #[must_use]
    pub fn part(&self, source_part: &str) -> Option<String> {
        if self.target_mode.as_deref() == Some("External") {
            return None;
        }
        Some(percent_decode(&resolve_target(source_part, &self.target)))
    }
}

/// Percent-декодирование пути части.
///
/// `+` остаётся собой: это путь, а не form-encoding. Битая последовательность
/// (`%`, `%2`, `%ZZ`) сохраняется дословно — пакет должен открыться, даже если
/// имена частей записаны с ошибками. Если байты не складываются в UTF-8,
/// возвращается исходная строка: терять данные нельзя.
fn percent_decode(target: &str) -> String {
    let bytes = target.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| target.to_string())
}

/// Значение hex-цифры; регистр не важен.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Имя части с relationships для части-источника:
/// `xl/workbook.xml` → `xl/_rels/workbook.xml.rels`.
#[must_use]
pub fn rels_part(source_part: &str) -> String {
    match source_part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{source_part}.rels"),
    }
}

/// Привести `Target` к пути части внутри пакета.
fn resolve_target(source_part: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }

    let base = source_part.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut segments: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            name => segments.push(name),
        }
    }
    segments.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_entities_in_target() {
        // Амперсанд в запросе URL: без разворачивания сущности цель уехала бы
        // в браузер в эскейпленной форме.
        let xml = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1" Type="http://x/hyperlink"
                              Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/>
            </Relationships>"#;
        let m = RelMap::parse(xml).unwrap();

        assert_eq!(
            m.get("rId1").unwrap().target,
            "https://example.com/?a=1&b=2"
        );
    }

    #[test]
    fn parses_simple_rels() {
        let xml = br#"<?xml version="1.0"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1" Type="http://x/officeDocument" Target="word/document.xml"/>
            </Relationships>"#;
        let m = RelMap::parse(xml).unwrap();
        assert_eq!(m.get("rId1").unwrap().target, "word/document.xml");
    }

    fn rel(target: &str, mode: Option<&str>) -> Relationship {
        Relationship {
            id: "rId1".into(),
            rel_type: "http://x/worksheet".into(),
            target: target.into(),
            target_mode: mode.map(Into::into),
        }
    }

    #[test]
    fn resolves_target_against_the_source_part() {
        assert_eq!(
            rel("worksheets/sheet1.xml", None).part("xl/workbook.xml"),
            Some("xl/worksheets/sheet1.xml".into())
        );
    }

    #[test]
    fn resolves_absolute_and_parent_targets() {
        assert_eq!(
            rel("/xl/worksheets/sheet2.xml", None).part("xl/workbook.xml"),
            Some("xl/worksheets/sheet2.xml".into())
        );
        assert_eq!(
            rel("../media/image1.png", None).part("xl/drawings/drawing1.xml"),
            Some("xl/media/image1.png".into())
        );
        assert_eq!(
            rel("./sheet1.xml", None).part("xl/workbook.xml"),
            Some("xl/sheet1.xml".into())
        );
    }

    #[test]
    fn external_targets_are_not_parts() {
        assert_eq!(
            rel("https://example.com/", Some("External")).part("xl/workbook.xml"),
            None
        );
    }

    #[test]
    fn percent_decodes_media_names() {
        assert_eq!(
            rel("media/image%201.png", None).part("word/document.xml"),
            Some("word/media/image 1.png".into())
        );
    }

    #[test]
    fn percent_decodes_utf8_in_deep_paths() {
        // %D0%BF… — «привет» в UTF-8; байты собираются в строку после раскрытия
        // `../`, иначе проверялся бы путь до нормализации.
        assert_eq!(
            rel("../media/%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82.xml", None)
                .part("word/document.xml"),
            Some("media/привет.xml".into())
        );
        assert_eq!(
            rel("/word/media/%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82.xml", None)
                .part("word/document.xml"),
            Some("word/media/привет.xml".into())
        );
    }

    #[test]
    fn percent_decode_ignores_hex_case() {
        assert_eq!(
            rel("media%2fimage%2F1.png", None).part("word/document.xml"),
            Some("word/media/image/1.png".into())
        );
    }

    #[test]
    fn keeps_broken_percent_sequences() {
        for broken in ["media/%", "media/%2", "media/%ZZ", "media/100%"] {
            assert_eq!(
                rel(broken, None).part("word/document.xml"),
                Some(format!("word/{broken}")),
                "битая последовательность {broken} должна остаться как есть"
            );
        }
    }

    #[test]
    fn keeps_the_path_when_decoded_bytes_are_not_utf8() {
        // %FF — не UTF-8-последовательность; строка остаётся как есть,
        // иначе байты пришлось бы выбрасывать.
        assert_eq!(
            rel("media/%FF.png", None).part("word/document.xml"),
            Some("word/media/%FF.png".into())
        );
    }

    #[test]
    fn plus_is_not_a_space() {
        assert_eq!(
            rel("media/a+b.xml", None).part("word/document.xml"),
            Some("word/media/a+b.xml".into())
        );
    }

    #[test]
    fn external_targets_are_not_percent_decoded() {
        assert_eq!(
            rel("https://example.com/my%20file.docx", Some("External")).part("word/document.xml"),
            None
        );
    }

    #[test]
    fn rels_part_is_derived_from_the_source_part() {
        assert_eq!(rels_part("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
        assert_eq!(
            rels_part("word/document.xml"),
            "word/_rels/document.xml.rels"
        );
    }
}
