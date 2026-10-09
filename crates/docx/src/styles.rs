//! Разбор `word/styles.xml` (слайс S8): таблица стилей и умолчания документа.
//!
//! Каскад здесь не разрешается (ADR-0013 §1): `basedOn`, `w:pPr` и `w:rPr`
//! складываются в модель как есть, а сводить их будет раскладка Спринта 9.
//! Поверх сложения парсер делает ровно одно — проверяет ссылки
//! ([`StyleTable::validate`]): циклы `basedOn` и ссылки на несуществующие стили
//! попадают в предупреждения уже здесь.
//!
//! Свойства абзаца и знака разбирают `parse_p_pr` и `parse_r_pr`: те же
//! элементы приходят и в теле документа, поэтому слайс S12 может переиспользовать
//! их вместо второго разбора `w:pPr`/`w:rPr` (слайс S7 пишет свой, чтобы
//! не блокироваться на этом файле).

// Парсер подключит сборка документа (слайс S12): до тех пор `dead_code`
// срабатывал бы на всём модуле; `allow` снимается вместе с подключением —
// как в `context.rs` и `xml.rs`.
#![allow(dead_code)]

use std::collections::BTreeMap;

use doc_converter_core::xml::XmlReader;
use doc_converter_core::WarningKind;
use quick_xml::events::{BytesStart, Event};

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::numbering::NumId;
use crate::model::raw::{
    Border, BorderStyle, CharacterSpacing, Color, FontHint, HalfPoint, Highlight, Ind,
    Justification, LineSpacing, LineSpacingRule, NumPr, ParagraphBorders, ParagraphSpacing, RFonts,
    RawPPr, RawRPr, RawTblPr, Shading, ShadingPattern, StyleId, TabLeader, TabStop, TabStopKind,
    Toggle, Twips, Underline, VertAlign,
};
use crate::model::style::{
    CharacterStyle, ConditionalFormat, DocDefaults, NumberingStyle, ParagraphStyle, StyleTable,
    TableStyle, TableStyleCondition,
};
use crate::model::table::{CellMargins, TableBorders, TableLook, TableWidth};
use crate::xml::{
    attr_i32, attr_toggle, attr_u32, attributes, capture_element, find, is_true, local_name, Attr,
};

/// Разобрать `word/styles.xml`.
///
/// # Errors
/// [`Error::malformed`] — битый XML или нет корня `w:styles`;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
pub(crate) fn parse(bytes: &[u8], ctx: &mut ParseCtx, part: &str) -> Result<StyleTable> {
    let mut reader = XmlReader::new(bytes, part);
    let mut styles = StyleTable::default();
    if has_styles_root(&mut reader, part)? {
        parse_styles(&mut reader, ctx, part, &mut styles)?;
    }
    // Проверки ссылок — часть разбора: цикл `basedOn` обязан попасть
    // в предупреждения уже здесь, а не когда таблицу тронет раскладка.
    styles.validate(part, ctx)?;
    Ok(styles)
}

/// Дойти до корневого `w:styles`; `false` — корень самозакрыт (`<w:styles/>`).
///
/// # Errors
/// [`Error::malformed`] — корень другой или его нет.
fn has_styles_root(reader: &mut XmlReader<'_>, part: &str) -> Result<bool> {
    while let Some(event) = reader.next_significant()? {
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                if local_name(element.name().as_ref()) != b"styles" {
                    return Err(Error::malformed(
                        part,
                        format!("unexpected root element `w:{}`", name_of(element)),
                    ));
                }
            }
            _ => continue,
        }
        return Ok(matches!(&event, Event::Start(_)));
    }
    Err(Error::malformed(part, "missing `w:styles` root"))
}

// ---------------------------------------------------------------------------
// Обход детей
// ---------------------------------------------------------------------------

/// Значимое событие среди детей разбираемого элемента.
enum Child {
    /// Начатый элемент: поддерево обязан вычитать обработчик.
    Start(BytesStart<'static>),
    /// Самозакрытый элемент: детей нет.
    Empty(BytesStart<'static>),
    /// Парный `End` родителя.
    End,
}

/// Следующий ребёнок разбираемого элемента.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше парного `End`.
fn next_child(reader: &mut XmlReader<'_>, part: &str, parent: &str) -> Result<Child> {
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                format!("unexpected end of input inside `w:{parent}`"),
            ));
        };
        match event {
            Event::Start(element) => return Ok(Child::Start(element)),
            Event::Empty(element) => return Ok(Child::Empty(element)),
            Event::End(_) => return Ok(Child::End),
            _ => {}
        }
    }
}

/// Дочитать элемент: у самозакрытого детей нет, у начатого — поддерево до `End`.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше парного `End`.
fn skip_child(reader: &mut XmlReader<'_>, empty: bool, part: &str) -> Result<()> {
    if empty {
        return Ok(());
    }
    let mut depth = 0u32;
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

/// Сохранить элемент целиком — так же, как это делает [`capture_element`] для поддеревьев.
///
/// # Errors
/// [`Error::malformed`] — байты события не UTF-8.
fn element_xml(element: &BytesStart<'_>, empty: bool, part: &str) -> Result<String> {
    let raw = std::str::from_utf8(element)
        .map_err(|e| Error::malformed(part, format!("event bytes are not UTF-8: {e}")))?;
    Ok(format!("<{raw}{}>", if empty { "/" } else { ">" }))
}

/// Сохранить элемент с поддеревом — для неподдержанных свойств (ADR-0014 §3).
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше парного `End`;
/// [`Error::TooManyWarnings`] — порог.
fn keep_element(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<String> {
    if empty {
        element_xml(element, true, part)
    } else {
        capture_element(reader, element, ctx, part)
    }
}

/// Локальное имя элемента строкой — для сообщений предупреждений.
fn name_of(element: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(local_name(element.name().as_ref())).into_owned()
}

// ---------------------------------------------------------------------------
// w:styles и w:style
// ---------------------------------------------------------------------------

/// Вид стиля (`w:style/@w:type`, `ST_StyleType`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

impl StyleKind {
    /// Вид по значению `w:type`; `None` — значение вне `ST_StyleType`.
    fn from_str(raw: &str) -> Option<Self> {
        match raw.trim() {
            "paragraph" => Some(Self::Paragraph),
            "character" => Some(Self::Character),
            "table" => Some(Self::Table),
            "numbering" => Some(Self::Numbering),
            _ => None,
        }
    }
}

/// Разобранные части `w:style` — до раскладки по видам стилей.
#[derive(Default)]
struct StyleParts {
    name: Option<String>,
    based_on: Option<StyleId>,
    next: Option<StyleId>,
    link: Option<StyleId>,
    is_default: bool,
    hidden: bool,
    custom: bool,
    aliases: Vec<String>,
    props: StyleProps,
    conditional: Vec<ConditionalFormat>,
}

/// Свойства, общие для стиля и условного формата: `w:pPr`, `w:rPr`, `w:tblPr`.
#[derive(Default)]
struct StyleProps {
    ppr: RawPPr,
    rpr: RawRPr,
    tbl_pr: RawTblPr,
}

impl StyleProps {
    /// Разобрать ребёнка-свойства; `false` — имя не из этого набора.
    ///
    /// # Errors
    /// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
    fn apply(
        &mut self,
        reader: &mut XmlReader<'_>,
        element: &BytesStart<'_>,
        empty: bool,
        ctx: &mut ParseCtx,
        part: &str,
    ) -> Result<bool> {
        match local_name(element.name().as_ref()) {
            b"pPr" => {
                self.ppr = if empty {
                    RawPPr::default()
                } else {
                    parse_p_pr(reader, ctx, part)?
                };
                Ok(true)
            }
            b"rPr" => {
                self.rpr = if empty {
                    RawRPr::default()
                } else {
                    parse_r_pr(reader, ctx, part)?
                };
                Ok(true)
            }
            b"tblPr" => {
                self.tbl_pr = if empty {
                    RawTblPr::default()
                } else {
                    parse_tbl_pr(reader, ctx, part)?
                };
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// Обойти детей `w:styles`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_styles(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    styles: &mut StyleTable,
) -> Result<()> {
    loop {
        let (element, empty) = match next_child(reader, part, "styles")? {
            Child::End => return Ok(()),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        match local_name(element.name().as_ref()) {
            b"docDefaults" => {
                styles.doc_defaults = if empty {
                    DocDefaults::default()
                } else {
                    parse_doc_defaults(reader, ctx, part)?
                };
            }
            b"style" => {
                if let Some((kind, id, parts)) = parse_style(reader, &element, empty, ctx, part)? {
                    insert_style_parts(styles, kind, id, parts, ctx, part)?;
                }
            }
            // `w:latentStyles` описывает стили, которых в документе нет, —
            // модель их не хранит.
            b"latentStyles" => skip_child(reader, empty, part)?,
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unrecognized element `w:{}` in `w:styles`",
                        name_of(&element)
                    ),
                )?;
                skip_child(reader, empty, part)?;
            }
        }
    }
}

/// Обойти детей `w:docDefaults`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_doc_defaults(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<DocDefaults> {
    let mut defaults = DocDefaults::default();
    loop {
        let (element, empty) = match next_child(reader, part, "docDefaults")? {
            Child::End => return Ok(defaults),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        match local_name(element.name().as_ref()) {
            // `empty` — самозакрытый `<w:rPrDefault/>`: детей у него нет.
            b"rPrDefault" | b"pPrDefault" if empty => {}
            b"rPrDefault" => {
                if let Some(r_pr) =
                    parse_default_child(reader, "rPrDefault", b"rPr", part, ctx, parse_r_pr)?
                {
                    defaults.r_pr = r_pr;
                }
            }
            b"pPrDefault" => {
                if let Some(p_pr) =
                    parse_default_child(reader, "pPrDefault", b"pPr", part, ctx, parse_p_pr)?
                {
                    defaults.p_pr = p_pr;
                }
            }
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unrecognized element `w:{}` in `w:docDefaults`",
                        name_of(&element)
                    ),
                )?;
                skip_child(reader, empty, part)?;
            }
        }
    }
}

