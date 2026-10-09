//! Общие XML-хелперы парсеров DOCX (слайс S6) и разбор `mc:AlternateContent` (ADR-0014).
//!
//! От `crates/xlsx/src/xml.rs` отличает толерантность: незнакомый атрибут или
//! элемент — это данные, а не ошибка (ADR-0016), фатально только то, что мешает
//! читать сам XML. Поэтому здесь [`Error::malformed`] — про битые байты и
//! обрыв потока, а всё восстановимое уходит предупреждениями в [`ParseCtx`].
//!
//! События приходят из `XmlReader` уже owned (`Event<'static>`), так что
//! буферизовать их можно свободно — на этом стоит разбор `mc:AlternateContent`:
//! ветку нельзя выбрать, не дочитав блок до конца.

// Потребителей у хелперов ещё нет: их подключат парсеры частей (слайсы S7–S12).
// До тех пор `dead_code` срабатывал бы на каждом элементе модуля; `allow`
// снимается вместе с подключением — как в `context.rs`.
#![allow(dead_code)]

use std::borrow::Cow;
use std::str::FromStr;

use doc_converter_core::xml::XmlReader;
use doc_converter_core::WarningKind;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::XmlVersion;

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::raw::Toggle;

/// Предел вложенности `mc:AlternateContent` (ADR-0014 §4).
const MAX_ALTERNATE_DEPTH: u32 = 16;

/// Максимальная глубина поддерева, которое сохраняет [`capture_element`].
///
/// Глубже — не сохраняем: `Unknown` не должен тянуть за собой произвольно
/// большой кусок чужого XML.
const MAX_CAPTURE_DEPTH: u32 = 32;

/// Префиксы, которые `mc:Choice/@Requires` может требовать от нас (ADR-0014 §2).
///
/// `v` и `o` не поддержаны: VML — non-goal, а `mc:Fallback` не разбирается
/// никогда.
const SUPPORTED_REQUIRES: [&str; 5] = ["wps", "wpg", "wpc", "w14", "wp14"];

// ---------------------------------------------------------------------------
// Атрибуты
// ---------------------------------------------------------------------------

/// Атрибут: локальное имя без префикса (`r:id` → `id`) и значение с развёрнутыми сущностями.
#[derive(Debug)]
pub(crate) struct Attr<'a> {
    pub(crate) name: &'a [u8],
    pub(crate) value: Cow<'a, str>,
}

