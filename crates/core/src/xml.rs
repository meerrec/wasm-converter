//! Streaming XML helpers over `quick-xml`.
//!
//! Вторая половина модуля — общие хелперы атрибутов OOXML (ADR-0017): раньше
//! DOCX и XLSX несли свои копии, теперь канон здесь. Ошибки возвращаются как
//! [`Error::Malformed`] **без имени части** — часть в сообщение добавляет
//! форматный маппер (`docx::settings::xml_error` и аналог XLSX): так сообщение
//! остаётся байт-в-байт прежним, а `Malformed` в core не знает частей.

use std::borrow::Cow;
use std::str::FromStr;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::error::{Error, Result};

pub struct XmlReader<'a> {
    inner: Reader<&'a [u8]>,
    buf: Vec<u8>,
    part: String,
}

impl<'a> XmlReader<'a> {
    /// Читатель, обрезающий пробелы по краям текстовых узлов.
    ///
    /// Годится для частей, где текст незначим (`_rels/*.rels`), но не для
    /// `sharedStrings.xml` или `<w:t>` — там пробелы это данные, и нужен
    /// [`XmlReader::preserving`].
    #[must_use]
    pub fn new(bytes: &'a [u8], part: impl Into<String>) -> Self {
        Self::with_trim(bytes, part, true)
    }

    /// Читатель, отдающий текст ровно так, как он записан.
    ///
    /// `quick-xml` не смотрит на `xml:space`, поэтому «preserve» обеспечивается
    /// здесь: `trim_text` выключен, и `<t xml:space="preserve"> x </t>` доходит
    /// без потерь.
    #[must_use]
    pub fn preserving(bytes: &'a [u8], part: impl Into<String>) -> Self {
        Self::with_trim(bytes, part, false)
    }

    fn with_trim(bytes: &'a [u8], part: impl Into<String>, trim: bool) -> Self {
        let mut inner = Reader::from_reader(bytes);
        inner.config_mut().trim_text(trim);
        Self {
            inner,
            buf: Vec::new(),
            part: part.into(),
        }
    }

    /// Читает следующий значимый event, пропуская `Decl`/`Comment`/`PI`.
    ///
    /// # Errors
    /// Если XML не разбирается.
    pub fn next_significant(&mut self) -> Result<Option<Event<'static>>> {
        loop {
            self.buf.clear();
            let ev = self
                .inner
                .read_event_into(&mut self.buf)
                .map_err(|e| Error::Xml {
                    part: self.part.clone(),
                    position: self.inner.buffer_position(),
                    message: e.to_string(),
                })?;
            match ev {
                Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
                Event::Eof => return Ok(None),
                other => return Ok(Some(other.into_owned())),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Атрибуты
// ---------------------------------------------------------------------------

/// Атрибут: локальное имя без префикса (`r:id` → `id`) и значение с развёрнутыми сущностями.
#[derive(Debug)]
pub struct Attr<'a> {
    /// Имя без префикса пространства имён.
    pub name: &'a [u8],
    /// Значение с развёрнутыми XML-сущностями.
    pub value: Cow<'a, str>,
}

/// Разобрать атрибуты элемента.
///
/// Имя теряет префикс пространства имён, значение получает развёрнутые
/// сущности и приведённые к пробелам переводы строк — как того требует XML для
/// значений атрибутов. Незнакомое имя атрибута ошибкой не считается: OOXML из
/// реального мира несёт лишние атрибуты, и терять из-за них документ нельзя.
///
/// `SmallVec` — атрибутов у элемента обычно 0–4, а читается он на каждой
/// ячейке, и аллокаций по пути быть не должно.
///
/// # Errors
/// [`Error::Malformed`] — атрибут не читается: битые байты, дубликат имени или
/// некорректная XML-сущность в значении. Имя части добавляет вызывающий.
pub fn attributes<'a>(element: &'a BytesStart<'a>) -> Result<SmallVec<[Attr<'a>; 4]>> {
    let mut attrs = SmallVec::new();
    for attr in element.attributes() {
        let attr = attr.map_err(|e| Error::Malformed(format!("bad attribute: {e}")))?;
        // XML-декларацию ридер пропускает, поэтому версия — 1.0.
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| Error::Malformed(format!("bad attribute value: {e}")))?;
        // `into_inner` отдаёт срез данных элемента, а не `attr`: иначе имя
        // ссылалось бы на локальную переменную цикла и не пережило бы его.
        attrs.push(Attr {
            name: local_name(attr.key.into_inner()),
            value,
        });
    }
    Ok(attrs)
}

/// Значение атрибута по локальному имени: `r:id` и `w:id` находятся оба.
#[must_use]
pub fn find<'a>(attrs: &'a [Attr<'_>], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.name == name.as_bytes())
        .map(|attr| attr.value.as_ref())
}