/// Дойти до `w:rPr` внутри `w:rPrDefault` (или `w:pPr` внутри `w:pPrDefault`).
///
/// Поддерево найденного элемента не читается — это делает вызывающий. `None` —
/// в родителе нужного элемента нет; его `End` уже вычитан.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше парного `End`.
fn default_child(
    reader: &mut XmlReader<'_>,
    parent: &str,
    child: &[u8],
    part: &str,
) -> Result<Option<(BytesStart<'static>, bool)>> {
    loop {
        let (element, empty) = match next_child(reader, part, parent)? {
            Child::End => return Ok(None),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        if local_name(element.name().as_ref()) == child {
            return Ok(Some((element, empty)));
        }
        skip_child(reader, empty, part)?;
    }
}

/// Разобрать `w:rPrDefault` (или `w:pPrDefault`) целиком: найти в нём
/// `w:rPr`/`w:pPr`, отдать его `parse` и дочитать остаток родителя.
///
/// Дочитать обязательно: [`default_child`] останавливается внутри
/// `w:rPrDefault`, а оставшийся `End` иначе примет за свой [`parse_styles`] —
/// и стили после `w:docDefaults` пропадут.
///
/// `None` — свойств нет: родитель пуст, либо `w:rPr`/`w:pPr` самозакрыт.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_default_child<T>(
    reader: &mut XmlReader<'_>,
    parent: &str,
    child: &[u8],
    part: &str,
    ctx: &mut ParseCtx,
    parse: fn(&mut XmlReader<'_>, &mut ParseCtx, &str) -> Result<T>,
) -> Result<Option<T>> {
    let Some((_, empty)) = default_child(reader, parent, child, part)? else {
        return Ok(None);
    };
    // Разбор `w:rPr`/`w:pPr` останавливается на его `End`; у самозакрытого
    // детей нет — в обоих случаях разбирать больше нечего.
    let parsed = if empty {
        None
    } else {
        Some(parse(reader, ctx, part)?)
    };
    skip_child(reader, false, part)?;
    Ok(parsed)
}

/// Разобрать `w:style`: атрибуты и дети.
///
/// `None` — стиль пропущен: без вида или без `w:styleId` он в модели не адресуем.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_style(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<(StyleKind, StyleId, StyleParts)>> {
    let attrs = attributes(element, part)?;
    let Some(raw) = find(&attrs, "type") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("style `{}` has no `w:type`; skipped", style_label(&attrs)),
        )?;
        skip_child(reader, empty, part)?;
        return Ok(None);
    };
    let Some(kind) = StyleKind::from_str(raw) else {
        ctx.warn(
            WarningKind::UnknownElement,
            part,
            format!(
                "style `{}` has unknown `w:type` `{raw}`; skipped",
                style_label(&attrs)
            ),
        )?;
        skip_child(reader, empty, part)?;
        return Ok(None);
    };

    let Some(id) = find(&attrs, "styleId") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:style` without `w:styleId` is skipped",
        )?;
        skip_child(reader, empty, part)?;
        return Ok(None);
    };
    let id = StyleId::new(id);

    let mut parts = StyleParts {
        is_default: find(&attrs, "default").is_some_and(is_true),
        custom: find(&attrs, "customStyle").is_some_and(is_true),
        ..StyleParts::default()
    };
    if !empty {
        parse_style_children(reader, ctx, part, &mut parts)?;
    }
    Ok(Some((kind, id, parts)))
}

/// Обойти детей `w:style`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_style_children(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    parts: &mut StyleParts,
) -> Result<()> {
    loop {
        let (element, empty) = match next_child(reader, part, "style")? {
            Child::End => return Ok(()),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        if parts.props.apply(reader, &element, empty, ctx, part)? {
            continue;
        }
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"name" => parts.name = find(&attrs, "val").map(str::to_owned),
            b"aliases" => parts.aliases = split_aliases(find(&attrs, "val")),
            b"basedOn" => parts.based_on = find(&attrs, "val").map(StyleId::new),
            b"next" => parts.next = find(&attrs, "val").map(StyleId::new),
            b"link" => parts.link = find(&attrs, "val").map(StyleId::new),
            // `w:hidden` и `w:semiHidden` в модели — одно поле: оба прячут стиль
            // из списка Word.
            b"hidden" | b"semiHidden" => {
                let element_name = format!("w:{}", name_of(&element));
                if !matches!(
                    attr_toggle(&attrs, ctx, part, &element_name)?,
                    Some(Toggle::Off)
                ) {
                    parts.hidden = true;
                }
            }
            b"tblStylePr" => {
                parts
                    .conditional
                    .push(parse_conditional(reader, &element, empty, ctx, part)?);
                continue;
            }
            // Дети `w:style`, которых модель не хранит: они не влияют
            // ни на раскладку, ни на каскад.
            b"uiPriority" | b"qFormat" | b"unhideWhenUsed" | b"locked" | b"personal"
            | b"personalCompose" | b"personalReply" | b"autoRedefine" | b"rsid" | b"trPr"
            | b"tcPr" => {}
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unrecognized element `w:{}` in style `{}`",
                        name_of(&element),
                        parts.name.as_deref().unwrap_or("<unnamed>")
                    ),
                )?;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Разобрать `w:tblStylePr` — условный формат табличного стиля.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_conditional(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<ConditionalFormat> {
    let attrs = attributes(element, part)?;
    // Без `w:type` схема подразумевает всю таблицу целиком.
    let kind = find(&attrs, "type").map_or(TableStyleCondition::WholeTable, table_style_condition);

    let mut props = StyleProps::default();
    if !empty {
        loop {
            let (child, child_empty) = match next_child(reader, part, "tblStylePr")? {
                Child::End => break,
                Child::Empty(child) => (child, true),
                Child::Start(child) => (child, false),
            };
            if props.apply(reader, &child, child_empty, ctx, part)? {
                continue;
            }
            match local_name(child.name().as_ref()) {
                // Свойства строк и ячеек условного формата модель пока не хранит:
                // у `ConditionalFormat` для них нет полей.
                b"trPr" | b"tcPr" => {}
                _ => {
                    ctx.warn(
                        WarningKind::UnknownElement,
                        part,
                        format!(
                            "unrecognized element `w:{}` in `w:tblStylePr`",
                            name_of(&child)
                        ),
                    )?;
                }
            }
            skip_child(reader, child_empty, part)?;
        }
    }

    Ok(ConditionalFormat {
        kind,
        ppr: props.ppr,
        rpr: props.rpr,
        tbl_pr: props.tbl_pr,
    })
}

/// Разложить разобранные части `w:style` по таблице нужного вида.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn insert_style_parts(
    styles: &mut StyleTable,
    kind: StyleKind,
    id: StyleId,
    parts: StyleParts,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<()> {
    let StyleParts {
        name,
        based_on,
        next,
        link,
        is_default,
        hidden,
        custom,
        aliases,
        props,
        conditional,
    } = parts;
    let StyleProps { ppr, rpr, tbl_pr } = props;

    match kind {
        StyleKind::Paragraph => {
            note_default(&mut styles.defaults.paragraph, &id, is_default);
            let style = ParagraphStyle {
                id: id.clone(),
                name,
                based_on,
                next,
                link,
                is_default,
                hidden,
                custom,
                aliases,
                ppr,
                rpr,
            };
            insert_style(&mut styles.paragraph, id, style, ctx, part)
        }
        StyleKind::Character => {
            note_default(&mut styles.defaults.character, &id, is_default);
            let style = CharacterStyle {
                id: id.clone(),
                name,
                based_on,
                link,
                is_default,
                hidden,
                custom,
                aliases,
                rpr,
            };
            insert_style(&mut styles.character, id, style, ctx, part)
        }
        StyleKind::Table => {
            note_default(&mut styles.defaults.table, &id, is_default);
            let style = TableStyle {
                id: id.clone(),
                name,
                based_on,
                is_default,
                hidden,
                custom,
                aliases,
                ppr,
                rpr,
                tbl_pr,
                conditional,
            };
            insert_style(&mut styles.table, id, style, ctx, part)
        }
        StyleKind::Numbering => {
            note_default(&mut styles.defaults.numbering, &id, is_default);
            let style = NumberingStyle {
                id: id.clone(),
                name,
                based_on,
                is_default,
                hidden,
                custom,
                aliases,
                ppr,
                rpr,
            };
            insert_style(&mut styles.numbering, id, style, ctx, part)
        }
    }
}

/// Запомнить первый стиль вида, помеченный `w:default="1"`.
fn note_default(slot: &mut Option<StyleId>, id: &StyleId, is_default: bool) {
    if is_default && slot.is_none() {
        *slot = Some(id.clone());
    }
}

/// Вставить стиль, не затирая ранее разобранный дубликат `w:styleId`.
///
/// Побеждает первое объявление: какое из двух описаний верное, файл не говорит,
/// а «последний победил» сделал бы разбор зависимым от порядка (ADR-0016 §2).
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn insert_style<T>(
    map: &mut BTreeMap<StyleId, T>,
    id: StyleId,
    style: T,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<()> {
    if map.contains_key(&id) {
        ctx.warn(
            WarningKind::DuplicateStyleId,
            part,
            format!("style `{id}` is declared twice; the first declaration wins"),
        )?;
        return Ok(());
    }
    map.insert(id, style);
    Ok(())
}

/// Идентификатор стиля для сообщения — или пометка, что его нет.
fn style_label<'a>(attrs: &'a [Attr<'a>]) -> &'a str {
    find(attrs, "styleId").unwrap_or("<without id>")
}

/// Синонимы имени (`w:aliases w:val="a, b"`).
fn split_aliases(raw: Option<&str>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// w:pPr, w:rPr, w:tblPr
// ---------------------------------------------------------------------------

/// Разобрать `w:pPr` — те же свойства, что и в теле документа.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
pub(crate) fn parse_p_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<RawPPr> {
    let mut ppr = RawPPr::default();
    loop {
        let (element, empty) = match next_child(reader, part, "pPr")? {
            Child::End => return Ok(ppr),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"pStyle" => ppr.style = find(&attrs, "val").map(StyleId::new),
            b"numPr" => {
                if empty {
                    ppr.num_pr = Some(NumPr::default());
                } else {
                    ppr.num_pr = Some(parse_num_pr(reader, ctx, part)?);
                    continue;
                }
            }
            b"spacing" => ppr.spacing = Some(parse_paragraph_spacing(&attrs, ctx, part)?),
            b"ind" => ppr.ind = Some(parse_ind(&attrs, ctx, part)?),
            b"jc" => ppr.jc = find(&attrs, "val").map(justification),
            b"keepNext" => ppr.keep_next = attr_toggle(&attrs, ctx, part, "w:keepNext")?,
            b"keepLines" => ppr.keep_lines = attr_toggle(&attrs, ctx, part, "w:keepLines")?,
            b"pageBreakBefore" => {
                ppr.page_break_before = attr_toggle(&attrs, ctx, part, "w:pageBreakBefore")?;
            }
            b"widowControl" => {
                ppr.widow_control = attr_toggle(&attrs, ctx, part, "w:widowControl")?;
            }
            b"outlineLvl" => ppr.outline_lvl = parse_outline_lvl(&attrs, ctx, part)?,
            b"pBdr" => {
                if empty {
                    ppr.p_bdr = Some(ParagraphBorders::default());
                } else {
                    ppr.p_bdr = Some(parse_p_bdr(reader, ctx, part)?);
                    continue;
                }
            }
            b"shd" => ppr.shd = Some(parse_shading(&attrs, ctx, part)?),
            b"tabs" => {
                if empty {
                    ppr.tabs = Vec::new();
                } else {
                    ppr.tabs = parse_tabs(reader, ctx, part)?;
                    continue;
                }
            }
            b"rPr" => {
                if empty {
                    ppr.r_pr = Some(RawRPr::default());
                } else {
                    ppr.r_pr = Some(parse_r_pr(reader, ctx, part)?);
                    continue;
                }
            }
            // Свойства секции — не этот слайс: внутри стиля `w:sectPr` смысла
            // не имеет, но XML сохраняем, чтобы не потерять его молча.
            b"sectPr" => {
                ppr.unknown.push((
                    "sectPr".to_owned(),
                    keep_element(reader, &element, empty, ctx, part)?,
                ));
                continue;
            }
            _ => {
                let name = name_of(&element);
                ppr.unknown
                    .push((name, keep_element(reader, &element, empty, ctx, part)?));
                continue;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Разобрать `w:rPr` — те же свойства, что и в теле документа.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
pub(crate) fn parse_r_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<RawRPr> {
    let mut rpr = RawRPr::default();
    loop {
        let (element, empty) = match next_child(reader, part, "rPr")? {
            Child::End => return Ok(rpr),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"rStyle" => rpr.style = find(&attrs, "val").map(StyleId::new),
            b"rFonts" => rpr.r_fonts = Some(parse_r_fonts(&attrs)),
            b"b" => rpr.b = attr_toggle(&attrs, ctx, part, "w:b")?,
            b"i" => rpr.i = attr_toggle(&attrs, ctx, part, "w:i")?,
            b"caps" => rpr.caps = attr_toggle(&attrs, ctx, part, "w:caps")?,
            b"smallCaps" => rpr.small_caps = attr_toggle(&attrs, ctx, part, "w:smallCaps")?,
            b"strike" => rpr.strike = attr_toggle(&attrs, ctx, part, "w:strike")?,
            b"dstrike" => rpr.dstrike = attr_toggle(&attrs, ctx, part, "w:dstrike")?,
            b"vanish" => rpr.vanish = attr_toggle(&attrs, ctx, part, "w:vanish")?,
            b"outline" => rpr.outline = attr_toggle(&attrs, ctx, part, "w:outline")?,
            b"shadow" => rpr.shadow = attr_toggle(&attrs, ctx, part, "w:shadow")?,
            b"emboss" => rpr.emboss = attr_toggle(&attrs, ctx, part, "w:emboss")?,
            b"imprint" => rpr.imprint = attr_toggle(&attrs, ctx, part, "w:imprint")?,
            b"color" => {
                rpr.color = match find(&attrs, "val") {
                    Some(raw) => parse_color(raw, ctx, part)?,
                    None => None,
                };
            }
            b"sz" => rpr.sz = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            b"szCs" => rpr.sz_cs = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            b"highlight" => rpr.highlight = find(&attrs, "val").map(highlight),
            // `w:u` без `w:val` — одиночная линия: так велит умолчание схемы.
            b"u" => rpr.u = Some(find(&attrs, "val").map_or(Underline::Single, underline)),
            b"vertAlign" => {
                rpr.vert_align = match find(&attrs, "val") {
                    Some(raw) => parse_vert_align(raw, ctx, part)?,
                    None => None,
                };
            }
            b"spacing" => {
                rpr.spacing = Some(CharacterSpacing {
                    value: attr_i32(&attrs, "val", ctx, part)?.map(Twips::new),
                });
            }
            b"position" => rpr.position = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            _ => {
                let name = name_of(&element);
                rpr.unknown
                    .push((name, keep_element(reader, &element, empty, ctx, part)?));
                continue;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Разобрать `w:tblPr` — свойства табличного стиля.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
pub(crate) fn parse_tbl_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<RawTblPr> {
    let mut tbl_pr = RawTblPr::default();
    loop {
        let (element, empty) = match next_child(reader, part, "tblPr")? {
            Child::End => return Ok(tbl_pr),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"tblBorders" => {
                if empty {
                    tbl_pr.borders = TableBorders::default();
                } else {
                    tbl_pr.borders = parse_tbl_borders(reader, ctx, part)?;
                    continue;
                }
            }
            b"tblCellMar" => {
                if empty {
                    tbl_pr.cell_margins = CellMargins::default();
                } else {
                    tbl_pr.cell_margins = parse_cell_margins(reader, ctx, part)?;
                    continue;
                }
            }
            b"tblW" => tbl_pr.width = parse_table_width(&attrs, ctx, part)?,
            b"tblLook" => tbl_pr.look = Some(parse_tbl_look(&attrs)),
            b"jc" => tbl_pr.jc = find(&attrs, "val").map(justification),
            b"tblInd" => tbl_pr.indent = attr_i32(&attrs, "w", ctx, part)?.map(Twips::new),
            _ => {
                let name = name_of(&element);
                tbl_pr
                    .unknown
                    .push((name, keep_element(reader, &element, empty, ctx, part)?));
                continue;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Разобрать `w:numPr` внутри `w:pPr` стиля.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_num_pr(reader: &mut XmlReader<'_>, ctx: &mut ParseCtx, part: &str) -> Result<NumPr> {
    let mut num_pr = NumPr::default();
    loop {
        let (element, empty) = match next_child(reader, part, "numPr")? {
            Child::End => return Ok(num_pr),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"ilvl" => num_pr.ilvl = parse_ilvl(&attrs, ctx, part)?,
            b"numId" => num_pr.num_id = attr_u32(&attrs, "val", ctx, part)?.map(NumId::new),
            _ => {
                // У `NumPr` нет поля для неподдержанного: сообщаем и идём дальше.
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unrecognized element `w:{}` in `w:numPr`",
                        name_of(&element)
                    ),
                )?;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Обойти детей `w:tabs`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_tabs(reader: &mut XmlReader<'_>, ctx: &mut ParseCtx, part: &str) -> Result<Vec<TabStop>> {
    let mut tabs = Vec::new();
    loop {
        let (element, empty) = match next_child(reader, part, "tabs")? {
            Child::End => return Ok(tabs),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        if local_name(element.name().as_ref()) == b"tab" {
            tabs.push(TabStop {
                val: Twips::new(attr_i32(&attrs, "pos", ctx, part)?.unwrap_or(0)),
                kind: find(&attrs, "val").map_or(TabStopKind::Left, tab_stop_kind),
                leader: find(&attrs, "leader").map_or(TabLeader::None, tab_leader),
            });
        }
        skip_child(reader, empty, part)?;
    }
}

/// Обойти детей `w:pBdr`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_p_bdr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<ParagraphBorders> {
    let mut borders = ParagraphBorders::default();
    loop {
        let (element, empty) = match next_child(reader, part, "pBdr")? {
            Child::End => return Ok(borders),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"top" => borders.top = Some(parse_border(&attrs, ctx, part)?),
            // `w:start`/`w:end` — то же, что `w:left`/`w:right`, но записано
            // по логическому краю строки.
            b"left" | b"start" => borders.left = Some(parse_border(&attrs, ctx, part)?),
            b"bottom" => borders.bottom = Some(parse_border(&attrs, ctx, part)?),
            b"right" | b"end" => borders.right = Some(parse_border(&attrs, ctx, part)?),
            b"between" => borders.between = Some(parse_border(&attrs, ctx, part)?),
            b"bar" => borders.bar = Some(parse_border(&attrs, ctx, part)?),
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!("unrecognized element `w:{}` in `w:pBdr`", name_of(&element)),
                )?;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Обойти детей `w:tblBorders`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_tbl_borders(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<TableBorders> {
    let mut borders = TableBorders::default();
    loop {
        let (element, empty) = match next_child(reader, part, "tblBorders")? {
            Child::End => return Ok(borders),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"top" => borders.top = Some(parse_border(&attrs, ctx, part)?),
            b"left" | b"start" => borders.left = Some(parse_border(&attrs, ctx, part)?),
            b"bottom" => borders.bottom = Some(parse_border(&attrs, ctx, part)?),
            b"right" | b"end" => borders.right = Some(parse_border(&attrs, ctx, part)?),
            b"insideH" => borders.inside_h = Some(parse_border(&attrs, ctx, part)?),
            b"insideV" => borders.inside_v = Some(parse_border(&attrs, ctx, part)?),
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unrecognized element `w:{}` in `w:tblBorders`",
                        name_of(&element)
                    ),
                )?;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Обойти детей `w:tblCellMar`.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_cell_margins(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<CellMargins> {
    let mut margins = CellMargins::default();
    loop {
        let (element, empty) = match next_child(reader, part, "tblCellMar")? {
            Child::End => return Ok(margins),
            Child::Empty(element) => (element, true),
            Child::Start(element) => (element, false),
        };
        let attrs = attributes(&element, part)?;
        match local_name(element.name().as_ref()) {
            b"top" => margins.top = parse_margin(&attrs, ctx, part)?,
            b"left" | b"start" => margins.left = parse_margin(&attrs, ctx, part)?,
            b"bottom" => margins.bottom = parse_margin(&attrs, ctx, part)?,
            b"right" | b"end" => margins.right = parse_margin(&attrs, ctx, part)?,
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unrecognized element `w:{}` in `w:tblCellMar`",
                        name_of(&element)
                    ),
                )?;
            }
        }
        skip_child(reader, empty, part)?;
    }
}

/// Поле ячейки (`w:top` внутри `w:tblCellMar`): `w:type="nil"` — поля нет.
///
/// # Errors
/// [`Error::malformed`] — битый XML; [`Error::TooManyWarnings`] — порог.
fn parse_margin(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<Twips>> {
    if find(attrs, "type").is_some_and(|raw| raw.trim() == "nil") {
        return Ok(None);
    }
    Ok(attr_i32(attrs, "w", ctx, part)?.map(Twips::new))
}

// ---------------------------------------------------------------------------
// Значения свойств
// ---------------------------------------------------------------------------

/// Первое из значений (`w:left`/`w:start`): Word пишет то одно, то другое.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог предупреждений о мусорных значениях.
fn attr_first(
    attrs: &[Attr<'_>],
    names: [&str; 2],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<i32>> {
    for name in names {
        if let Some(value) = attr_i32(attrs, name, ctx, part)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

/// `w:ind` — отступы абзаца.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_ind(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Ind> {
    Ok(Ind {
        left: attr_first(attrs, ["left", "start"], ctx, part)?.map(Twips::new),
        right: attr_first(attrs, ["right", "end"], ctx, part)?.map(Twips::new),
        first_line: attr_i32(attrs, "firstLine", ctx, part)?.map(Twips::new),
        hanging: attr_i32(attrs, "hanging", ctx, part)?.map(Twips::new),
    })
}

/// `w:spacing` внутри `w:pPr` — интервалы абзаца.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_paragraph_spacing(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<ParagraphSpacing> {
    Ok(ParagraphSpacing {
        before: attr_i32(attrs, "before", ctx, part)?.map(Twips::new),
        after: attr_i32(attrs, "after", ctx, part)?.map(Twips::new),
        line: attr_i32(attrs, "line", ctx, part)?.map(LineSpacing::new),
        line_rule: find(attrs, "lineRule").map(line_spacing_rule),
        before_lines: attr_u32(attrs, "beforeLines", ctx, part)?,
        after_lines: attr_u32(attrs, "afterLines", ctx, part)?,
        before_autospacing: find(attrs, "beforeAutospacing").is_some_and(is_true),
        after_autospacing: find(attrs, "afterAutospacing").is_some_and(is_true),
    })
}

/// `w:outlineLvl` — уровень структуры 0..=9.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_outline_lvl(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<u8>> {
    let Some(raw) = find(attrs, "val") else {
        return Ok(None);
    };
    if let Ok(level) = raw.trim().parse::<u8>() {
        if level <= 9 {
            return Ok(Some(level));
        }
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`w:outlineLvl`: `{raw}` is not a level 0..=9, ignored"),
    )?;
    Ok(None)
}

/// `w:ilvl` — уровень списка 0..=8.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_ilvl(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<u8>> {
    let Some(raw) = find(attrs, "val") else {
        return Ok(None);
    };
    if let Ok(level) = raw.trim().parse::<u8>() {
        if level <= 8 {
            return Ok(Some(level));
        }
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`w:ilvl`: `{raw}` is not a list level 0..=8, ignored"),
    )?;
    Ok(None)
}

/// `w:rFonts` — шрифты знака.
fn parse_r_fonts(attrs: &[Attr<'_>]) -> RFonts {
    RFonts {
        ascii: find(attrs, "ascii").map(str::to_owned),
        h_ansi: find(attrs, "hAnsi").map(str::to_owned),
        east_asia: find(attrs, "eastAsia").map(str::to_owned),
        cs: find(attrs, "cs").map(str::to_owned),
        hint: find(attrs, "hint").map(font_hint),
    }
}

/// `w:color` — `auto`, `none` или `RRGGBB`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_color(raw: &str, ctx: &mut ParseCtx, part: &str) -> Result<Option<Color>> {
    let value = raw.trim();
    match value.to_ascii_lowercase().as_str() {
        "auto" => return Ok(Some(Color::Auto)),
        "none" => return Ok(Some(Color::None)),
        _ => {}
    }
    if value.len() == 6 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(u32::from_str_radix(value, 16).ok().map(Color::Rgb));
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`w:color`: `{raw}` is not an `RRGGBB` value, ignored"),
    )?;
    Ok(None)
}

/// `w:shd` — заливка.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_shading(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Shading> {
    let color = match find(attrs, "color") {
        Some(raw) => parse_color(raw, ctx, part)?,
        None => None,
    };
    let fill = match find(attrs, "fill") {
        Some(raw) => parse_color(raw, ctx, part)?,
        None => None,
    };
    Ok(Shading {
        val: find(attrs, "val").map_or(ShadingPattern::Clear, shading_pattern),
        color,
        fill,
    })
}

/// `w:top`, `w:left`, … внутри `w:pBdr`/`w:tblBorders`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_border(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Border> {
    let color = match find(attrs, "color") {
        Some(raw) => parse_color(raw, ctx, part)?,
        None => None,
    };
    Ok(Border {
        // Умолчание `w:val` по схеме — одиночная линия.
        val: find(attrs, "val").map_or(BorderStyle::Single, border_style),
        sz: attr_u32(attrs, "sz", ctx, part)?,
        space: attr_u32(attrs, "space", ctx, part)?,
        color,
    })
}

/// `w:tblW` — ширина таблицы; `w:type` по умолчанию `dxa`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_table_width(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<TableWidth>> {
    let Some(width) = attr_i32(attrs, "w", ctx, part)? else {
        return Ok(None);
    };
    Ok(Some(match find(attrs, "type").map_or("dxa", str::trim) {
        "auto" | "nil" => TableWidth::Auto,
        // `pct` записан в пятидесятых долях процента, а модель хранит проценты.
        "pct" => TableWidth::Pct(percent(width)),
        _ => TableWidth::Dxa(Twips::new(width)),
    }))
}

/// Пятидесятые доли процента — в проценты.
///
/// Значение приходит из файла, и `f32` для него заведомо достаточно: больше
/// двух значащих цифр в `w:w` у `pct` не бывает.
#[allow(clippy::cast_precision_loss)]
fn percent(value: i32) -> f32 {
    value as f32 / 50.0
}

/// `w:tblLook`: явные атрибуты старше устаревшей маски `w:val` (ECMA-376 §17.4.56).
fn parse_tbl_look(attrs: &[Attr<'_>]) -> TableLook {
    let mut look = match find(attrs, "val").and_then(|raw| u32::from_str_radix(raw.trim(), 16).ok())
    {
        Some(bits) => TableLook {
            first_row: bits & 0x0020 != 0,
            last_row: bits & 0x0040 != 0,
            first_column: bits & 0x0080 != 0,
            last_column: bits & 0x0100 != 0,
            no_h_band: bits & 0x0200 != 0,
            no_v_band: bits & 0x0400 != 0,
        },
        None => TableLook::default(),
    };
    for (name, field) in [
        ("firstRow", &mut look.first_row),
        ("lastRow", &mut look.last_row),
        ("firstColumn", &mut look.first_column),
        ("lastColumn", &mut look.last_column),
        ("noHBand", &mut look.no_h_band),
        ("noVBand", &mut look.no_v_band),
    ] {
        if let Some(raw) = find(attrs, name) {
            *field = is_true(raw);
        }
    }
    look
}

/// `w:vertAlign`; у `ST_VerticalAlignRun` нет `Other` — незнакомое значение теряется.
///
/// # Errors
/// [`Error::TooManyWarnings`] — порог.
fn parse_vert_align(raw: &str, ctx: &mut ParseCtx, part: &str) -> Result<Option<VertAlign>> {
    match raw.trim() {
        "baseline" => Ok(Some(VertAlign::Baseline)),
        "superscript" => Ok(Some(VertAlign::Superscript)),
        "subscript" => Ok(Some(VertAlign::Subscript)),
        other => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:vertAlign`: unknown value `{other}`, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Сопоставить XML-значение варианту перечисления.