/// Разобрать атрибуты элемента.
///
/// Имя теряет префикс пространства имён, значение получает развёрнутые
/// сущности и приведённые к пробелам переводы строк — как того требует XML для
/// значений атрибутов. Незнакомое имя атрибута ошибкой не считается: OOXML из
/// реального мира несёт лишние атрибуты, и терять из-за них документ нельзя.
///
/// Возвращается `Vec`, а не `SmallVec`: список живёт ровно до конца разбора
/// элемента, а тип `Vec` — тот, что записан в контракте слайса.
///
/// # Errors
/// [`Error::malformed`] — атрибут не читается: битые байты, дубликат имени или
/// некорректная XML-сущность в значении.
pub(crate) fn attributes<'a>(element: &'a BytesStart<'a>, part: &str) -> Result<Vec<Attr<'a>>> {
    let mut attrs = Vec::new();
    for attr in element.attributes() {
        let attr = attr.map_err(|e| Error::malformed(part, format!("bad attribute: {e}")))?;
        // XML-декларацию ридер пропускает, поэтому версия — 1.0.
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| Error::malformed(part, format!("bad attribute value: {e}")))?;
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
pub(crate) fn find<'a>(attrs: &'a [Attr<'_>], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.name == name.as_bytes())
        .map(|attr| attr.value.as_ref())
}

/// `xsd:boolean` OOXML: истина — `1`, `true`, `on`.
///
/// `on` схема допускает наравне с `true`, а Word пишет и так, и так.
#[must_use]
pub(crate) fn is_true(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

/// Локальное имя без префикса пространства имён: `w:p` → `p`.
#[must_use]
pub(crate) fn local_name(name: &[u8]) -> &[u8] {
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
/// [`Error::malformed`] — ссылка не декодируется, содержит недопустимый код
/// символа или ссылается на неизвестную сущность.
pub(crate) fn resolve_reference(reference: &BytesRef<'_>, part: &str) -> Result<String> {
    let name = reference
        .decode()
        .map_err(|e| Error::malformed(part, format!("bad reference: {e}")))?;
    if let Some(ch) = reference
        .resolve_char_ref()
        .map_err(|e| Error::malformed(part, format!("bad character reference: {e}")))?
    {
        return Ok(ch.to_string());
    }
    resolve_predefined_entity(&name)
        .map(str::to_owned)
        .ok_or_else(|| Error::malformed(part, format!("unknown entity `&{name};`")))
}

// ---------------------------------------------------------------------------
// Значения свойств
// ---------------------------------------------------------------------------

/// Тоггл (`w:b`, `w:i`, …): отсутствие `w:val` → [`Toggle::On`].
///
/// Наличие элемента свойство включает — так это читает и Word; поэтому
/// нераспознанное значение (`w:val="bogus"`) — это предупреждение и [`Toggle::On`],
/// а не отказ. Элемент-пустышка (`<w:b/>`) атрибутов не имеет вовсе, и это тоже
/// `On`, а не ошибка.
///
/// # Errors
/// [`Error::malformed`] — атрибуты не читаются; [`Error::TooManyWarnings`] —
/// предупреждений стало больше порога.
pub(crate) fn attr_toggle(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
    element: &str,
) -> Result<Option<Toggle>> {
    let Some(raw) = find(attrs, "val") else {
        return Ok(Some(Toggle::On));
    };
    if let Some(toggle) = Toggle::from_val(Some(raw)) {
        return Ok(Some(toggle));
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`{element}`: unrecognized `w:val` `{raw}`, the property counts as on"),
    )?;
    Ok(Some(Toggle::On))
}

/// Целочисленный атрибут (`w:val`, `w:w`, …).
///
/// # Errors
/// [`Error::malformed`] — атрибуты не читаются; [`Error::TooManyWarnings`] — порог.
pub(crate) fn attr_i32(
    attrs: &[Attr<'_>],
    name: &str,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<i32>> {
    parse_attr(attrs, name, ctx, part)
}

/// Атрибут-счётчик.
///
/// # Errors
/// Те же, что у [`attr_i32`].
pub(crate) fn attr_u32(
    attrs: &[Attr<'_>],
    name: &str,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<u32>> {
    parse_attr(attrs, name, ctx, part)
}

/// Атрибут-мера (pt, доля, множитель).
///
/// `inf` и `nan` `f32` разбирает, но для меры они бессмысленны и отравили бы
/// раскладку Спринта 9 — поэтому считаются порчей файла, как и прочий мусор.
///
/// # Errors
/// Те же, что у [`attr_i32`].
pub(crate) fn attr_f32(
    attrs: &[Attr<'_>],
    name: &str,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<f32>> {
    let Some(raw) = find(attrs, name) else {
        return Ok(None);
    };
    match raw.trim().parse::<f32>() {
        Ok(value) if value.is_finite() => Ok(Some(value)),
        Ok(_) | Err(_) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`{name}`: `{raw}` is not a number, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Разобрать числовой атрибут: нет атрибута → `None`, мусор → предупреждение и `None`.
fn parse_attr<T: FromStr>(
    attrs: &[Attr<'_>],
    name: &str,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<T>> {
    let Some(raw) = find(attrs, name) else {
        return Ok(None);
    };
    if let Ok(value) = raw.trim().parse::<T>() {
        return Ok(Some(value));
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`{name}`: `{raw}` is not a number, ignored"),
    )?;
    Ok(None)
}

// ---------------------------------------------------------------------------
// Сохранение поддерева
// ---------------------------------------------------------------------------

/// Сериализовать поддерево начатого элемента обратно в XML (для `Unknown`).
///
/// `start` — уже прочитанный `Start`-элемент; функция дочитывает поток до его
/// парного `End` включительно. Байты тегов, текста и ссылок переносятся как
/// есть (`attributes_raw()`): фрагмент — точная копия исходного XML, а не его
/// интерпретация, поэтому сущности не разворачиваются, а `&amp;` остаётся
/// `&amp;` — иначе повторный разбор фрагмента дал бы другой текст.
///
/// Вложенность ограничена [`MAX_CAPTURE_DEPTH`]: глубже поддерево в вывод не
/// попадает. Вход при этом всё равно вычитывается до парного `End` — иначе
/// следующий `next_significant` вернул бы вызывающему чужие закрывающие теги.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше парного `End` или байты события
/// не UTF-8; [`Error::TooManyWarnings`] — предупреждений стало больше порога.
pub(crate) fn capture_element(
    reader: &mut XmlReader<'_>,
    start: &BytesStart<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<String> {
    let mut out = String::new();
    push_tag(&mut out, start, false, part)?;
    let mut depth: u32 = 1;
    let mut warned = false;
    loop {
        let event = reader
            .next_significant()?
            .ok_or_else(|| Error::malformed(part, "unexpected end of input"))?;
        match &event {
            Event::Start(_) if depth >= MAX_CAPTURE_DEPTH => {
                if !warned {
                    ctx.warn(
                        WarningKind::DeepNesting,
                        part,
                        format!("nesting deeper than {MAX_CAPTURE_DEPTH}; the subtree is dropped"),
                    )?;
                    warned = true;
                }
                skip_element(reader, part)?;
            }
            Event::Start(_) => {
                depth += 1;
                push_event(&mut out, &event, part)?;
            }
            Event::End(_) => {
                depth -= 1;
                push_event(&mut out, &event, part)?;
                if depth == 0 {
                    return Ok(out);
                }
            }
            _ => push_event(&mut out, &event, part)?,
        }
    }
}

/// Вычитать начатый элемент целиком, не заглядывая внутрь.
fn skip_element(reader: &mut XmlReader<'_>, part: &str) -> Result<()> {
    let mut depth: u32 = 0;
    loop {
        let event = reader
            .next_significant()?
            .ok_or_else(|| Error::malformed(part, "unexpected end of input"))?;
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(()),
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
}

/// Дописать событие во фрагмент дословно: теги, текст и ссылки — исходные байты.
fn push_event(out: &mut String, event: &Event<'_>, part: &str) -> Result<()> {
    match event {
        Event::Start(element) => push_tag(out, element, false, part),
        Event::Empty(element) => push_tag(out, element, true, part),
        Event::End(name) => {
            out.push_str("</");
            push_bytes(out, name, part)?;
            out.push('>');
            Ok(())
        }
        Event::Text(text) => push_bytes(out, text, part),
        Event::CData(data) => {
            out.push_str("<![CDATA[");
            push_bytes(out, data, part)?;
            out.push_str("]]>");
            Ok(())
        }
        Event::Comment(text) => {
            out.push_str("<!--");
            push_bytes(out, text, part)?;
            out.push_str("-->");
            Ok(())
        }
        Event::GeneralRef(reference) => {
            out.push('&');
            push_bytes(out, reference, part)?;
            out.push(';');
            Ok(())
        }
        // `next_significant` эти события не отдаёт, а `Eof` — конец потока.
        Event::Decl(_) | Event::PI(_) | Event::DocType(_) | Event::Eof => Ok(()),
    }
}

/// Дописать тег: имя и атрибуты — те же байты, что пришли в событии.
fn push_tag(out: &mut String, element: &BytesStart<'_>, empty: bool, part: &str) -> Result<()> {
    out.push('<');
    push_bytes(out, element, part)?;
    out.push_str(if empty { "/>" } else { ">" });
    Ok(())
}

/// Дописать сырые байты события.
fn push_bytes(out: &mut String, bytes: &[u8], part: &str) -> Result<()> {
    out.push_str(
        std::str::from_utf8(bytes)
            .map_err(|e| Error::malformed(part, format!("event bytes are not UTF-8: {e}")))?,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// mc:AlternateContent
// ---------------------------------------------------------------------------

/// Результат разбора `mc:AlternateContent` (ADR-0014 §1).
#[derive(Debug)]
pub(crate) enum AlternateContent {
    /// Внутренний XML выбранной `mc:Choice` (уже без обёртки) — его надо разобрать дальше.
    Choice(String),
    /// Ни одна ветка не поддержана (или глубина > 16): весь блок целиком — в `Unknown`.
    Unsupported(String),
}

/// Разобрать `mc:AlternateContent`, начатый элементом `start`, вместе с вложенными блоками.
///
/// Выбирается первая `mc:Choice`, чьи `Requires` поддержаны (пустой список —
/// поддерживается); поддерживаются `wps`, `wpg`, `wpc`, `w14`, `wp14`; `v`, `o` — нет
/// (ADR-0014 §2). `mc:Fallback` не разбирается никогда. Глубина вложенности > 16 →
/// предупреждение [`WarningKind::DeepNesting`] + [`AlternateContent::Unsupported`]:
/// слишком глубокий блок сохраняется как есть, без рекурсии.
///
/// Каждое предупреждение привязывается к `xml_path` — вызывающий знает путь до
/// блока, а сам блок после выбора ветки разбирается уже другим парсером.
///
/// # Errors
/// [`Error::malformed`] — поток кончился внутри блока или байты события не
/// UTF-8; [`Error::TooManyWarnings`] — предупреждений стало больше порога.
pub(crate) fn resolve_alternate_content(
    reader: &mut XmlReader<'_>,
    start: &BytesStart<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<AlternateContent> {
    // Ветку нельзя выбрать, не увидев всех `mc:Choice`: поддерживаемая может
    // стоять второй, а неподдержанный блок нужен целиком — как `Unknown`.
    let children = read_children(reader, part)?;
    resolve_block(start, &children, ctx, part, xml_path, 1)
}

/// Дочитать содержимое начатого элемента: события до его парного `End`, сам `End` не входит.
fn read_children(reader: &mut XmlReader<'_>, part: &str) -> Result<Vec<Event<'static>>> {
    let mut children = Vec::new();
    let mut depth: u32 = 0;
    loop {
        let event = reader
            .next_significant()?
            .ok_or_else(|| Error::malformed(part, "unexpected end of input"))?;
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(children),
            Event::End(_) => depth -= 1,
            _ => {}
        }
        children.push(event);
    }
}

/// Выбрать ветку по содержимому блока; `depth` — вложенность самого блока (1 у внешнего).
fn resolve_block(
    start: &BytesStart<'_>,
    children: &[Event<'static>],
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: u32,
) -> Result<AlternateContent> {
    if depth > MAX_ALTERNATE_DEPTH {
        ctx.warn_at(
            WarningKind::DeepNesting,
            part,
            Some(xml_path),
            format!(
                "`mc:AlternateContent` nested deeper than {MAX_ALTERNATE_DEPTH}; the block is kept as unknown"
            ),
        )?;
        return render_verbatim(start, children, part).map(AlternateContent::Unsupported);
    }

    let mut index = 0;
    let mut level: u32 = 0;
    while index < children.len() {
        match &children[index] {
            Event::Start(element) => {
                if level == 0 && local_name(element.name().as_ref()) == b"Choice" {
                    let end = matching_end(children, index)
                        .ok_or_else(|| Error::malformed(part, "unclosed `mc:Choice`"))?;
                    if requires_supported(element, part)? {
                        let inner = render_children(
                            &children[index + 1..end],
                            ctx,
                            part,
                            xml_path,
                            depth + 1,
                        )?;
                        return Ok(AlternateContent::Choice(inner));
                    }
                    // Ветка не наша: её содержимое не пригодится — ни для
                    // выбора, ни как `Unknown` (там нужен блок целиком).
                    index = end;
                } else {
                    level += 1;
                }
            }
            Event::Empty(element)
                if level == 0 && local_name(element.name().as_ref()) == b"Choice" =>
            {
                if requires_supported(element, part)? {
                    return Ok(AlternateContent::Choice(String::new()));
                }
            }
            Event::End(_) => level = level.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }

    render_verbatim(start, children, part).map(AlternateContent::Unsupported)
}

/// Собрать XML содержимого выбранной ветки, разрешая вложенные блоки (`depth`) рекурсивно.
fn render_children(
    events: &[Event<'static>],
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: u32,
) -> Result<String> {
    let mut out = String::new();
    let mut index = 0;
    while index < events.len() {
        if let Event::Start(element) = &events[index] {
            if local_name(element.name().as_ref()) == b"AlternateContent" {
                let end = matching_end(events, index)
                    .ok_or_else(|| Error::malformed(part, "unclosed `mc:AlternateContent`"))?;
                match resolve_block(element, &events[index + 1..end], ctx, part, xml_path, depth)? {
                    // Поддержанную вложенную ветку разворачиваем на месте: во
                    // внешнем фрагменте от неё не должно остаться обёртки.
                    AlternateContent::Choice(inner) => out.push_str(&inner),
                    AlternateContent::Unsupported(xml) => out.push_str(&xml),
                }
                index = end + 1;
                continue;
            }
        }
        push_event(&mut out, &events[index], part)?;
        index += 1;
    }
    Ok(out)
}

/// Проверить `mc:Choice/@Requires`: пустой список поддержан всегда (ADR-0014 §2).
///
/// Отсутствие атрибута — тоже пустой список: документ с таким `mc:Choice` крив,
/// но выбрать его безопаснее, чем потерять содержимое.
fn requires_supported(element: &BytesStart<'_>, part: &str) -> Result<bool> {
    let attrs = attributes(element, part)?;
    let Some(requires) = find(&attrs, "Requires") else {
        return Ok(true);
    };
    Ok(requires
        .split_whitespace()
        .all(|prefix| SUPPORTED_REQUIRES.contains(&prefix)))
}

/// Индекс `End`, закрывающего элемент, который начат событием `start`.
fn matching_end(events: &[Event<'static>], start: usize) -> Option<usize> {
    let mut depth: u32 = 0;
    for (offset, event) in events[start..].iter().enumerate() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Блок целиком, как он был в XML, — вместе с обёрткой `mc:AlternateContent`.
fn render_verbatim(
    start: &BytesStart<'_>,
    children: &[Event<'static>],
    part: &str,
) -> Result<String> {
    let mut out = String::new();
    push_tag(&mut out, start, false, part)?;
    for event in children {
        push_event(&mut out, event, part)?;
    }
    out.push_str("</");
    push_bytes(&mut out, &start.to_end(), part)?;
    out.push('>');
    Ok(out)
}

/// Обернуть фрагмент XML в синтетический корень, чтобы разобрать его обычным `XmlReader`.
///
/// Префиксы (`w:`, `wp:`) ридер не резолвит, поэтому обёртки достаточно: без
/// общего родителя фрагмент с несколькими корнями или с текстом не разобрался бы.
#[must_use]
pub(crate) fn wrap_fragment(fragment: &str) -> String {
    format!("<docx-fragment>{fragment}</docx-fragment>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_converter_core::Archive;

    const PART: &str = "word/document.xml";
    const PATH: &str = "w:document/w:body/mc:AlternateContent";

    /// Начать разбор фрагмента: ридер и первый значимый `Start`-элемент.
    ///
    /// Не-`Start` события до него пропускаются.
    fn parse_start(xml: &str) -> (XmlReader<'_>, BytesStart<'static>) {
        parse_first(XmlReader::new(xml.as_bytes(), PART))
    }

    /// То же, но текст берётся дословно — так парсеры читают `w:t`.
    fn parse_start_verbatim(xml: &str) -> (XmlReader<'_>, BytesStart<'static>) {
        parse_first(XmlReader::preserving(xml.as_bytes(), PART))
    }

    /// Первый `Start`-элемент потока вместе с его ридером.
    fn parse_first(mut reader: XmlReader<'_>) -> (XmlReader<'_>, BytesStart<'static>) {
        let mut skipped = 0;
        while let Some(event) = reader.next_significant().expect("XML разбирается") {
            if let Event::Start(element) = event {
                return (reader, element);
            }
            skipped += 1;
        }
        panic!("во фрагменте нет Start-элемента ({skipped} событий до конца)");
    }

    /// Первый `Start`/`Empty`-элемент фрагмента.
    ///
    /// События у ридера owned, поэтому элемент переживает ридер — и атрибуты
    /// можно читать там же, где это делают парсеры.
    fn first_element(xml: &str) -> BytesStart<'static> {
        let mut reader = XmlReader::new(xml.as_bytes(), PART);
        while let Some(event) = reader.next_significant().expect("XML разбирается") {
            if let Event::Start(element) | Event::Empty(element) = event {
                return element;
            }
        }
        panic!("во фрагменте нет элемента");
    }

    /// Разобрать первый `mc:AlternateContent` части и вернуть результат с видами предупреждений.
    fn resolve_first(part_xml: &[u8]) -> (AlternateContent, Vec<WarningKind>) {
        let mut reader = XmlReader::new(part_xml, PART);
        let mut ctx = ParseCtx::new();
        while let Some(event) = reader.next_significant().expect("часть разбирается")
        {
            if let Event::Start(element) = event {
                if local_name(element.name().as_ref()) == b"AlternateContent" {
                    let resolved =
                        resolve_alternate_content(&mut reader, &element, &mut ctx, PART, PATH)
                            .expect("блок разбирается");
                    return (resolved, ctx.warnings().iter().map(|w| w.kind).collect());
                }
            }
        }
        panic!("в части нет `mc:AlternateContent`");
    }

    /// Разобранный `mc:AlternateContent` из строки: у блока один корень, обёртка не нужна.
    fn resolve_str(xml: &str) -> (AlternateContent, Vec<WarningKind>) {
        resolve_first(xml.as_bytes())
    }

    /// XML выбранной ветки.
    fn choice(result: AlternateContent) -> String {
        match result {
            AlternateContent::Choice(xml) => xml,
            AlternateContent::Unsupported(xml) => panic!("ожидалась Choice, пришло `{xml}`"),
        }
    }

    /// XML сохранённого блока.
    fn unsupported(result: AlternateContent) -> String {
        match result {
            AlternateContent::Unsupported(xml) => xml,
            AlternateContent::Choice(xml) => panic!("ожидалась Unsupported, пришла `{xml}`"),
        }
    }

    /// Строковые имена атрибутов — для сравнения в тестах.
    fn names<'a>(attrs: &'a [Attr<'a>]) -> Vec<&'a str> {
        attrs
            .iter()
            .map(|attr| std::str::from_utf8(attr.name).expect("имя атрибута — UTF-8"))
            .collect()
    }

    // -- атрибуты ----------------------------------------------------------

    #[test]
    fn attributes_drop_the_prefix_and_resolve_entities() {
        let element =
            first_element(r#"<w:p r:id="rId7" w:val="Доходы &amp; расходы" foo="a&#65;b"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");

        assert_eq!(names(&attrs), ["id", "val", "foo"]);
        assert_eq!(find(&attrs, "id"), Some("rId7"));
        assert_eq!(find(&attrs, "val"), Some("Доходы & расходы"));
        assert_eq!(find(&attrs, "foo"), Some("aAb"));
        assert_eq!(find(&attrs, "w:val"), None, "префикса в имени уже нет");
    }

    #[test]
    fn a_broken_attribute_is_malformed() {
        for xml in [
            r#"<w:p w:val="&bogus;"/>"#,
            "<w:p w:val>",
            r#"<w:p w:val="a" w:val="b"/>"#,
        ] {
            let element = first_element(xml);
            let err = attributes(&element, PART).expect_err("атрибут не читается");
            assert!(matches!(err, Error::Malformed { .. }), "{xml}: {err}");
        }
    }

    #[test]
    fn local_name_strips_the_prefix() {
        assert_eq!(local_name(b"w:p"), b"p");
        assert_eq!(local_name(b"document"), b"document");
        assert_eq!(local_name(b"mc:AlternateContent"), b"AlternateContent");
        assert_eq!(local_name(b""), b"");
        // Как у quick-xml: отрезается всё до первого двоеточия — второй в имени
        // невалиден и остаётся как есть.
        assert_eq!(local_name(b"a:b:c"), b"b:c");
    }

    #[test]
    fn is_true_accepts_1_true_on() {
        for value in ["1", "true", "on"] {
            assert!(is_true(value), "{value} — истина");
        }
        for value in ["0", "false", "off", "", "TRUE", "yes"] {
            assert!(!is_true(value), "{value} — не истина");
        }
    }

    #[test]
    fn resolve_reference_expands_entities_and_char_refs() {
        let xml = "a &amp; b &lt; &gt; &#65; &#x41; &quot;";
        let mut reader = XmlReader::new(xml.as_bytes(), PART);
        let mut resolved = Vec::new();
        while let Some(event) = reader.next_significant().expect("XML разбирается") {
            if let Event::GeneralRef(reference) = event {
                resolved.push(resolve_reference(&reference, PART).expect("ссылка разворачивается"));
            }
        }
        assert_eq!(resolved, ["&", "<", ">", "A", "A", "\""]);
    }

    #[test]
    fn resolve_reference_rejects_an_unknown_entity() {
        let xml = "a &bogus; b";
        let mut reader = XmlReader::new(xml.as_bytes(), PART);
        let reference = loop {
            match reader.next_significant().expect("XML разбирается") {
                Some(Event::GeneralRef(reference)) => break reference,
                Some(_) => {}
                None => panic!("ссылки во фрагменте нет"),
            }
        };

        let err = resolve_reference(&reference, PART).expect_err("сущность неизвестна");
        assert!(matches!(err, Error::Malformed { .. }), "{err}");
    }

    // -- значения свойств --------------------------------------------------

    #[test]
    fn attr_toggle_reads_off_and_inherit() {
        let mut ctx = ParseCtx::new();
        for (xml, expected) in [
            ("<w:b/>", Toggle::On),
            (r#"<w:b w:val="1"/>"#, Toggle::On),
            (r#"<w:b w:val="true"/>"#, Toggle::On),
            (r#"<w:b w:val="0"/>"#, Toggle::Off),
            (r#"<w:b w:val="false"/>"#, Toggle::Off),
            (r#"<w:b w:val="off"/>"#, Toggle::Off),
            (r#"<w:b w:val="inherit"/>"#, Toggle::Inherit),
        ] {
            let element = first_element(xml);
            let toggle = attr_toggle(
                &attributes(&element, PART).expect("атрибуты читаются"),
                &mut ctx,
                PART,
                "w:b",
            )
            .expect("значение читается");
            assert_eq!(toggle, Some(expected), "{xml}");
        }
        assert!(
            ctx.warnings().is_empty(),
            "штатные значения не предупреждают"
        );
    }

    #[test]
    fn attr_toggle_warns_on_an_unrecognized_value() {
        let mut ctx = ParseCtx::new();
        let element = first_element(r#"<w:b w:val="bogus"/>"#);

        let toggle = attr_toggle(
            &attributes(&element, PART).expect("атрибуты читаются"),
            &mut ctx,
            PART,
            "w:b",
        )
        .expect("значение читается");

        assert_eq!(
            toggle,
            Some(Toggle::On),
            "наличие элемента свойство включает"
        );
        assert_eq!(ctx.warnings().len(), 1);
        let warning = &ctx.warnings()[0];
        assert_eq!(warning.kind, WarningKind::InvalidAttribute);
        assert!(warning.message.contains("w:b"), "{}", warning.message);
    }

    #[test]
    fn attr_i32_reads_bounds_and_ignores_garbage() {
        let mut ctx = ParseCtx::new();
        let element = first_element(r#"<w:ind w:left="-2147483648" w:right="2147483647"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");
        assert_eq!(
            attr_i32(&attrs, "left", &mut ctx, PART).expect("читается"),
            Some(i32::MIN)
        );
        assert_eq!(
            attr_i32(&attrs, "right", &mut ctx, PART).expect("читается"),
            Some(i32::MAX)
        );
        assert_eq!(
            attr_i32(&attrs, "missing", &mut ctx, PART).expect("читается"),
            None
        );
        assert!(ctx.warnings().is_empty());

        let element = first_element(r#"<w:ind w:left="2147483648" w:right="1.5"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");
        assert_eq!(
            attr_i32(&attrs, "left", &mut ctx, PART).expect("читается"),
            None
        );
        assert_eq!(
            attr_i32(&attrs, "right", &mut ctx, PART).expect("читается"),
            None
        );
        assert_eq!(ctx.warnings().len(), 2);
        assert!(ctx
            .warnings()
            .iter()
            .all(|w| w.kind == WarningKind::InvalidAttribute));
    }

    #[test]
    fn attr_u32_rejects_negative_values() {
        let mut ctx = ParseCtx::new();
        let element = first_element(r#"<w:pgSz w:w="11906" w:h="4294967295"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");
        assert_eq!(
            attr_u32(&attrs, "w", &mut ctx, PART).expect("читается"),
            Some(11_906)
        );
        assert_eq!(
            attr_u32(&attrs, "h", &mut ctx, PART).expect("читается"),
            Some(u32::MAX)
        );
        assert!(ctx.warnings().is_empty());

        let element = first_element(r#"<w:pgSz w:w="-1"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");
        assert_eq!(
            attr_u32(&attrs, "w", &mut ctx, PART).expect("читается"),
            None
        );
        assert_eq!(ctx.warnings().len(), 1);
    }

    #[test]
    fn attr_f32_reads_measurements_but_not_infinities() {
        let mut ctx = ParseCtx::new();
        let element = first_element(r#"<w:spacing w:line="12.5" w:before="0"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");
        assert_eq!(
            attr_f32(&attrs, "line", &mut ctx, PART).expect("читается"),
            Some(12.5)
        );
        assert_eq!(
            attr_f32(&attrs, "before", &mut ctx, PART).expect("читается"),
            Some(0.0)
        );
        assert_eq!(
            attr_f32(&attrs, "after", &mut ctx, PART).expect("читается"),
            None
        );
        assert!(ctx.warnings().is_empty());

        let element = first_element(r#"<w:spacing w:line="inf" w:before="nan" w:after="широко"/>"#);
        let attrs = attributes(&element, PART).expect("атрибуты читаются");
        for name in ["line", "before", "after"] {
            assert_eq!(
                attr_f32(&attrs, name, &mut ctx, PART).expect("читается"),
                None
            );
        }
        assert_eq!(
            ctx.warnings().len(),
            3,
            "каждое значение предупреждает один раз"
        );
    }

    // -- capture_element ---------------------------------------------------

    #[test]
    fn capture_element_keeps_the_subtree_verbatim() {
        let xml = r#"<w:p><w:r><w:t xml:space="preserve">Доходы &amp; расходы</w:t></w:r><w:br/><w:tbl><w:tr/></w:tbl></w:p>"#;
        // Дословность держится на ридере: `XmlReader::new` обрезал бы пробелы
        // вокруг сущности, и `w:t` потерял бы текст (ради этого и есть `preserving`).
        let (mut reader, start) = parse_start_verbatim(xml);
        let mut ctx = ParseCtx::new();

        let captured =
            capture_element(&mut reader, &start, &mut ctx, PART).expect("поддерево читается");

        assert_eq!(captured, xml);
        assert!(ctx.warnings().is_empty());
    }

    #[test]
    fn capture_element_stops_at_the_depth_limit() {
        use std::fmt::Write as _;

        let mut xml = String::from("<root>");
        for n in 1..=40 {
            let _ = write!(xml, "<x{n}>");
        }
        for n in (1..=40).rev() {
            let _ = write!(xml, "</x{n}>");
        }
        xml.push_str("</root>");
        let (mut reader, start) = parse_start(&xml);
        let mut ctx = ParseCtx::new();

        let captured =
            capture_element(&mut reader, &start, &mut ctx, PART).expect("поддерево читается");

        let kinds: Vec<WarningKind> = ctx.warnings().iter().map(|w| w.kind).collect();
        assert_eq!(kinds, [WarningKind::DeepNesting], "предупреждение одно");
        // `root` — глубина 1, значит последний сохранённый уровень — `x31`,
        // а `x32` уже не влезает и пропускается вместе с поддеревом.
        assert!(
            captured.contains("<x31>"),
            "предел не достигнут: {captured}"
        );
        assert!(
            !captured.contains("<x32>"),
            "глубже предела не пишем: {captured}"
        );
        assert_well_formed(&captured);
    }

    #[test]
    fn capture_element_reports_an_unclosed_element() {
        let (mut reader, start) = parse_start("<w:p><w:r>");
        let mut ctx = ParseCtx::new();

        let err =
            capture_element(&mut reader, &start, &mut ctx, PART).expect_err("парного End нет");

        assert!(matches!(err, Error::Malformed { .. }), "{err}");
    }

    /// Фрагмент — well-formed XML: теги сбалансированы и разбираются.
    fn assert_well_formed(fragment: &str) {
        let wrapped = wrap_fragment(fragment);
        assert_eq!(
            wrapped,
            format!("<docx-fragment>{fragment}</docx-fragment>"),
            "обёртка — синтетический корень"
        );
        let mut reader = XmlReader::new(wrapped.as_bytes(), PART);
        let mut depth: u32 = 0;
        while let Some(event) = reader.next_significant().expect("фрагмент разбирается")
        {
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth = depth.checked_sub(1).expect("лишний End"),
                _ => {}
            }
        }
        assert_eq!(depth, 0, "незакрытый тег во фрагменте: {fragment}");
    }

    // -- mc:AlternateContent -----------------------------------------------

    #[test]
    fn alternate_content_prefers_the_choice_over_the_fallback() {
        let (result, warnings) = resolve_str(
            r#"<mc:AlternateContent><mc:Choice Requires="wps"><w:r><w:t>a</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>b</w:t></w:r></mc:Fallback></mc:AlternateContent>"#,
        );

        assert_eq!(choice(result), "<w:r><w:t>a</w:t></w:r>");
        assert!(warnings.is_empty());
    }

    #[test]
    fn alternate_content_skips_unsupported_choices() {
        let (result, warnings) = resolve_str(
            r#"<mc:AlternateContent><mc:Choice Requires="v"><w:pict/></mc:Choice><mc:Choice Requires="wps"><w:r/></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent>"#,
        );
        assert_eq!(choice(result), "<w:r/>");
        assert!(warnings.is_empty());

        // Ни одной поддержанной ветки — блок сохраняется целиком.
        let (result, warnings) = resolve_str(
            r#"<mc:AlternateContent><mc:Choice Requires="v"><w:pict/></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent>"#,
        );
        assert!(unsupported(result).contains("<mc:Choice Requires=\"v\">"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn alternate_content_takes_an_empty_requires() {
        for xml in [
            r#"<mc:AlternateContent><mc:Choice Requires=""><w:r/></mc:Choice></mc:AlternateContent>"#,
            r"<mc:AlternateContent><mc:Choice><w:r/></mc:Choice></mc:AlternateContent>",
            r#"<mc:AlternateContent><mc:Choice Requires="  "/><mc:Fallback/></mc:AlternateContent>"#,
        ] {
            let (result, warnings) = resolve_str(xml);
            let xml_out = match result {
                AlternateContent::Choice(inner) => inner,
                AlternateContent::Unsupported(block) => panic!("{xml}: блок не выбран: {block}"),
            };
            assert!(
                xml_out != "<mc:Fallback/>",
                "{xml}: Fallback не разбирается"
            );
            assert!(warnings.is_empty(), "{xml}");
        }

        assert_eq!(
            choice(resolve_str(r#"<mc:AlternateContent><mc:Choice Requires=""/><mc:Fallback/></mc:AlternateContent>"#).0),
            ""
        );
    }

    #[test]
    fn alternate_content_needs_every_requested_prefix() {
        let (result, _) = resolve_str(
            r#"<mc:AlternateContent><mc:Choice Requires="wps wpg"><w:r/></mc:Choice><mc:Fallback/></mc:AlternateContent>"#,
        );
        assert_eq!(choice(result), "<w:r/>", "все префиксы поддержаны");

        for requires in ["wps v", "v wps", "wpg o", "w14x"] {
            let xml = format!(
                r#"<mc:AlternateContent><mc:Choice Requires="{requires}"><w:r/></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent>"#
            );
            let (result, _) = resolve_str(&xml);
            assert!(
                unsupported(result).contains("<w:pict/>"),
                "{requires}: ветка с неподдержанным префиксом выбрана"
            );
        }
    }

    #[test]
    fn alternate_content_resolves_nested_blocks() {
        let (result, warnings) = resolve_str(
            r#"<mc:AlternateContent><mc:Choice Requires="wps"><w:r><mc:AlternateContent><mc:Choice Requires="wpg"><wpg:wgp/></mc:Choice><mc:Fallback><w:t>вложенный fallback</w:t></mc:Fallback></mc:AlternateContent></w:r></mc:Choice><mc:Fallback><w:t>внешний fallback</w:t></mc:Fallback></mc:AlternateContent>"#,
        );

        let xml = choice(result);
        assert!(
            xml.contains("<wpg:wgp/>"),
            "вложенная ветка не развёрнута: {xml}"
        );
        assert!(!xml.contains("вложенный fallback"), "{xml}");
        assert!(!xml.contains("внешний fallback"), "{xml}");
        assert!(
            !xml.contains("AlternateContent"),
            "обёрток не осталось: {xml}"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn alternate_content_stops_at_the_depth_limit() {
        // 17 вложенных блоков: внешние разрешаются, семнадцатый — нет.
        let mut xml = r"<w:t>на дне</w:t>".to_owned();
        for _ in 0..17 {
            xml = format!(
                r#"<mc:AlternateContent><mc:Choice Requires="wps">{xml}</mc:Choice><mc:Fallback><w:t>вложенный fallback</w:t></mc:Fallback></mc:AlternateContent>"#
            );
        }

        let outer = format!("<w:body>{xml}</w:body>");
        let (result, warnings) = resolve_first(outer.as_bytes());

        assert_eq!(
            warnings,
            [WarningKind::DeepNesting],
            "предел — 16 вложенных блоков"
        );
        let xml = choice(result);
        assert!(xml.contains("на дне"), "внешние ветки развёрнуты: {xml}");
        assert!(
            xml.contains("вложенный fallback"),
            "глубокий блок сохранён целиком: {xml}"
        );
    }

    #[test]
    fn alternate_content_reports_an_unclosed_block() {
        let xml = r#"<mc:AlternateContent><mc:Choice Requires="wps"><w:r>"#;
        let mut reader = XmlReader::new(xml.as_bytes(), PART);
        let mut ctx = ParseCtx::new();
        let Some(Event::Start(element)) = reader.next_significant().expect("разбирается")
        else {
            panic!("ожидался Start");
        };

        let err = resolve_alternate_content(&mut reader, &element, &mut ctx, PART, PATH)
            .expect_err("блок не закрыт");

        assert!(matches!(err, Error::Malformed { .. }), "{err}");
    }

    // -- ридер -------------------------------------------------------------

    #[test]
    fn bom_and_doctype_are_the_readers_business() {
        let xml = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                   <!DOCTYPE w:document>\
                   <!-- комментарий -->\
                   <?mso-application progid=\"Word.Document\"?>\
                   <w:document w:foo=\"1\"><w:body/></w:document>";
        let (mut reader, start) = parse_start(xml);

        let attrs = attributes(&start, PART).expect("атрибуты читаются");
        assert_eq!(find(&attrs, "foo"), Some("1"));
        assert_eq!(local_name(start.name().as_ref()), b"document");
        assert!(matches!(
            reader.next_significant().expect("разбирается"),
            Some(Event::Empty(_))
        ));
    }

    #[test]
    fn wrap_fragment_makes_a_single_root() {
        let wrapped = wrap_fragment("<w:r/><w:r/>текст");

        let (mut reader, start) = parse_start(&wrapped);
        assert_eq!(start.name().as_ref(), b"docx-fragment");
        let mut events = 0;
        while let Some(event) = reader.next_significant().expect("разбирается") {
            if !matches!(event, Event::End(_)) {
                events += 1;
            }
        }
        assert_eq!(
            events, 3,
            "два элемента и текст — корень уже прочитан отдельно"
        );
    }

    // -- фикстуры ----------------------------------------------------------

    /// Ожидание фикстуры: какую ветку обязан выбрать разбор и что в неё не попало.
    #[derive(Debug)]
    enum Expected {
        Choice {
            present: &'static str,
            absent: &'static str,
        },
        Unsupported {
            present: &'static str,
        },
    }

    /// Часть `word/document.xml` фикстуры `alternate_content/…`.
    fn fixture_document(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx/alternate_content")
            .join(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("{} не читается: {e}", path.display()));
        let mut archive = Archive::new(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        archive
            .read("word/document.xml")
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Пять фикстур каталога: выбор ветки и отсутствие Fallback в результате.
    ///
    /// Предупреждений разбор блока не даёт: `UnknownElement` для
    /// `AlternateContent::Unsupported` добавит парсер, который положит блок в
    /// `BlockItem::Unknown` (так это и записано в сайдкаре `fallback_only.json`).
    #[test]
    fn fixtures_resolve_alternate_content() {
        for (name, expected) in [
            (
                "choice_fallback_textbox.docx",
                Expected::Choice {
                    present: "wps:txbx",
                    absent: "v:textbox",
                },
            ),
            (
                "choice_shape.docx",
                Expected::Choice {
                    present: "wps:wsp",
                    absent: "v:oval",
                },
            ),
            (
                "group_shape.docx",
                Expected::Choice {
                    present: "wpg:wgp",
                    absent: "v:group",
                },
            ),
            (
                "fallback_only.docx",
                Expected::Unsupported {
                    present: "v:textbox",
                },
            ),
            (
                "nested_alternate.docx",
                Expected::Choice {
                    present: "wpg:wgp",
                    absent: "внешний fallback",
                },
            ),
        ] {
            let document = fixture_document(name);
            let (result, warnings) = resolve_first(&document);

            assert!(warnings.is_empty(), "{name}: предупреждения: {warnings:?}");
            match (expected, &result) {
                (Expected::Choice { present, absent }, AlternateContent::Choice(xml)) => {
                    assert!(
                        xml.contains(present),
                        "{name}: в Choice нет `{present}`: {xml}"
                    );
                    assert!(!xml.contains(absent), "{name}: в Choice попал Fallback");
                }
                (Expected::Unsupported { present }, AlternateContent::Unsupported(xml)) => {
                    assert!(
                        xml.contains(present),
                        "{name}: в сохранённом блоке нет `{present}`: {xml}"
                    );
                }
                (expected, result) => panic!("{name}: ожидалось {expected:?}, получено {result:?}"),
            }
        }

        // У вложенной фикстуры внешний Choice поддержан, поэтому Fallback не
        // должен попасть в результат ни на одном уровне.
        let (result, _) = resolve_first(&fixture_document("nested_alternate.docx"));
        assert!(!choice(result).contains("fallback"));
    }
}
