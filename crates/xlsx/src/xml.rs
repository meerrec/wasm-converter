//! Чтение OOXML-атрибутов.
//!
//! В OOXML имя атрибута приходит с префиксом пространства имён (`r:id`), а
//! значение — с XML-сущностями (`Доходы &amp; расходы`). Здесь они приводятся
//! к рабочему виду: локальное имя плюс развёрнутое значение.
//!
//! Аллокаций по пути нет, пока в значении не встретится сущность: атрибуты
//! читаются на каждой ячейке, а их в книге бывает миллион.

use std::borrow::Cow;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart};
use quick_xml::XmlVersion;
use smallvec::SmallVec;

use crate::error::{Result, XlsxError};

/// Атрибут: локальное имя без префикса и развёрнутое значение.
#[derive(Debug)]
pub(crate) struct Attr<'a> {
    /// Имя без префикса: `r:id` → `id`.
    name: &'a [u8],
    /// Значение с развёрнутыми XML-сущностями.
    value: Cow<'a, str>,
}

/// Разобрать атрибуты элемента.
///
/// # Errors
///
/// [`XlsxError::Malformed`] — атрибут не читается: битые байты или
/// некорректная XML-сущность в значении.
pub(crate) fn attributes<'a>(
    element: &'a BytesStart<'a>,
    part: &str,
) -> Result<SmallVec<[Attr<'a>; 4]>> {
    let mut attrs = SmallVec::new();
    for attr in element.attributes() {
        let attr = attr.map_err(|e| XlsxError::malformed(part, format!("bad attribute: {e}")))?;
        // `normalized_value` разворачивает сущности и приводит переводы строк
        // к пробелам — как того требует XML для значений атрибутов.
        // XML-декларацию читатель пропускает, поэтому версия — по умолчанию 1.0.
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| XlsxError::malformed(part, format!("bad attribute value: {e}")))?;
        let key = attr.key.into_inner();
        let name = key
            .iter()
            .position(|&b| b == b':')
            .map_or(key, |colon| &key[colon + 1..]);
        attrs.push(Attr { name, value });
    }
    Ok(attrs)
}

/// Значение атрибута по локальному имени.
pub(crate) fn find<'a>(attrs: &'a [Attr<'_>], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.name == name.as_bytes())
        .map(|attr| attr.value.as_ref())
}

/// `xsd:boolean`: истина — `1` или `true`.
pub(crate) fn is_true(value: &str) -> bool {
    matches!(value, "1" | "true")
}

/// Развернуть ссылку на сущность (`&amp;`, `&#65;`, `&#x41;`) в текст.
///
/// Начиная с quick-xml 0.41 ссылки приходят отдельным событием, и разворачивать
/// их должен вызывающий. В OOXML встречаются только пять предопределённых
/// сущностей и числовые ссылки: внешние потребовали бы DTD, которого в этих
/// файлах не бывает, — поэтому незнакомая ссылка означает порчу файла, а не
/// повод что-то додумывать.
///
/// # Errors
///
/// [`XlsxError::Malformed`] — ссылка не декодируется, содержит недопустимый код
/// символа или ссылается на неизвестную сущность.
pub(crate) fn resolve_reference(reference: &BytesRef<'_>, part: &str) -> Result<String> {
    let name = reference
        .decode()
        .map_err(|e| XlsxError::malformed(part, format!("bad reference: {e}")))?;
    if let Some(ch) = reference
        .resolve_char_ref()
        .map_err(|e| XlsxError::malformed(part, format!("bad character reference: {e}")))?
    {
        return Ok(ch.to_string());
    }
    resolve_predefined_entity(&name)
        .map(str::to_owned)
        .ok_or_else(|| XlsxError::malformed(part, format!("unknown entity `&{name};`")))
}