///
/// Незнакомое значение уходит в `Other`: модель хранит свойства сырыми,
/// и терять его нельзя — предупреждать тоже не о чем, `Other` для этого и заведён.
macro_rules! xml_variant {
    ($raw:expr, $other:path, { $($pattern:pat => $variant:expr),+ $(,)? }) => {
        match $raw {
            $($pattern => $variant,)+
            other => $other(other.to_owned()),
        }
    };
}

/// `w:jc` (`ST_Jc`).
fn justification(raw: &str) -> Justification {
    xml_variant!(raw, Justification::Other, {
        "left" => Justification::Left,
        "center" => Justification::Center,
        "right" => Justification::Right,
        "both" => Justification::Both,
        "distribute" => Justification::Distribute,
        "start" => Justification::Start,
        "end" => Justification::End,
    })
}

/// `w:val` границы (`ST_Border`).
fn border_style(raw: &str) -> BorderStyle {
    xml_variant!(raw, BorderStyle::Other, {
        "nil" => BorderStyle::Nil,
        "none" => BorderStyle::None,
        "single" => BorderStyle::Single,
        "thick" => BorderStyle::Thick,
        "double" => BorderStyle::Double,
        "dotted" => BorderStyle::Dotted,
        "dashed" => BorderStyle::Dashed,
        "dotDash" => BorderStyle::DotDash,
        "dotDotDash" => BorderStyle::DotDotDash,
        "triple" => BorderStyle::Triple,
        "thinThickSmallGap" => BorderStyle::ThinThickSmallGap,
        "thickThinSmallGap" => BorderStyle::ThickThinSmallGap,
        "thinThickThinSmallGap" => BorderStyle::ThinThickThinSmallGap,
        "thinThickMediumGap" => BorderStyle::ThinThickMediumGap,
        "thickThinMediumGap" => BorderStyle::ThickThinMediumGap,
        "thinThickThinMediumGap" => BorderStyle::ThinThickThinMediumGap,
        "thinThickLargeGap" => BorderStyle::ThinThickLargeGap,
        "thickThinLargeGap" => BorderStyle::ThickThinLargeGap,
        "thinThickThinLargeGap" => BorderStyle::ThinThickThinLargeGap,
        "wave" => BorderStyle::Wave,
        "doubleWave" => BorderStyle::DoubleWave,
        "dashSmallGap" => BorderStyle::DashSmallGap,
        "dashDotStroked" => BorderStyle::DashDotStroked,
        "threeDEmboss" => BorderStyle::ThreeDEmboss,
        "threeDEngrave" => BorderStyle::ThreeDEngrave,
        "outset" => BorderStyle::Outset,
        "inset" => BorderStyle::Inset,
    })
}