/// `xsd:boolean` OOXML: истина — `1`, `true`, `on`.
///
/// `on` схема допускает наравне с `true` (Word пишет и так, и так), а XLSX
/// пишет `1`/`true`; надмножество покрывает оба формата.
#[must_use]
pub fn is_true(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

/// Локальное имя без префикса пространства имён: `w:p` → `p`.
#[must_use]
pub fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(colon) => &name[colon + 1..],
        None => name,
    }
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
/// [`Error::Malformed`] — ссылка не декодируется, содержит недопустимый код
/// символа или ссылается на неизвестную сущность. Имя части добавляет вызывающий.
pub fn resolve_reference(reference: &BytesRef<'_>) -> Result<String> {
    let name = reference
        .decode()
        .map_err(|e| Error::Malformed(format!("bad reference: {e}")))?;
    if let Some(ch) = reference
        .resolve_char_ref()
        .map_err(|e| Error::Malformed(format!("bad character reference: {e}")))?
    {
        return Ok(ch.to_string());
    }
    resolve_predefined_entity(&name)
        .map(str::to_owned)
        .ok_or_else(|| Error::Malformed(format!("unknown entity `&{name};`")))
}

// ---------------------------------------------------------------------------
// Значения свойств
// ---------------------------------------------------------------------------

/// Тумблер OOXML (`w:b`, `w:keepNext`, …): элемент с `w:val`, но без полезной
/// нагрузки.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Toggle {
    /// Включено; элемент без `w:val` значит то же самое — потому `On` и есть `Default`.
    #[default]
    On,
    /// Выключено явно (`w:val` = `0`, `false`, `off`).
    Off,
    /// Наследуется от стиля (`w:val="inherit"`).
    Inherit,
}

impl Toggle {
    /// Разбирает `w:val` тумблера.
    ///
    /// `None` → `Some(On)` (элемент без `w:val` включён); `"0"`, `"false"`, `"off"` → `Off`;
    /// `"inherit"` → `Inherit`; `"1"`, `"true"`, `"on"` → `On`; прочее → `None`
    /// (парсер обязан выдать `InvalidAttribute` и взять `On`).
    /// Регистр не важен, пробелы по краям обрезаются.
    #[must_use]
    pub fn from_val(value: Option<&str>) -> Option<Self> {
        let Some(raw) = value else {
            return Some(Self::On);
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "on" => Some(Self::On),
            "0" | "false" | "off" => Some(Self::Off),
            "inherit" => Some(Self::Inherit),
            _ => None,
        }
    }
}

