//! Разбор `xl/sharedStrings.xml` — общей таблицы строк книги.
//!
//! Повторяющиеся строки хранятся в книге один раз, а ячейки ссылаются на них
//! индексом ([`CellValue::SharedString`](crate::model::CellValue::SharedString)).
//! Разметка rich runs (`<r>`) пока склеивается в одну строку: украшения внутри
//! ячейки — дело рендера, а не модели.

use doc_converter_core::xml::XmlReader;
use quick_xml::events::Event;

use crate::error::{Result, XlsxError};

/// Общая таблица строк книги.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedStrings {
    items: Vec<String>,
}

impl SharedStrings {
    /// Разобрать часть `xl/sharedStrings.xml`.
    ///
    /// Текст читается ровно как записан: `quick-xml` не смотрит на `xml:space`,
    /// поэтому пробелы сохраняет
    /// [`XmlReader::preserving`](doc_converter_core::xml::XmlReader::preserving).
    /// Фонетические подсказки (`<rPh>`) пропускаются — это не содержимое ячейки.
    ///
    /// # Errors
    ///
    /// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] —
    /// файл оборвался внутри `<si>`.
    pub fn parse(bytes: &[u8], part: impl Into<String>) -> Result<Self> {
        let part = part.into();
        let mut reader = XmlReader::preserving(bytes, part.clone());
        let mut items = Vec::new();

        while let Some(event) = reader.next_significant()? {
            match event {
                // `<si/>` — пустая строка, но индекс она занимает.
                Event::Start(start) if start.local_name().as_ref() == b"si" => {
                    items.push(read_item_text(&mut reader, &part, "si")?);
                }
                Event::Empty(empty) if empty.local_name().as_ref() == b"si" => {
                    items.push(String::new());
                }
                _ => {}
            }
        }

        Ok(Self { items })
    }

    /// Число строк в таблице.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Таблица пуста.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Строка по индексу из ячейки; `None` — индекс за пределами таблицы.
    #[must_use]
    pub fn get(&self, index: u32) -> Option<&str> {
        self.items.get(index as usize).map(String::as_str)
    }

    /// Обход строк в порядке индексов.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(String::as_str)
    }
}