/// `w:shd/@w:val` (`ST_Shd`).
fn shading_pattern(raw: &str) -> ShadingPattern {
    xml_variant!(raw, ShadingPattern::Other, {
        "nil" => ShadingPattern::Nil,
        "clear" => ShadingPattern::Clear,
        "solid" => ShadingPattern::Solid,
        "horzStripe" => ShadingPattern::HorzStripe,
        "vertStripe" => ShadingPattern::VertStripe,
        "reverseDiagStripe" => ShadingPattern::ReverseDiagStripe,
        "diagStripe" => ShadingPattern::DiagStripe,
        "horzCross" => ShadingPattern::HorzCross,
        "diagCross" => ShadingPattern::DiagCross,
        "thinHorzStripe" => ShadingPattern::ThinHorzStripe,
        "thinVertStripe" => ShadingPattern::ThinVertStripe,
        "thinReverseDiagStripe" => ShadingPattern::ThinReverseDiagStripe,
        "thinDiagStripe" => ShadingPattern::ThinDiagStripe,
        "thinHorzCross" => ShadingPattern::ThinHorzCross,
        "thinDiagCross" => ShadingPattern::ThinDiagCross,
        "pct5" => ShadingPattern::Pct5,
        "pct10" => ShadingPattern::Pct10,
        "pct12" => ShadingPattern::Pct12,
        "pct15" => ShadingPattern::Pct15,
        "pct20" => ShadingPattern::Pct20,
        "pct25" => ShadingPattern::Pct25,
        "pct30" => ShadingPattern::Pct30,
        "pct35" => ShadingPattern::Pct35,
        "pct37" => ShadingPattern::Pct37,
        "pct40" => ShadingPattern::Pct40,
        "pct45" => ShadingPattern::Pct45,
        "pct50" => ShadingPattern::Pct50,
        "pct55" => ShadingPattern::Pct55,
        "pct60" => ShadingPattern::Pct60,
        "pct62" => ShadingPattern::Pct62,
        "pct65" => ShadingPattern::Pct65,
        "pct70" => ShadingPattern::Pct70,
        "pct75" => ShadingPattern::Pct75,
        "pct80" => ShadingPattern::Pct80,
        "pct85" => ShadingPattern::Pct85,
        "pct87" => ShadingPattern::Pct87,
        "pct90" => ShadingPattern::Pct90,
        "pct95" => ShadingPattern::Pct95,
    })
}

/// `w:u/@w:val` (`ST_Underline`).
fn underline(raw: &str) -> Underline {
    xml_variant!(raw, Underline::Other, {
        "single" => Underline::Single,
        "words" => Underline::Words,
        "double" => Underline::Double,
        "thick" => Underline::Thick,
        "dotted" => Underline::Dotted,
        "dottedHeavy" => Underline::DottedHeavy,
        "dash" => Underline::Dash,
        "dashedHeavy" => Underline::DashedHeavy,
        "dashLong" => Underline::DashLong,
        "dashLongHeavy" => Underline::DashLongHeavy,
        "dotDash" => Underline::DotDash,
        "dashDotHeavy" => Underline::DashDotHeavy,
        "dotDotDash" => Underline::DotDotDash,
        "dashDotDotHeavy" => Underline::DashDotDotHeavy,
        "wave" => Underline::Wave,
        "wavyHeavy" => Underline::WavyHeavy,
        "wavyDouble" => Underline::WavyDouble,
        "none" => Underline::None,
    })
}

/// `w:highlight/@w:val` (`ST_HighlightColor`).
fn highlight(raw: &str) -> Highlight {
    xml_variant!(raw, Highlight::Other, {
        "black" => Highlight::Black,
        "blue" => Highlight::Blue,
        "cyan" => Highlight::Cyan,
        "green" => Highlight::Green,
        "magenta" => Highlight::Magenta,
        "red" => Highlight::Red,
        "yellow" => Highlight::Yellow,
        "white" => Highlight::White,
        "darkBlue" => Highlight::DarkBlue,
        "darkCyan" => Highlight::DarkCyan,
        "darkGreen" => Highlight::DarkGreen,
        "darkMagenta" => Highlight::DarkMagenta,
        "darkRed" => Highlight::DarkRed,
        "darkYellow" => Highlight::DarkYellow,
        "darkGray" | "darkGrey" => Highlight::DarkGray,
        "lightGray" | "lightGrey" => Highlight::LightGray,
        "none" => Highlight::None,
    })
}

/// `w:hint` внутри `w:rFonts` (`ST_Hint`).
fn font_hint(raw: &str) -> FontHint {
    xml_variant!(raw, FontHint::Other, {
        "default" => FontHint::Default,
        "eastAsia" => FontHint::EastAsia,
        "cs" => FontHint::Cs,
    })
}

/// `w:lineRule` (`ST_LineSpacingRule`).
fn line_spacing_rule(raw: &str) -> LineSpacingRule {
    xml_variant!(raw, LineSpacingRule::Other, {
        "auto" => LineSpacingRule::Auto,
        "exact" => LineSpacingRule::Exact,
        "atLeast" => LineSpacingRule::AtLeast,
    })
}

/// `w:tab/@w:val` (`ST_TabJc`).
fn tab_stop_kind(raw: &str) -> TabStopKind {
    xml_variant!(raw, TabStopKind::Other, {
        "bar" => TabStopKind::Bar,
        "center" => TabStopKind::Center,
        "clear" => TabStopKind::Clear,
        "decimal" => TabStopKind::Decimal,
        "end" => TabStopKind::End,
        "num" => TabStopKind::Num,
        "start" => TabStopKind::Start,
        "left" => TabStopKind::Left,
        "right" => TabStopKind::Right,
    })
}

/// `w:tab/@w:leader` (`ST_TabTlc`).
fn tab_leader(raw: &str) -> TabLeader {
    xml_variant!(raw, TabLeader::Other, {
        "none" => TabLeader::None,
        "dot" => TabLeader::Dot,
        "hyphen" => TabLeader::Hyphen,
        "middleDot" => TabLeader::MiddleDot,
        "heavy" => TabLeader::Heavy,
        "underscore" => TabLeader::Underscore,
    })
}