/// Тоггл (`w:b`, `w:i`, …): отсутствие `w:val` → [`Toggle::On`].
///
/// Наличие элемента свойство включает — так это читает и Word; поэтому
/// нераспознанное значение (`w:val="bogus"`) — это предупреждение (через
/// `warn`) и [`Toggle::On`], а не отказ. Элемент-пустышка (`<w:b/>`) атрибутов
/// не имеет вовсе, и это тоже `On`, а не ошибка.
///
/// `warn` вызывается с текстом предупреждения и может вернуть ошибку формата
/// (например, «предупреждений стало больше порога»).
///
/// # Errors
/// Если `warn` вернул ошибку.
pub fn attr_toggle<E>(
    attrs: &[Attr<'_>],
    element: &str,
    warn: impl FnMut(&str) -> std::result::Result<(), E>,
) -> std::result::Result<Option<Toggle>, E> {
    let mut warn = warn;
    let Some(raw) = find(attrs, "val") else {
        return Ok(Some(Toggle::On));
    };
    if let Some(toggle) = Toggle::from_val(Some(raw)) {
        return Ok(Some(toggle));
    }
    warn(&format!(
        "`{element}`: unrecognized `w:val` `{raw}`, the property counts as on"
    ))?;
    Ok(Some(Toggle::On))
}

/// Разобрать числовой атрибут: нет атрибута → `None`, мусор → предупреждение и `None`.
fn parse_attr<T: FromStr, E>(
    attrs: &[Attr<'_>],
    name: &str,
    warn: &mut impl FnMut(&str) -> std::result::Result<(), E>,
) -> std::result::Result<Option<T>, E> {
    let Some(raw) = find(attrs, name) else {
        return Ok(None);
    };
    if let Ok(value) = raw.trim().parse::<T>() {
        return Ok(Some(value));
    }
    warn(&format!("`{name}`: `{raw}` is not a number, ignored"))?;
    Ok(None)
}

/// Целочисленный атрибут (`w:val`, `w:w`, …).
///
/// `warn` — приёмник предупреждения о мусорном значении (см. [`attr_toggle`]).
///
/// # Errors
/// Если `warn` вернул ошибку.
pub fn attr_i32<E>(
    attrs: &[Attr<'_>],
    name: &str,
    warn: impl FnMut(&str) -> std::result::Result<(), E>,
) -> std::result::Result<Option<i32>, E> {
    parse_attr(attrs, name, &mut { warn })
}

/// Атрибут-счётчик.
///
/// # Errors
/// Те же, что у [`attr_i32`].
pub fn attr_u32<E>(
    attrs: &[Attr<'_>],
    name: &str,
    warn: impl FnMut(&str) -> std::result::Result<(), E>,
) -> std::result::Result<Option<u32>, E> {
    parse_attr(attrs, name, &mut { warn })
}

/// Атрибут-мера (pt, доля, множитель).
///
/// `inf` и `nan` `f32` разбирает, но для меры они бессмысленны и отравили бы
/// раскладку — поэтому считаются порчей файла, как и прочий мусор.
///
/// # Errors
/// Те же, что у [`attr_i32`].
pub fn attr_f32<E>(
    attrs: &[Attr<'_>],
    name: &str,
    warn: impl FnMut(&str) -> std::result::Result<(), E>,
) -> std::result::Result<Option<f32>, E> {
    let mut warn = warn;
    let Some(raw) = find(attrs, name) else {
        return Ok(None);
    };
    match raw.trim().parse::<f32>() {
        Ok(value) if value.is_finite() => Ok(Some(value)),
        Ok(_) | Err(_) => {
            warn(&format!("`{name}`: `{raw}` is not a number, ignored"))?;
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Цвет
// ---------------------------------------------------------------------------

/// Цвет из атрибутов `rgb`, `theme` или `indexed` — до резолва палитр.
///
/// `rgb` бывает восьмизначным (`AARRGGBB`) и шестизначным (`RRGGBB`); во втором
/// случае альфы в файле нет, и она считается непрозрачной.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpec {
    /// Цвет не задан — действует унаследованный.
    None,
    /// `rgb="AARRGGBB"`.
    Rgb(u32),
    /// `theme="n"` — индекс в палитре темы.
    Theme(u32),
    /// `indexed="n"` — индекс в устаревшей палитре.
    Indexed(u32),
}

/// Разобрать цвет из атрибутов.
#[must_use]
pub fn color(attrs: &[Attr<'_>]) -> ColorSpec {
    if let Some(rgb) = find(attrs, "rgb") {
        let digits = rgb.trim().trim_start_matches('#');
        if let Ok(value) = u32::from_str_radix(digits, 16) {
            // Шестизначная запись — это `RRGGBB` без альфы, а не полностью
            // прозрачный цвет: без этой поправки заливки исчезали бы.
            return ColorSpec::Rgb(if digits.len() <= 6 {
                value | 0xFF00_0000
            } else {
                value
            });
        }
    }
    if let Some(theme) = find(attrs, "theme").and_then(|value| value.trim().parse().ok()) {
        return ColorSpec::Theme(theme);
    }
    if let Some(indexed) = find(attrs, "indexed").and_then(|value| value.trim().parse().ok()) {
        return ColorSpec::Indexed(indexed);
    }
    ColorSpec::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_are_localized_and_resolved() {
        let bytes = br#"<x:rPr x:b="1" x:name="A &amp; B"/>"#;
        let mut reader = XmlReader::preserving(bytes, "test.xml");
        let Event::Empty(el) = reader.next_significant().unwrap().unwrap() else {
            panic!("ожидался Empty-элемент");
        };
        let attrs = attributes(&el).unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].name, b"b");
        assert_eq!(attrs[1].name, b"name");
        assert_eq!(attrs[1].value, "A & B");
    }

    #[test]
    fn truth_knows_all_oooxml_forms() {
        assert!(is_true("1"));
        assert!(is_true("true"));
        assert!(is_true("on"));
        assert!(!is_true("0"));
        assert!(!is_true("false"));
    }

    #[test]
    fn color_rgb_without_alpha_is_opaque() {
        let bytes = br#"<x:c rgb="FF0000"/>"#;
        let mut reader = XmlReader::preserving(bytes, "test.xml");
        let Event::Empty(el) = reader.next_significant().unwrap().unwrap() else {
            panic!("ожидался Empty-элемент");
        };
        let attrs = attributes(&el).unwrap();
        assert_eq!(color(&attrs), ColorSpec::Rgb(0xFFFF_0000));
    }

    #[test]
    fn toggle_accepts_on_off_inherit() {
        assert_eq!(Toggle::from_val(None), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("true")), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("OFF")), Some(Toggle::Off));
        assert_eq!(Toggle::from_val(Some("inherit")), Some(Toggle::Inherit));
        assert_eq!(Toggle::from_val(Some("bogus")), None);
    }
}