/// Дочитать текст элемента `<si>` или `<is>` до парного закрывающего тега.
///
/// Вызывается, когда читатель стоит сразу за открывающим тегом `<si>`/`<is>`.
/// Текст всех `<t>` склеивается в одну строку, фонетические подсказки (`<rPh>`)
/// пропускаются: это не содержимое ячейки. Общая для `sharedStrings.xml` и для
/// `inlineStr` внутри листа — структура у них одна и та же.
///
/// # Errors
///
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] — файл
/// оборвался внутри элемента.
pub(crate) fn read_item_text(reader: &mut XmlReader<'_>, part: &str, tag: &str) -> Result<String> {
    let mut text = String::new();
    // Глубина вложенности внутри элемента: на нуле закрывающий тег наш.
    let mut depth = 0usize;
    let mut in_text = false;
    let mut in_phonetic = false;

    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(start) => {
                match start.local_name().as_ref() {
                    b"rPh" => in_phonetic = true,
                    b"t" if !in_phonetic => in_text = true,
                    _ => {}
                }
                depth += 1;
            }
            Event::Text(chunk) if in_text => {
                let decoded = chunk
                    .unescape()
                    .map_err(|e| XlsxError::malformed(part, format!("bad text: {e}")))?;
                text.push_str(&decoded);
            }
            Event::CData(chunk) if in_text => {
                // В CDATA подстановки не действуют: содержимое и так буквальное.
                let raw = chunk.into_inner();
                let decoded = std::str::from_utf8(&raw)
                    .map_err(|e| XlsxError::malformed(part, format!("bad CDATA: {e}")))?;
                text.push_str(decoded);
            }
            Event::End(end) => {
                if depth == 0 {
                    return Ok(text);
                }
                depth -= 1;
                match end.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"rPh" => in_phonetic = false,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Err(XlsxError::malformed(
        part,
        format!("unexpected end of file inside <{tag}>"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> SharedStrings {
        SharedStrings::parse(xml.as_bytes(), "xl/sharedStrings.xml").unwrap()
    }

    #[test]
    fn plain_items() {
        let table = parse(
            r#"<?xml version="1.0" encoding="UTF-8"?>
               <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                    count="3" uniqueCount="2">
                 <si><t>Привет</t></si>
                 <si><t>world</t></si>
               </sst>"#,
        );

        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
        assert_eq!(table.get(0), Some("Привет"));
        assert_eq!(table.get(1), Some("world"));
        assert_eq!(table.get(2), None);
        assert_eq!(table.iter().collect::<Vec<_>>(), vec!["Привет", "world"]);
    }

    #[test]
    fn preserves_whitespace() {
        let table = parse(
            r#"<sst>
                 <si><t xml:space="preserve">  два пробела  </t></si>
                 <si><t>обрезать нечего</t></si>
               </sst>"#,
        );

        assert_eq!(table.get(0), Some("  два пробела  "));
        assert_eq!(table.get(1), Some("обрезать нечего"));
    }

    #[test]
    fn concatenates_runs_and_skips_phonetics() {
        let table = parse(
            r#"<sst>
                 <si>
                   <r><rPr><b/></rPr><t>Hello </t></r>
                   <r><t xml:space="preserve">world</t></r>
                   <rPh sb="0" eb="5"><t>へろー</t></rPh>
                   <phoneticPr fontId="1"/>
                 </si>
               </sst>"#,
        );

        assert_eq!(table.get(0), Some("Hello world"));
    }

    #[test]
    fn unescapes_entities_and_keeps_cdata_literal() {
        let table = parse(
            r"<sst>
                 <si><t>a &amp; b &lt;c&gt; &quot;d&quot;</t></si>
                 <si><t><![CDATA[a & b <c>]]></t></si>
               </sst>",
        );

        assert_eq!(table.get(0), Some(r#"a & b <c> "d""#));
        assert_eq!(table.get(1), Some("a & b <c>"));
    }

    #[test]
    fn empty_strings_still_take_an_index() {
        let table = parse("<sst><si/><si><t/></si><si><t>x</t></si></sst>");

        assert_eq!(table.len(), 3);
        assert_eq!(table.get(0), Some(""));
        assert_eq!(table.get(1), Some(""));
        assert_eq!(table.get(2), Some("x"));
    }

    #[test]
    fn prefixed_names_are_accepted() {
        let table = parse(
            r#"<x:sst xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                 <x:si><x:t>ok</x:t></x:si>
               </x:sst>"#,
        );

        assert_eq!(table.get(0), Some("ok"));
    }

    #[test]
    fn empty_part_is_an_empty_table() {
        assert!(parse("<sst/>").is_empty());
        assert!(SharedStrings::default().is_empty());
    }

    #[test]
    fn broken_xml_is_an_error() {
        let err = SharedStrings::parse(b"<sst><si><t>x</t></si></sst><", "xl/sharedStrings.xml")
            .unwrap_err();

        assert!(matches!(err, XlsxError::Core(_)));
    }

    #[test]
    fn truncated_last_item_is_an_error() {
        let err = SharedStrings::parse(b"<sst><si><t>x</t>", "xl/sharedStrings.xml").unwrap_err();

        assert!(matches!(err, XlsxError::Malformed { .. }));
        assert!(err.to_string().contains("unexpected end of file"));
    }

    #[test]
    fn nested_items_are_read_as_one_string() {
        // Схема вложенных `<si>` не допускает; если такие всё же встретятся,
        // текст склеивается, а не теряется.
        let table = parse("<sst><si><t>a</t><si><t>b</t></si></si></sst>");

        assert_eq!(table.len(), 1);
        assert_eq!(table.get(0), Some("ab"));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Текст, допустимый в XML 1.0: без управляющих символов, но со спецсимволами.
    const TEXT: &str = "[a-zA-Z0-9 _&<>'\"\u{0400}-\u{04FF}]{0,24}";

    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    proptest! {
        /// Что записали в `<si>`, то и должны прочитать — байт в байт.
        #[test]
        fn round_trips_escaped_text(items in proptest::collection::vec(TEXT, 1..6)) {
            let mut xml = String::from("<sst>");
            for item in &items {
                xml.push_str("<si><t xml:space=\"preserve\">");
                xml.push_str(&escape_xml(item));
                xml.push_str("</t></si>");
            }
            xml.push_str("</sst>");

            let table = SharedStrings::parse(xml.as_bytes(), "xl/sharedStrings.xml").unwrap();
            prop_assert_eq!(table.len(), items.len());
            for (i, item) in items.iter().enumerate() {
                let index = u32::try_from(i).unwrap();
                prop_assert_eq!(table.get(index), Some(item.as_str()));
            }
        }
    }
}