/// `w:tblStylePr/@w:type` (`ST_TblStyleOverrideType`).
fn table_style_condition(raw: &str) -> TableStyleCondition {
    xml_variant!(raw, TableStyleCondition::Other, {
        "wholeTable" => TableStyleCondition::WholeTable,
        "firstRow" => TableStyleCondition::FirstRow,
        "lastRow" => TableStyleCondition::LastRow,
        "firstCol" => TableStyleCondition::FirstCol,
        "lastCol" => TableStyleCondition::LastCol,
        "band1Vert" => TableStyleCondition::Band1Vert,
        "band2Vert" => TableStyleCondition::Band2Vert,
        "band1Horz" => TableStyleCondition::Band1Horz,
        "band2Horz" => TableStyleCondition::Band2Horz,
        "neCell" => TableStyleCondition::NeCell,
        "nwCell" => TableStyleCondition::NwCell,
        "seCell" => TableStyleCondition::SeCell,
        "swCell" => TableStyleCondition::SwCell,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use doc_converter_core::Archive;

    use super::*;

    /// Обернуть фрагмент в корень `w:styles`.
    fn styles_xml(body: &str) -> String {
        format!(
            "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{body}</w:styles>"
        )
    }

    /// Разобрать фрагмент и вернуть таблицу вместе с видами предупреждений.
    fn parse_xml(body: &str) -> (StyleTable, Vec<WarningKind>) {
        let xml = styles_xml(body);
        let mut ctx = ParseCtx::new();
        let table =
            parse(xml.as_bytes(), &mut ctx, "word/styles.xml").expect("the fragment parses");
        let kinds = ctx
            .warnings()
            .iter()
            .map(|warning| warning.kind)
            .collect::<Vec<_>>();
        (table, kinds)
    }

    #[test]
    fn doc_defaults_and_paragraph_properties_are_parsed() {
        let (table, warnings) = parse_xml(
            r#"<w:docDefaults>
                 <w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/><w:b/></w:rPr></w:rPrDefault>
                 <w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="160" w:line="259" w:lineRule="auto"/><w:jc w:val="both"/></w:pPr></w:pPrDefault>
               </w:docDefaults>
               <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let fonts = table
            .doc_defaults
            .r_pr
            .r_fonts
            .as_ref()
            .expect("the default fonts are parsed");
        assert_eq!(fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(table.doc_defaults.r_pr.sz, Some(HalfPoint::new(22)));
        assert_eq!(table.doc_defaults.r_pr.b, Some(Toggle::On));

        let spacing = table
            .doc_defaults
            .p_pr
            .spacing
            .as_ref()
            .expect("the default spacing is parsed");
        assert_eq!(spacing.after, Some(Twips::new(160)));
        assert_eq!(spacing.line, Some(LineSpacing::new(259)));
        assert_eq!(spacing.line_rule, Some(LineSpacingRule::Auto));
        assert_eq!(table.doc_defaults.p_pr.jc, Some(Justification::Both));

        let normal = table
            .paragraph
            .get(&StyleId::new("Normal"))
            .expect("the default paragraph style is parsed");
        assert!(normal.is_default);
        assert_eq!(table.defaults.paragraph, Some(StyleId::new("Normal")));
    }

    #[test]
    fn all_four_style_kinds_are_split_apart() {
        // Основы `Heading1` и `Heading1Char` объявлены: `basedOn` на стиль,
        // которого в части нет, — это `MissingStyleRef` (ADR-0013 §3), а здесь
        // проверяется раскладка по четырём видам, а не проверка ссылок.
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
               <w:style w:type="character" w:styleId="DefaultParagraphFont"/>
               <w:style w:type="paragraph" w:styleId="Heading1">
                 <w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:link w:val="Heading1Char"/>
                 <w:pPr><w:keepNext/><w:numPr><w:ilvl w:val="1"/><w:numId w:val="5"/></w:numPr></w:pPr>
                 <w:rPr><w:b/><w:color w:val="2F5496"/></w:rPr>
               </w:style>
               <w:style w:type="character" w:customStyle="1" w:styleId="Heading1Char">
                 <w:name w:val="Heading 1 Char"/><w:basedOn w:val="DefaultParagraphFont"/><w:link w:val="Heading1"/><w:rPr><w:i/></w:rPr>
               </w:style>
               <w:style w:type="table" w:styleId="TableNormal"><w:name w:val="Normal Table"/></w:style>
               <w:style w:type="numbering" w:styleId="NoList"><w:name w:val="No List"/></w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let heading = table
            .paragraph
            .get(&StyleId::new("Heading1"))
            .expect("the paragraph style is parsed");
        assert_eq!(heading.based_on, Some(StyleId::new("Normal")));
        assert_eq!(heading.next, Some(StyleId::new("Normal")));
        assert_eq!(heading.link, Some(StyleId::new("Heading1Char")));
        assert_eq!(heading.ppr.keep_next, Some(Toggle::On));
        let num_pr = heading
            .ppr
            .num_pr
            .as_ref()
            .expect("the numbering is parsed");
        assert_eq!(num_pr.ilvl, Some(1));
        assert_eq!(num_pr.num_id, Some(NumId::new(5)));
        assert_eq!(heading.rpr.b, Some(Toggle::On));
        assert_eq!(heading.rpr.color, Some(Color::Rgb(0x2F_5496)));

        let character = table
            .character
            .get(&StyleId::new("Heading1Char"))
            .expect("the character style is parsed");
        assert!(character.custom);
        assert_eq!(character.link, Some(StyleId::new("Heading1")));
        assert_eq!(character.rpr.i, Some(Toggle::On));

        assert_eq!(table.table.len(), 1);
        assert_eq!(table.numbering.len(), 1);
        assert_eq!(
            table.table.keys().next().map(StyleId::as_str),
            Some("TableNormal")
        );
    }

    #[test]
    fn a_based_on_chain_of_three_levels_is_kept_as_is() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="Normal"/>
               <w:style w:type="paragraph" w:styleId="Heading1"><w:basedOn w:val="Normal"/></w:style>
               <w:style w:type="paragraph" w:styleId="Heading2"><w:basedOn w:val="Heading1"/></w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            table.paragraph[&StyleId::new("Heading2")].based_on,
            Some(StyleId::new("Heading1"))
        );
    }

    #[test]
    fn hidden_aliases_and_custom_style_flags_are_parsed() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:customStyle="1" w:styleId="Quote">
                 <w:aliases w:val="Pull Quote, Цитата"/>
                 <w:semiHidden/><w:qFormat/>
               </w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let quote = &table.paragraph[&StyleId::new("Quote")];
        assert!(quote.custom);
        assert!(quote.hidden);
        assert_eq!(
            quote.aliases,
            vec!["Pull Quote".to_owned(), "Цитата".to_owned()]
        );
    }

    #[test]
    fn a_duplicate_style_id_keeps_the_first_style() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="first"/></w:style>
               <w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="second"/></w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::DuplicateStyleId]);
        let normal = &table.paragraph[&StyleId::new("Normal")];
        assert_eq!(normal.name.as_deref(), Some("first"));
    }

    #[test]
    fn a_based_on_cycle_warns_and_the_parse_ends() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="A"><w:basedOn w:val="B"/></w:style>
               <w:style w:type="paragraph" w:styleId="B"><w:basedOn w:val="A"/></w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::CyclicBasedOn]);
        assert_eq!(table.paragraph.len(), 2);
    }

    #[test]
    fn a_missing_base_style_warns() {
        let (_, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="A"><w:basedOn w:val="Gone"/></w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::MissingStyleRef]);
    }

    #[test]
    fn a_table_style_keeps_its_table_properties_and_conditional_formats() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="table" w:styleId="Banded">
                 <w:tblPr>
                   <w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="7F7F7F"/><w:insideV w:val="dotted"/></w:tblBorders>
                   <w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:top w:w="0" w:type="nil"/></w:tblCellMar>
                   <w:tblW w:w="5000" w:type="dxa"/>
                   <w:tblLook w:val="04A0"/>
                 </w:tblPr>
                 <w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr><w:tblPr><w:tblW w:w="2500" w:type="pct"/></w:tblPr></w:tblStylePr>
                 <w:tblStylePr><w:pPr><w:jc w:val="center"/></w:pPr></w:tblStylePr>
               </w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let style = &table.table[&StyleId::new("Banded")];
        let top = style
            .tbl_pr
            .borders
            .top
            .as_ref()
            .expect("the top border is parsed");
        assert_eq!(top.val, BorderStyle::Single);
        assert_eq!(top.sz, Some(4));
        assert_eq!(top.color, Some(Color::Rgb(0x7F_7F_7F)));
        assert_eq!(
            style
                .tbl_pr
                .borders
                .inside_v
                .as_ref()
                .map(|border| &border.val),
            Some(&BorderStyle::Dotted)
        );
        assert_eq!(style.tbl_pr.cell_margins.left, Some(Twips::new(108)));
        assert_eq!(style.tbl_pr.cell_margins.top, None);
        assert_eq!(style.tbl_pr.width, Some(TableWidth::Dxa(Twips::new(5000))));
        let look = style.tbl_pr.look.clone().expect("the look is parsed");
        assert!(look.first_row && look.first_column && look.no_v_band);
        assert!(!look.last_row && !look.last_column && !look.no_h_band);

        assert_eq!(style.conditional.len(), 2);
        let first_row = &style.conditional[0];
        assert_eq!(first_row.kind, TableStyleCondition::FirstRow);
        assert_eq!(first_row.rpr.b, Some(Toggle::On));
        assert_eq!(first_row.tbl_pr.width, Some(TableWidth::Pct(50.0)));
        assert_eq!(style.conditional[1].kind, TableStyleCondition::WholeTable);
        assert_eq!(style.conditional[1].ppr.jc, Some(Justification::Center));
    }

    #[test]
    fn an_unknown_style_type_is_skipped_with_a_warning() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="chart" w:styleId="Chart"/><w:style w:type="paragraph" w:styleId="Normal"/>"#,
        );

        assert_eq!(warnings, vec![WarningKind::UnknownElement]);
        assert_eq!(table.paragraph.len(), 1);
        assert!(table.table.is_empty());
    }

    #[test]
    fn a_style_without_an_id_is_skipped_with_a_warning() {
        let (table, warnings) =
            parse_xml(r#"<w:style w:type="paragraph"><w:name w:val="Nameless"/></w:style>"#);

        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        assert!(table.paragraph.is_empty());
    }

    #[test]
    fn unrecognized_properties_are_kept_as_xml() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="Normal">
                 <w:pPr><w:lang w:val="en-US"/><w:weirdThing><w:inner/></w:weirdThing></w:pPr>
                 <w:rPr><w:rtl/></w:rPr>
               </w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let style = &table.paragraph[&StyleId::new("Normal")];
        assert!(style
            .ppr
            .unknown
            .iter()
            .any(|(name, xml)| name == "lang" && xml.contains("w:val=\"en-US\"")));
        assert!(style
            .ppr
            .unknown
            .iter()
            .any(|(name, xml)| name == "weirdThing" && xml.contains("<w:inner/>")));
        assert!(style.rpr.unknown.iter().any(|(name, _)| name == "rtl"));
    }

    #[test]
    fn a_broken_styles_part_is_rejected() {
        let mut ctx = ParseCtx::new();
        let err = parse(b"<w:styles><w:style ", &mut ctx, "word/styles.xml")
            .expect_err("a truncated document is rejected");
        assert!(
            matches!(
                err,
                Error::XmlFatal { .. } | Error::Malformed { .. } | Error::Core(_)
            ),
            "{err}"
        );

        let mut ctx = ParseCtx::new();
        let err = parse(b"<w:notStyles/>", &mut ctx, "word/styles.xml")
            .expect_err("a foreign root is rejected");
        assert!(matches!(err, Error::Malformed { .. }), "{err}");
    }

    #[test]
    fn an_empty_styles_part_is_valid() {
        let (table, warnings) = parse_xml("");

        assert!(warnings.is_empty());
        assert_eq!(table, StyleTable::default());
    }

    // -----------------------------------------------------------------------
    // Таблицы значений ECMA-376
    // -----------------------------------------------------------------------

    /// Стиль абзаца с данным содержимым — заготовка для табличных тестов.
    fn paragraph_style(id: &str, inner: &str) -> String {
        format!("<w:style w:type=\"paragraph\" w:styleId=\"{id}\">{inner}</w:style>")
    }

    /// Табличный стиль с данным содержимым — заготовка для условных форматов.
    fn table_style(id: &str, inner: &str) -> String {
        format!("<w:style w:type=\"table\" w:styleId=\"{id}\">{inner}</w:style>")
    }

    /// Таблица `ST_Border` (ECMA-376 §17.18.2) целиком: каждый `w:val` границы
    /// разбирается в свой вариант, незнакомый сохраняется в `Other`.
    #[test]
    fn border_style_covers_the_whole_st_border() {
        let cases: [(&str, BorderStyle); 27] = [
            ("nil", BorderStyle::Nil),
            ("none", BorderStyle::None),
            ("single", BorderStyle::Single),
            ("thick", BorderStyle::Thick),
            ("double", BorderStyle::Double),
            ("dotted", BorderStyle::Dotted),
            ("dashed", BorderStyle::Dashed),
            ("dotDash", BorderStyle::DotDash),
            ("dotDotDash", BorderStyle::DotDotDash),
            ("triple", BorderStyle::Triple),
            ("thinThickSmallGap", BorderStyle::ThinThickSmallGap),
            ("thickThinSmallGap", BorderStyle::ThickThinSmallGap),
            ("thinThickThinSmallGap", BorderStyle::ThinThickThinSmallGap),
            ("thinThickMediumGap", BorderStyle::ThinThickMediumGap),
            ("thickThinMediumGap", BorderStyle::ThickThinMediumGap),
            (
                "thinThickThinMediumGap",
                BorderStyle::ThinThickThinMediumGap,
            ),
            ("thinThickLargeGap", BorderStyle::ThinThickLargeGap),
            ("thickThinLargeGap", BorderStyle::ThickThinLargeGap),
            ("thinThickThinLargeGap", BorderStyle::ThinThickThinLargeGap),
            ("wave", BorderStyle::Wave),
            ("doubleWave", BorderStyle::DoubleWave),
            ("dashSmallGap", BorderStyle::DashSmallGap),
            ("dashDotStroked", BorderStyle::DashDotStroked),
            ("threeDEmboss", BorderStyle::ThreeDEmboss),
            ("threeDEngrave", BorderStyle::ThreeDEngrave),
            ("outset", BorderStyle::Outset),
            ("inset", BorderStyle::Inset),
        ];
        // Перечень модели обязан совпасть со `ST_Border`: разойдясь, они
        // разойдутся и с таблицей разбора.
        assert_eq!(cases.len(), BorderStyle::ALL.len());

        let mut body = String::new();
        for (index, (raw, _)) in cases.iter().enumerate() {
            body.push_str(&paragraph_style(
                &format!("S{index}"),
                &format!("<w:pPr><w:pBdr><w:top w:val=\"{raw}\"/></w:pBdr></w:pPr>"),
            ));
        }
        body.push_str(&paragraph_style(
            "Other",
            "<w:pPr><w:pBdr><w:top w:val=\"squiggly\"/></w:pBdr></w:pPr>",
        ));

        let (table, warnings) = parse_xml(&body);

        assert!(warnings.is_empty(), "{warnings:?}");
        for (index, (raw, expected)) in cases.iter().enumerate() {
            let top = table.paragraph[&StyleId::new(format!("S{index}"))]
                .ppr
                .p_bdr
                .as_ref()
                .and_then(|borders| borders.top.as_ref())
                .expect("the top border is parsed");
            assert_eq!(&top.val, expected, "w:val=\"{raw}\"");
        }
        let other = table.paragraph[&StyleId::new("Other")]
            .ppr
            .p_bdr
            .as_ref()
            .and_then(|borders| borders.top.as_ref())
            .expect("the top border is parsed");
        assert_eq!(other.val, BorderStyle::Other("squiggly".to_owned()));
    }

    /// Слоты `w:pBdr` (§17.3.1.29): `w:start`/`w:end` — то же, что
    /// `w:left`/`w:right`, `w:bottom` без `w:val` — запасное `single`
    /// у `parse_border`, лишний ребёнок — `UnknownElement`, пустой `w:pBdr`
    /// не заполняет слоты.
    #[test]
    fn paragraph_borders_cover_every_slot_and_the_default_style() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="All">
                 <w:pPr><w:pBdr>
                   <w:top w:val="double" w:sz="8" w:space="1" w:color="FF0000"/>
                   <w:left w:val="nil"/><w:start w:val="dashed"/>
                   <w:bottom/><w:right w:val="wave"/><w:end w:val="dotted"/>
                   <w:between w:val="thick"/><w:bar w:val="inset"/>
                   <w:weird/>
                 </w:pBdr></w:pPr>
               </w:style>
               <w:style w:type="paragraph" w:styleId="Empty"><w:pPr><w:pBdr/></w:pPr></w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::UnknownElement]);
        let borders = table.paragraph[&StyleId::new("All")]
            .ppr
            .p_bdr
            .as_ref()
            .expect("the borders are parsed");
        let top = borders.top.as_ref().expect("the top border is parsed");
        assert_eq!(top.val, BorderStyle::Double);
        assert_eq!(top.sz, Some(8));
        assert_eq!(top.space, Some(1));
        assert_eq!(top.color, Some(Color::Rgb(0x00_FF_00_00)));
        // `w:start`/`w:end` записаны после `w:left`/`w:right` и перекрывают их:
        // это тот же край, а не отдельный слот.
        assert_eq!(
            borders.left.as_ref().map(|border| &border.val),
            Some(&BorderStyle::Dashed)
        );
        assert_eq!(
            borders.right.as_ref().map(|border| &border.val),
            Some(&BorderStyle::Dotted)
        );
        assert_eq!(
            borders.bottom.as_ref().map(|border| &border.val),
            Some(&BorderStyle::Single)
        );
        assert_eq!(
            borders.between.as_ref().map(|border| &border.val),
            Some(&BorderStyle::Thick)
        );
        assert_eq!(
            borders.bar.as_ref().map(|border| &border.val),
            Some(&BorderStyle::Inset)
        );
        assert_eq!(
            table.paragraph[&StyleId::new("Empty")].ppr.p_bdr,
            Some(ParagraphBorders::default())
        );
    }

    /// Таблица `ST_Underline` (§17.3.2.29) целиком: 18 значений, незнакомое —
    /// `Other`, а `w:u` без `w:val` даёт запасное `single`.
    #[test]
    fn underline_covers_the_whole_st_underline() {
        let cases: [(&str, Underline); 18] = [
            ("single", Underline::Single),
            ("words", Underline::Words),
            ("double", Underline::Double),
            ("thick", Underline::Thick),
            ("dotted", Underline::Dotted),
            ("dottedHeavy", Underline::DottedHeavy),
            ("dash", Underline::Dash),
            ("dashedHeavy", Underline::DashedHeavy),
            ("dashLong", Underline::DashLong),
            ("dashLongHeavy", Underline::DashLongHeavy),
            ("dotDash", Underline::DotDash),
            ("dashDotHeavy", Underline::DashDotHeavy),
            ("dotDotDash", Underline::DotDotDash),
            ("dashDotDotHeavy", Underline::DashDotDotHeavy),
            ("wave", Underline::Wave),
            ("wavyHeavy", Underline::WavyHeavy),
            ("wavyDouble", Underline::WavyDouble),
            ("none", Underline::None),
        ];

        let mut body = String::new();
        for (index, (raw, _)) in cases.iter().enumerate() {
            body.push_str(&paragraph_style(
                &format!("S{index}"),
                &format!("<w:rPr><w:u w:val=\"{raw}\"/></w:rPr>"),
            ));
        }
        body.push_str(&paragraph_style("NoVal", "<w:rPr><w:u/></w:rPr>"));
        body.push_str(&paragraph_style(
            "Other",
            "<w:rPr><w:u w:val=\"squiggly\"/></w:rPr>",
        ));

        let (table, warnings) = parse_xml(&body);

        assert!(warnings.is_empty(), "{warnings:?}");
        for (index, (raw, expected)) in cases.iter().enumerate() {
            let style = &table.paragraph[&StyleId::new(format!("S{index}"))];
            assert_eq!(style.rpr.u.as_ref(), Some(expected), "w:val=\"{raw}\"");
        }
        assert_eq!(
            table.paragraph[&StyleId::new("NoVal")].rpr.u,
            Some(Underline::Single)
        );
        assert_eq!(
            table.paragraph[&StyleId::new("Other")].rpr.u,
            Some(Underline::Other("squiggly".to_owned()))
        );
    }

    /// Таблица `ST_HighlightColor` (§17.18.40) целиком. `darkGrey`/`lightGrey` —
    /// не значения стандарта (там `darkGray`/`lightGray`), а написание из
    /// `DrawingML` `PresetColorValues`; парсер принимает их как синонимы.
    #[test]
    fn highlight_covers_the_whole_st_highlight_color() {
        let cases: [(&str, Highlight); 19] = [
            ("black", Highlight::Black),
            ("blue", Highlight::Blue),
            ("cyan", Highlight::Cyan),
            ("green", Highlight::Green),
            ("magenta", Highlight::Magenta),
            ("red", Highlight::Red),
            ("yellow", Highlight::Yellow),
            ("white", Highlight::White),
            ("darkBlue", Highlight::DarkBlue),
            ("darkCyan", Highlight::DarkCyan),
            ("darkGreen", Highlight::DarkGreen),
            ("darkMagenta", Highlight::DarkMagenta),
            ("darkRed", Highlight::DarkRed),
            ("darkYellow", Highlight::DarkYellow),
            ("darkGray", Highlight::DarkGray),
            ("lightGray", Highlight::LightGray),
            ("none", Highlight::None),
            ("darkGrey", Highlight::DarkGray),
            ("lightGrey", Highlight::LightGray),
        ];

        let mut body = String::new();
        for (index, (raw, _)) in cases.iter().enumerate() {
            body.push_str(&paragraph_style(
                &format!("S{index}"),
                &format!("<w:rPr><w:highlight w:val=\"{raw}\"/></w:rPr>"),
            ));
        }
        body.push_str(&paragraph_style("NoVal", "<w:rPr><w:highlight/></w:rPr>"));
        body.push_str(&paragraph_style(
            "Other",
            "<w:rPr><w:highlight w:val=\"squiggly\"/></w:rPr>",
        ));

        let (table, warnings) = parse_xml(&body);

        assert!(warnings.is_empty(), "{warnings:?}");
        for (index, (raw, expected)) in cases.iter().enumerate() {
            let style = &table.paragraph[&StyleId::new(format!("S{index}"))];
            assert_eq!(
                style.rpr.highlight.as_ref(),
                Some(expected),
                "w:val=\"{raw}\""
            );
        }
        assert_eq!(table.paragraph[&StyleId::new("NoVal")].rpr.highlight, None);
        assert_eq!(
            table.paragraph[&StyleId::new("Other")].rpr.highlight,
            Some(Highlight::Other("squiggly".to_owned()))
        );
    }

    /// Таблица `ST_Shd` (§17.18.78) целиком: 38 узоров заливки, незнакомый —
    /// `Other`, отсутствие `w:val` (в схеме обязательного) — `clear`.
    #[test]
    fn shading_covers_the_whole_st_shd() {
        let cases: [(&str, ShadingPattern); 38] = [
            ("nil", ShadingPattern::Nil),
            ("clear", ShadingPattern::Clear),
            ("solid", ShadingPattern::Solid),
            ("horzStripe", ShadingPattern::HorzStripe),
            ("vertStripe", ShadingPattern::VertStripe),
            ("reverseDiagStripe", ShadingPattern::ReverseDiagStripe),
            ("diagStripe", ShadingPattern::DiagStripe),
            ("horzCross", ShadingPattern::HorzCross),
            ("diagCross", ShadingPattern::DiagCross),
            ("thinHorzStripe", ShadingPattern::ThinHorzStripe),
            ("thinVertStripe", ShadingPattern::ThinVertStripe),
            (
                "thinReverseDiagStripe",
                ShadingPattern::ThinReverseDiagStripe,
            ),
            ("thinDiagStripe", ShadingPattern::ThinDiagStripe),
            ("thinHorzCross", ShadingPattern::ThinHorzCross),
            ("thinDiagCross", ShadingPattern::ThinDiagCross),
            ("pct5", ShadingPattern::Pct5),
            ("pct10", ShadingPattern::Pct10),
            ("pct12", ShadingPattern::Pct12),
            ("pct15", ShadingPattern::Pct15),
            ("pct20", ShadingPattern::Pct20),
            ("pct25", ShadingPattern::Pct25),
            ("pct30", ShadingPattern::Pct30),
            ("pct35", ShadingPattern::Pct35),
            ("pct37", ShadingPattern::Pct37),
            ("pct40", ShadingPattern::Pct40),
            ("pct45", ShadingPattern::Pct45),
            ("pct50", ShadingPattern::Pct50),
            ("pct55", ShadingPattern::Pct55),
            ("pct60", ShadingPattern::Pct60),
            ("pct62", ShadingPattern::Pct62),
            ("pct65", ShadingPattern::Pct65),
            ("pct70", ShadingPattern::Pct70),
            ("pct75", ShadingPattern::Pct75),
            ("pct80", ShadingPattern::Pct80),
            ("pct85", ShadingPattern::Pct85),
            ("pct87", ShadingPattern::Pct87),
            ("pct90", ShadingPattern::Pct90),
            ("pct95", ShadingPattern::Pct95),
        ];
        assert_eq!(cases.len(), ShadingPattern::ALL.len());

        let mut body = String::new();
        for (index, (raw, _)) in cases.iter().enumerate() {
            body.push_str(&paragraph_style(
                &format!("S{index}"),
                &format!("<w:pPr><w:shd w:val=\"{raw}\"/></w:pPr>"),
            ));
        }
        body.push_str(&paragraph_style("NoVal", "<w:pPr><w:shd/></w:pPr>"));
        body.push_str(&paragraph_style(
            "Other",
            "<w:pPr><w:shd w:val=\"squiggly\"/></w:pPr>",
        ));

        let (table, warnings) = parse_xml(&body);

        assert!(warnings.is_empty(), "{warnings:?}");
        for (index, (raw, expected)) in cases.iter().enumerate() {
            let shd = table.paragraph[&StyleId::new(format!("S{index}"))]
                .ppr
                .shd
                .as_ref()
                .expect("the shading is parsed");
            assert_eq!(&shd.val, expected, "w:val=\"{raw}\"");
        }
        assert_eq!(
            table.paragraph[&StyleId::new("NoVal")]
                .ppr
                .shd
                .as_ref()
                .map(|shd| &shd.val),
            Some(&ShadingPattern::Clear)
        );
        assert_eq!(
            table.paragraph[&StyleId::new("Other")]
                .ppr
                .shd
                .as_ref()
                .map(|shd| shd.val.clone()),
            Some(ShadingPattern::Other("squiggly".to_owned()))
        );
    }

    /// Цвета `w:shd` (`w:color`/`w:fill`, §17.18.78): `auto` и `RRGGBB`;
    /// негодный цвет не теряется молча.
    #[test]
    fn shading_parses_colors_and_warns_about_a_broken_one() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="Shaded">
                 <w:pPr><w:shd w:val="pct25" w:color="auto" w:fill="00FF00"/></w:pPr>
               </w:style>
               <w:style w:type="paragraph" w:styleId="Broken">
                 <w:pPr><w:shd w:val="solid" w:color="not-a-color"/></w:pPr>
               </w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        let shaded = table.paragraph[&StyleId::new("Shaded")]
            .ppr
            .shd
            .as_ref()
            .expect("the shading is parsed");
        assert_eq!(shaded.val, ShadingPattern::Pct25);
        assert_eq!(shaded.color, Some(Color::Auto));
        assert_eq!(shaded.fill, Some(Color::Rgb(0x00_FF_00)));

        let broken = table.paragraph[&StyleId::new("Broken")]
            .ppr
            .shd
            .as_ref()
            .expect("the shading is parsed");
        assert_eq!(broken.val, ShadingPattern::Solid);
        assert_eq!(broken.color, None);
        assert_eq!(broken.fill, None);
    }

    /// Таблицы `ST_TabJc` (`w:tab/@w:val`, §17.3.1.37) и `ST_TabTlc`
    /// (`w:tab/@w:leader`) целиком; `w:pos` переносится в twips как есть.
    #[test]
    fn tab_stops_carry_the_position_kind_and_leader() {
        let kinds: [(&str, TabStopKind); 9] = [
            ("bar", TabStopKind::Bar),
            ("center", TabStopKind::Center),
            ("clear", TabStopKind::Clear),
            ("decimal", TabStopKind::Decimal),
            ("end", TabStopKind::End),
            ("num", TabStopKind::Num),
            ("start", TabStopKind::Start),
            ("left", TabStopKind::Left),
            ("right", TabStopKind::Right),
        ];
        let leaders: [(&str, TabLeader); 6] = [
            ("none", TabLeader::None),
            ("dot", TabLeader::Dot),
            ("hyphen", TabLeader::Hyphen),
            ("middleDot", TabLeader::MiddleDot),
            ("heavy", TabLeader::Heavy),
            ("underscore", TabLeader::Underscore),
        ];

        let mut body = String::new();
        for (index, (raw, _)) in kinds.iter().enumerate() {
            body.push_str(&paragraph_style(
                &format!("Kind{index}"),
                &format!("<w:pPr><w:tabs><w:tab w:pos=\"720\" w:val=\"{raw}\"/></w:tabs></w:pPr>"),
            ));
        }
        for (index, (raw, _)) in leaders.iter().enumerate() {
            body.push_str(&paragraph_style(
                &format!("Leader{index}"),
                &format!(
                    "<w:pPr><w:tabs><w:tab w:pos=\"720\" w:leader=\"{raw}\"/></w:tabs></w:pPr>"
                ),
            ));
        }
        // `w:val`/`w:pos` в схеме обязательны; без них парсер не выдумывает
        // мусор, а берёт `left` и нулевую позицию.
        body.push_str(&paragraph_style(
            "Bare",
            "<w:pPr><w:tabs><w:tab/></w:tabs></w:pPr>",
        ));
        body.push_str(&paragraph_style(
            "Other",
            "<w:pPr><w:tabs><w:tab w:pos=\"720\" w:val=\"squiggly\" w:leader=\"squiggly\"/></w:tabs></w:pPr>",
        ));
        // Ребёнок `w:tabs`, не `w:tab`, пропускается целиком.
        body.push_str(&paragraph_style(
            "Extra",
            "<w:pPr><w:tabs><w:bogus><w:inner/></w:bogus><w:tab w:pos=\"720\"/></w:tabs></w:pPr>",
        ));

        let (table, warnings) = parse_xml(&body);

        assert!(warnings.is_empty(), "{warnings:?}");
        for (index, (raw, expected)) in kinds.iter().enumerate() {
            let stop = &table.paragraph[&StyleId::new(format!("Kind{index}"))]
                .ppr
                .tabs[0];
            assert_eq!(stop.val, Twips::new(720), "w:val=\"{raw}\"");
            assert_eq!(&stop.kind, expected, "w:val=\"{raw}\"");
            assert_eq!(stop.leader, TabLeader::None, "w:val=\"{raw}\"");
        }
        for (index, (raw, expected)) in leaders.iter().enumerate() {
            let stop = &table.paragraph[&StyleId::new(format!("Leader{index}"))]
                .ppr
                .tabs[0];
            assert_eq!(stop.kind, TabStopKind::Left, "w:leader=\"{raw}\"");
            assert_eq!(&stop.leader, expected, "w:leader=\"{raw}\"");
        }
        let bare = &table.paragraph[&StyleId::new("Bare")].ppr.tabs[0];
        assert_eq!(bare.val, Twips::new(0));
        let other = &table.paragraph[&StyleId::new("Other")].ppr.tabs[0];
        assert_eq!(other.kind, TabStopKind::Other("squiggly".to_owned()));
        assert_eq!(other.leader, TabLeader::Other("squiggly".to_owned()));
        let extra = &table.paragraph[&StyleId::new("Extra")].ppr.tabs;
        assert_eq!(extra.len(), 1);
        assert_eq!(extra[0].kind, TabStopKind::Left);
    }

    /// Таблица `ST_TblStyleOverrideType` (§17.18.92) целиком; условный формат
    /// без `w:type` — вся таблица (§17.7.6), `w:trPr`/`w:tcPr` не хранятся
    /// в модели и не считаются ошибкой, лишний ребёнок — считается.
    #[test]
    fn table_style_conditions_cover_the_whole_st_tbl_style_override_type() {
        let cases: [(&str, TableStyleCondition); 13] = [
            ("wholeTable", TableStyleCondition::WholeTable),
            ("firstRow", TableStyleCondition::FirstRow),
            ("lastRow", TableStyleCondition::LastRow),
            ("firstCol", TableStyleCondition::FirstCol),
            ("lastCol", TableStyleCondition::LastCol),
            ("band1Vert", TableStyleCondition::Band1Vert),
            ("band2Vert", TableStyleCondition::Band2Vert),
            ("band1Horz", TableStyleCondition::Band1Horz),
            ("band2Horz", TableStyleCondition::Band2Horz),
            ("neCell", TableStyleCondition::NeCell),
            ("nwCell", TableStyleCondition::NwCell),
            ("seCell", TableStyleCondition::SeCell),
            ("swCell", TableStyleCondition::SwCell),
        ];
        assert_eq!(cases.len(), TableStyleCondition::ALL.len());

        let mut body = String::new();
        for (index, (raw, _)) in cases.iter().enumerate() {
            body.push_str(&table_style(
                &format!("S{index}"),
                &format!("<w:tblStylePr w:type=\"{raw}\"/>"),
            ));
        }
        body.push_str(
            r#"<w:style w:type="table" w:styleId="Extra">
                 <w:tblStylePr w:type="firstRow">
                   <w:trPr><w:cnfStyle w:val="001000000000"/></w:trPr><w:tcPr/><w:weird/>
                 </w:tblStylePr>
                 <w:tblStylePr w:type="squiggly"/>
               </w:style>"#,
        );

        let (table, warnings) = parse_xml(&body);

        assert_eq!(warnings, vec![WarningKind::UnknownElement]);
        for (index, (raw, expected)) in cases.iter().enumerate() {
            let style = &table.table[&StyleId::new(format!("S{index}"))];
            assert_eq!(style.conditional.len(), 1, "w:type=\"{raw}\"");
            assert_eq!(&style.conditional[0].kind, expected, "w:type=\"{raw}\"");
        }
        let extra = &table.table[&StyleId::new("Extra")];
        assert_eq!(extra.conditional.len(), 2);
        assert_eq!(extra.conditional[0].kind, TableStyleCondition::FirstRow);
        assert_eq!(extra.conditional[0].ppr, RawPPr::default());
        assert_eq!(
            extra.conditional[1].kind,
            TableStyleCondition::Other("squiggly".to_owned())
        );
    }

    /// Редкие дети `w:pPr`: пустые `w:numPr`/`w:tabs`/`w:pBdr`/`w:rPr` дают
    /// умолчания, `w:outlineLvl` держится в 0..=9, `w:ind` читает логические
    /// края, а `w:sectPr` сохраняется как неподдержанный XML.
    #[test]
    fn p_pr_covers_rare_properties_and_outline_levels() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="paragraph" w:styleId="Rare">
                 <w:pPr>
                   <w:pStyle w:val="Base"/><w:numPr/>
                   <w:ind w:left="113" w:right="226" w:firstLine="57" w:hanging="28"/>
                   <w:keepLines w:val="0"/><w:pageBreakBefore w:val="1"/>
                   <w:widowControl w:val="false"/><w:outlineLvl w:val="9"/>
                   <w:tabs/><w:pBdr/><w:rPr/>
                 </w:pPr>
               </w:style>
               <w:style w:type="paragraph" w:styleId="Bidi">
                 <w:pPr><w:ind w:start="113" w:end="226"/></w:pPr>
               </w:style>
               <w:style w:type="paragraph" w:styleId="Section">
                 <w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr>
               </w:style>
               <w:style w:type="paragraph" w:styleId="BadLevel">
                 <w:pPr><w:outlineLvl w:val="10"/></w:pPr>
               </w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        let rare = &table.paragraph[&StyleId::new("Rare")];
        assert_eq!(rare.ppr.style, Some(StyleId::new("Base")));
        assert_eq!(rare.ppr.num_pr, Some(NumPr::default()));
        assert_eq!(rare.ppr.keep_lines, Some(Toggle::Off));
        assert_eq!(rare.ppr.page_break_before, Some(Toggle::On));
        assert_eq!(rare.ppr.widow_control, Some(Toggle::Off));
        assert_eq!(rare.ppr.outline_lvl, Some(9));
        assert_eq!(rare.ppr.tabs, Vec::new());
        assert_eq!(rare.ppr.p_bdr, Some(ParagraphBorders::default()));
        assert_eq!(rare.ppr.r_pr, Some(RawRPr::default()));
        let ind = rare.ppr.ind.as_ref().expect("the indents are parsed");
        assert_eq!(ind.left, Some(Twips::new(113)));
        assert_eq!(ind.right, Some(Twips::new(226)));
        assert_eq!(ind.first_line, Some(Twips::new(57)));
        assert_eq!(ind.hanging, Some(Twips::new(28)));

        let bidi = table.paragraph[&StyleId::new("Bidi")]
            .ppr
            .ind
            .as_ref()
            .expect("the indents are parsed");
        assert_eq!(bidi.left, Some(Twips::new(113)));
        assert_eq!(bidi.right, Some(Twips::new(226)));

        assert_eq!(
            table.paragraph[&StyleId::new("BadLevel")].ppr.outline_lvl,
            None
        );
        let section = &table.paragraph[&StyleId::new("Section")];
        let kept = section
            .ppr
            .unknown
            .iter()
            .find(|(name, _)| name == "sectPr")
            .expect("w:sectPr is kept as XML");
        assert!(kept.1.contains("w:pgSz"), "{}", kept.1);
    }

    /// Редкие свойства `w:rPr`: переключатели начертания, `w:szCs`,
    /// `w:spacing`/`w:position` и негодный `w:color`.
    #[test]
    fn r_pr_covers_toggles_character_spacing_and_broken_color() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="character" w:styleId="Full">
                 <w:rPr>
                   <w:rStyle w:val="Base"/>
                   <w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="MS Mincho" w:cs="Arial" w:hint="eastAsia"/>
                   <w:b/><w:i w:val="0"/><w:caps/><w:smallCaps/><w:strike/><w:dstrike/>
                   <w:vanish/><w:outline/><w:shadow/><w:emboss/><w:imprint/>
                   <w:color w:val="00FF00"/><w:sz w:val="24"/><w:szCs w:val="24"/>
                   <w:spacing w:val="-20"/><w:position w:val="6"/><w:vertAlign w:val="superscript"/>
                   <w:weird/>
                 </w:rPr>
               </w:style>
               <w:style w:type="character" w:styleId="Broken">
                 <w:rPr><w:color w:val="zzz"/></w:rPr>
               </w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        let full = &table.character[&StyleId::new("Full")].rpr;
        assert_eq!(full.style, Some(StyleId::new("Base")));
        let fonts = full.r_fonts.as_ref().expect("the fonts are parsed");
        assert_eq!(fonts.ascii.as_deref(), Some("Arial"));
        assert_eq!(fonts.east_asia.as_deref(), Some("MS Mincho"));
        assert_eq!(fonts.hint, Some(FontHint::EastAsia));
        assert_eq!(full.b, Some(Toggle::On));
        assert_eq!(full.i, Some(Toggle::Off));
        assert_eq!(full.caps, Some(Toggle::On));
        assert_eq!(full.small_caps, Some(Toggle::On));
        assert_eq!(full.strike, Some(Toggle::On));
        assert_eq!(full.dstrike, Some(Toggle::On));
        assert_eq!(full.vanish, Some(Toggle::On));
        assert_eq!(full.outline, Some(Toggle::On));
        assert_eq!(full.shadow, Some(Toggle::On));
        assert_eq!(full.emboss, Some(Toggle::On));
        assert_eq!(full.imprint, Some(Toggle::On));
        assert_eq!(full.color, Some(Color::Rgb(0x00_FF_00)));
        assert_eq!(full.sz, Some(HalfPoint::new(24)));
        assert_eq!(full.sz_cs, Some(HalfPoint::new(24)));
        assert_eq!(
            full.spacing,
            Some(CharacterSpacing {
                value: Some(Twips::new(-20))
            })
        );
        assert_eq!(full.position, Some(HalfPoint::new(6)));
        assert_eq!(full.vert_align, Some(VertAlign::Superscript));
        assert!(full.unknown.iter().any(|(name, _)| name == "weird"));

        let broken = &table.character[&StyleId::new("Broken")].rpr;
        assert_eq!(broken.color, None);
    }

    /// Редкие дети `w:tblPr`: пустые `w:tblBorders`/`w:tblCellMar`, ширина
    /// `auto`, выравнивание и отступ таблицы, неподдержанный ребёнок.
    #[test]
    fn tbl_pr_covers_empty_blocks_alignment_and_indent() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:type="table" w:styleId="Plain">
                 <w:tblPr>
                   <w:tblBorders/><w:tblCellMar/><w:tblW w:w="0" w:type="auto"/>
                   <w:jc w:val="center"/><w:tblInd w:w="113"/><w:weird/>
                 </w:tblPr>
               </w:style>
               <w:style w:type="table" w:styleId="Empty"><w:tblPr/></w:style>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let plain = &table.table[&StyleId::new("Plain")].tbl_pr;
        assert_eq!(plain.borders, TableBorders::default());
        assert_eq!(plain.cell_margins, CellMargins::default());
        assert_eq!(plain.width, Some(TableWidth::Auto));
        assert_eq!(plain.jc, Some(Justification::Center));
        assert_eq!(plain.indent, Some(Twips::new(113)));
        assert!(plain.unknown.iter().any(|(name, _)| name == "weird"));
        assert_eq!(
            table.table[&StyleId::new("Empty")].tbl_pr,
            RawTblPr::default()
        );
    }

    /// `w:style` без `w:type` пропускается с предупреждением (§17.7.2),
    /// а самозакрытый стиль детей не читает вовсе.
    #[test]
    fn styles_without_a_type_or_children_are_handled() {
        let (table, warnings) = parse_xml(
            r#"<w:style w:styleId="NoType"/>
               <w:style w:type="paragraph" w:styleId="Empty"/>
               <w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#,
        );

        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        assert!(!table.paragraph.contains_key(&StyleId::new("NoType")));
        assert_eq!(
            table.paragraph[&StyleId::new("Empty")].ppr,
            RawPPr::default()
        );
        assert_eq!(
            table.paragraph[&StyleId::new("Normal")].name.as_deref(),
            Some("Normal")
        );
    }

    // -----------------------------------------------------------------------
    // Фикстуры
    // -----------------------------------------------------------------------

    /// Стиль, найденный в таблице: вид и поля, которые сверяет сайдкар.
    struct Found<'a> {
        kind: &'static str,
        based_on: Option<&'a str>,
        is_default: bool,
    }

    /// Найти стиль по идентификатору в любой из четырёх таблиц.
    fn find_style<'a>(table: &'a StyleTable, id: &str) -> Option<Found<'a>> {
        let key = StyleId::new(id);
        if let Some(style) = table.paragraph.get(&key) {
            return Some(Found {
                kind: "paragraph",
                based_on: style.based_on.as_ref().map(StyleId::as_str),
                is_default: style.is_default,
            });
        }
        if let Some(style) = table.character.get(&key) {
            return Some(Found {
                kind: "character",
                based_on: style.based_on.as_ref().map(StyleId::as_str),
                is_default: style.is_default,
            });
        }
        if let Some(style) = table.table.get(&key) {
            return Some(Found {
                kind: "table",
                based_on: style.based_on.as_ref().map(StyleId::as_str),
                is_default: style.is_default,
            });
        }
        table.numbering.get(&key).map(|style| Found {
            kind: "numbering",
            based_on: style.based_on.as_ref().map(StyleId::as_str),
            is_default: style.is_default,
        })
    }

    /// Разобрать фикстуру вместе с её сайдкаром.
    fn fixture(dir: &Path, name: &str) -> (StyleTable, Vec<WarningKind>, serde_json::Value) {
        let mut archive = Archive::new(
            fs::read(dir.join(format!("{name}.docx"))).expect("the fixture is readable"),
        )
        .expect("the fixture is an OOXML package");
        let bytes = archive
            .read("word/styles.xml")
            .expect("the fixture has word/styles.xml");
        let mut ctx = ParseCtx::new();
        let table = parse(&bytes, &mut ctx, "word/styles.xml").expect("the fixture parses");
        let warnings = ctx
            .warnings()
            .iter()
            .map(|warning| warning.kind)
            .collect::<Vec<_>>();
        let sidecar = serde_json::from_str(
            &fs::read_to_string(dir.join(format!("{name}.json"))).expect("the sidecar is readable"),
        )
        .expect("the sidecar is JSON");
        (table, warnings, sidecar)
    }

    /// Сверить таблицу с сайдкаром: id, вид, основа и умолчание.
    fn check_fixture(dir: &Path, name: &str) {
        let (table, warnings, sidecar) = fixture(dir, name);
        assert!(
            warnings.is_empty(),
            "{name}: unexpected warnings {warnings:?}"
        );

        // Сайдкар перечисляет только те стили, которые нужны его сценарию:
        // остальные (`Heading1Char` и прочая обвязка Word) в нём не значатся,
        // поэтому сверяется каждая запись сайдкара, а не их количество.
        let expected = sidecar["content"]["styles"]
            .as_array()
            .expect("the sidecar lists styles");
        for style in expected {
            let id = style["id"].as_str().expect("the sidecar style has an id");
            let found =
                find_style(&table, id).unwrap_or_else(|| panic!("{name}: style `{id}` is missing"));
            assert_eq!(
                found.kind,
                style["type"].as_str().expect("the type"),
                "{name}: `{id}`"
            );
            assert_eq!(
                found.based_on,
                style["basedOn"].as_str(),
                "{name}: `{id}` basedOn"
            );
            assert_eq!(
                found.is_default,
                style["default"].as_bool().unwrap_or(false),
                "{name}: `{id}` default"
            );
        }
    }

    #[test]
    fn fixtures_match_their_sidecars() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx/styles");
        for name in [
            "based_on_chain",
            "character_style_run",
            "default_paragraph",
            "direct_formatting_override",
            "doc_defaults",
            "linked_character",
            "qformat_latent",
            "table_style",
        ] {
            check_fixture(&dir, name);
        }
    }

    #[test]
    fn the_table_style_fixture_keeps_borders_and_the_conditional_format() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx/styles");
        let (table, _, _) = fixture(&dir, "table_style");

        let style = &table.table[&StyleId::new("BandedGrid")];
        let top = style
            .tbl_pr
            .borders
            .top
            .as_ref()
            .expect("the fixture has borders");
        assert_eq!(top.val, BorderStyle::Single);
        assert_eq!(top.color, Some(Color::Rgb(0x7F_7F_7F)));
        assert_eq!(style.tbl_pr.cell_margins.left, Some(Twips::new(108)));
        assert_eq!(style.conditional.len(), 1);
        assert_eq!(style.conditional[0].kind, TableStyleCondition::FirstRow);
        assert_eq!(style.conditional[0].rpr.b, Some(Toggle::On));
    }

    #[test]
    fn the_cyclic_fixture_warns_about_the_cycle() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx/broken");
        let (table, warnings, sidecar) = fixture(&dir, "cyclic_based_on");

        assert_eq!(
            sidecar["metadata"]["expectedWarnings"][0].as_str(),
            Some("CyclicBasedOn")
        );
        assert_eq!(warnings, vec![WarningKind::CyclicBasedOn]);
        assert!(table.paragraph.contains_key(&StyleId::new("StyleA")));
    }
}
