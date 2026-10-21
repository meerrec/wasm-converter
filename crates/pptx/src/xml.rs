//! Лёгкое DOM-дерево частей PPTX поверх `quick-xml` (ADR-0033).
//!
//! Части `PresentationML` невелики (лимиты ADR-0015), а структура вложенная —
//! `p:sp` → `p:txBody` → `a:p` → `a:r` → `a:rPr`. Стримовый разбор такой
//! вложенности стоил бы явного стека состояний на каждый уровень; DOM проще и
//! читается как сам XML. Имена узлов — локальные (без префикса пространства
//! имён), атрибуты — как записаны (`r:id` остаётся с префиксом: на `p:sldId`
//! соседствуют `id` и `r:id`). Текст — с развёрнутыми сущностями и без
//! обрезки пробелов (пробелы в тексте слайда — данные).
//!
//! Глубина вложенности ограничена [`MAX_DEPTH`]: битый пакет с миллионом
//! вложенных элементов не уронит стек и не съест память.

use std::collections::BTreeMap;

use doc_converter_core::xml::XmlReader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, Event};
use quick_xml::XmlVersion;

use crate::error::{Error, Result};

/// Предел вложенности DOM (ADR-0033: оборона от битых пакетов).
const MAX_DEPTH: usize = 128;

/// Узел дерева: локальное имя, атрибуты по локальным именам, дети и текст.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) attrs: BTreeMap<String, String>,
    pub(crate) children: Vec<Node>,
    pub(crate) text: String,
}

impl Node {
    /// Значение атрибута по локальному имени (`r:id` → `id`).
    #[must_use]
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    /// Первый ребёнок с таким локальным именем.
    #[must_use]
    pub(crate) fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Все дети с таким локальным именем.
    pub(crate) fn children_named<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Node> {
        let name = name.to_owned();
        self.children.iter().filter(move |child| child.name == name)
    }

    /// Весь текст поддерева — собственный плюс детей (для заголовков).
    #[must_use]
    pub(crate) fn text_all(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        out.push_str(&self.text);
        for child in &self.children {
            out.push_str(&child.text_all());
        }
        out
    }
}

/// Разобрать часть пакета в DOM.
///
/// # Errors
/// [`Error::Core`] — XML не читается (битые байты, обрыв потока);
/// [`Error::Malformed`] — неизвестная сущность или превышена глубина.
pub(crate) fn parse_dom(bytes: &[u8], part: &str) -> Result<Node> {
    let mut reader = XmlReader::preserving(bytes, part);
    let mut stack: Vec<Node> = Vec::new();
    let mut root = Node::default();

    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(element) => {
                let node = node_from(&element, part)?;
                if stack.len() >= MAX_DEPTH {
                    return Err(Error::Malformed(format!(
                        "{part}: nesting deeper than {MAX_DEPTH}"
                    )));
                }
                stack.push(node);
            }
            Event::Empty(element) => {
                let node = node_from(&element, part)?;
                push_node(&mut stack, &mut root, node);
            }
            Event::End(_) => {
                if let Some(node) = stack.pop() {
                    push_node(&mut stack, &mut root, node);
                }
            }
            Event::Text(text) => {
                let decoded = text
                    .xml10_content()
                    .map_err(|e| Error::Malformed(format!("{part}: bad text: {e}")))?;
                push_text(&mut stack, &mut root, &decoded);
            }
            Event::CData(data) => {
                let decoded = data
                    .xml10_content()
                    .map_err(|e| Error::Malformed(format!("{part}: bad CDATA: {e}")))?;
                push_text(&mut stack, &mut root, &decoded);
            }
            Event::GeneralRef(reference) => {
                push_text(&mut stack, &mut root, &resolve_reference(&reference, part)?);
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }

    // Битая часть: непарный корневой элемент. Терпимо — DOM остаётся тем, что
    // удалось прочитать: предупреждение разбор добавит, а не уронит пакет.
    for node in stack {
        root.children.push(node);
    }
    Ok(root)
}

/// Добавить завершённый узел текущему родителю или корню.
fn push_node(stack: &mut [Node], root: &mut Node, node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        root.children.push(node);
    }
}

/// Дописать текст текущему (самому вложенному) узлу или корню.
fn push_text(stack: &mut [Node], root: &mut Node, text: &str) {
    if let Some(node) = stack.last_mut() {
        node.text.push_str(text);
    } else {
        root.text.push_str(text);
    }
}

/// Узел из открывающего элемента: локальное имя и атрибуты.
///
/// Ключ атрибута — полное имя с префиксом, если префикс есть (`r:id`), и
/// локальное без него (`cx`): на `p:sldId` соседствуют `id` и `r:id`, и
/// локальное имя схлопнуло бы их в один ключ.
fn node_from(element: &quick_xml::events::BytesStart<'_>, part: &str) -> Result<Node> {
    let name = local_name(element.name().as_ref())
        .ok_or_else(|| Error::Malformed(format!("{part}: empty element name")))?;
    let mut attrs = BTreeMap::new();
    for attr in element.attributes() {
        let attr = attr.map_err(|e| Error::Malformed(format!("{part}: bad attribute: {e}")))?;
        let key = String::from_utf8(attr.key.as_ref().to_vec())
            .map_err(|_| Error::Malformed(format!("{part}: non-utf8 attribute name")))?;
        if key.is_empty() {
            return Err(Error::Malformed(format!("{part}: empty attribute name")));
        }
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| Error::Malformed(format!("{part}: bad attribute value: {e}")))?;
        attrs.insert(key, value.into_owned());
    }
    Ok(Node {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
    })
}

/// Локальное имя без префикса пространства имён: `p:sp` → `sp`.
fn local_name(name: &[u8]) -> Option<String> {
    let local = match name.iter().position(|&b| b == b':') {
        Some(colon) => &name[colon + 1..],
        None => name,
    };
    if local.is_empty() {
        return None;
    }
    String::from_utf8(local.to_vec()).ok()
}

/// Развернуть ссылку на сущность (`&amp;`, `&#65;`, `&#x41;`).
///
/// quick-xml 0.41 отдаёт ссылку отдельным событием; предопределённые и
/// числовые — всё, что встречается в OOXML. Незнакомая ссылка — порча файла.
fn resolve_reference(reference: &BytesRef<'_>, part: &str) -> Result<String> {
    let name = reference
        .decode()
        .map_err(|e| Error::Malformed(format!("{part}: bad reference: {e}")))?;
    if let Some(ch) = reference
        .resolve_char_ref()
        .map_err(|e| Error::Malformed(format!("{part}: bad character reference: {e}")))?
    {
        return Ok(ch.to_string());
    }
    resolve_predefined_entity(&name)
        .map(str::to_owned)
        .ok_or_else(|| Error::Malformed(format!("{part}: unknown entity `&{name};`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_preserves_text_and_resolves_entities() {
        let dom = parse_dom(
            br#"<p:sp xmlns:p="urn:x"><p:nvSpPr><p:cNvPr name="Box 1"/></p:nvSpPr><p:txBody><a:p><a:r><a:t>A &amp; B</a:t></a:r></a:p></p:txBody></p:sp>"#,
            "test.xml",
        )
        .unwrap();
        let sp = dom.child("sp").unwrap();
        assert_eq!(
            sp.child("nvSpPr")
                .unwrap()
                .child("cNvPr")
                .unwrap()
                .attr("name"),
            Some("Box 1")
        );
        let text = sp.child("txBody").unwrap().text_all();
        assert_eq!(text, "A & B");
    }

    #[test]
    fn dom_keeps_prefixed_attribute_names() {
        let dom = parse_dom(br#"<p:sldId id="256" r:id="rId2"/>"#, "test.xml").unwrap();
        let sld = dom.child("sldId").unwrap();
        assert_eq!(sld.attr("id"), Some("256"));
        assert_eq!(sld.attr("r:id"), Some("rId2"));
    }

    #[test]
    fn deep_nesting_is_rejected() {
        let mut xml = String::from("<root>");
        for _ in 0..200 {
            xml.push_str("<x>");
        }
        xml.push_str("</x>".repeat(200).as_str());
        xml.push_str("</root>");
        assert!(matches!(
            parse_dom(xml.as_bytes(), "deep.xml"),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(matches!(
            parse_dom(b"<a><b>ok</a>", "broken.xml"),
            Err(Error::Core(_))
        ));
    }
}
