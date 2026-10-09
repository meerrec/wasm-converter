//! Разбор `word/document.xml`: тело, абзацы и run'ы (слайс S7a).
//!
//! Здесь первая половина части: `w:body`, `w:p`, `w:pPr`, `w:r`, `w:rPr`,
//! текст, разрывы и символы. Таблицы, рисунки, поля, ссылки, закладки и
//! завершающий `w:sectPr` приедут слайсом S7b — до тех пор такие элементы
//! сохраняются как `Unknown` с предупреждением [`WarningKind::UnknownElement`],
//! чтобы S7b заменил ветки разбором, а не переделывал структуру.
//!
//! Разбор потоковый, одним проходом. Читатель — [`XmlReader::preserving`]:
//! пробелы в `w:t` это данные (`xml:space="preserve"`), а не отступы разметки.

// Вызывающего у парсера ещё нет: его подключит `parse.rs` (слайс S12). До тех
// пор `dead_code` срабатывал бы на каждом элементе модуля — как в `xml.rs`.
#![allow(dead_code)]

use std::borrow::Cow;

use doc_converter_core::xml::XmlReader;
use doc_converter_core::{NodeId, WarningKind};
use quick_xml::events::{BytesStart, Event};

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::{
    AlignH, AlignV, Anchor, BlockItem, Body, Bookmark, Border, BorderStyle, BreakKind, Cell,
    CellBorders, CellMargins, CellVAlign, CellWidth, CharacterSpacing, Color, ColumnDef, Columns,
    Extent, Field, FieldKind, FontHint, GridCol, HalfPoint, HeightRule, Highlight, Hyperlink, Ind,
    Inline, InlineImage, InlineOrAnchor, Justification, LineSpacing, LineSpacingRule, Margins,
    NumId, NumPr, Orientation, PageSize, Paragraph, ParagraphBorders, ParagraphSpacing, PartRef,
    PositionH, PositionV, RFonts, RawPPr, RawRPr, RelFromH, RelFromV, Relationships, Row,
    RowHeight, Run, RunContent, Section, SectionProperties, SectionType, Shading, ShadingPattern,
    StyleId, TabLeader, TabStop, TabStopKind, Table, TableBorders, TableLayout, TableLook,
    TableWidth, Toggle, Twips, Underline, VMerge, VertAlign, WrapKind,
};
use crate::xml::{
    attr_f32, attr_i32, attr_toggle, attr_u32, attributes, capture_element, find, is_true,
    local_name, resolve_alternate_content, resolve_reference, wrap_fragment, AlternateContent,
    Attr,
};

/// Путь до тела — основа `xml_path` в предупреждениях.
const BODY_PATH: &str = "w:document/w:body";

/// Имя синтетического корня, которым [`wrap_fragment`] оборачивает фрагмент `mc:Choice`.
const FRAGMENT_ROOT: &[u8] = b"docx-fragment";

/// Предел `w:ilvl`: уровней списка в `WordprocessingML` девять (0..=8).
const MAX_ILVL: u8 = 8;

/// Предел `w:outlineLvl`: уровней структуры десять (0..=9).
const MAX_OUTLINE_LVL: u8 = 9;

// ---------------------------------------------------------------------------
// Точки входа
// ---------------------------------------------------------------------------

/// Разобрать блочное содержимое: `w:body`, колонтитул, сноску, комментарий.
///
/// Останавливается на `End` объемлющего элемента. `xml_path` — для warning'ов
/// (`w:document/w:body`).
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше `End` или байты события не
/// UTF-8; [`Error::TooManyWarnings`] — предупреждений стало больше порога.
pub(crate) fn parse_blocks(
    reader: &mut XmlReader<'_>,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Vec<BlockItem>> {
    let mut items = Vec::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside block content",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(items),
            _ => continue,
        };
        parse_block_into(
            reader, element, empty, rels, ctx, part, xml_path, &mut items,
        )?;
    }
}

/// Разобрать один блочный элемент и дописать его в `out`.
///
/// Вынесено из [`parse_blocks`], чтобы тем же диспетчером разбирать содержимое
/// ячейки таблицы (`w:tc`): её `w:tcPr` читается отдельно, а всё остальное —
/// те же блоки, что и в теле, вплоть до вложенных таблиц.
///
/// # Errors
/// То же, что у [`parse_blocks`].
// Аргументов больше порога, чем у соседних разборщиков: к их набору добавляются
// связи части — они нужны таблицам. Собирать их в структуру значило бы завести
// ещё один вид «контекста» рядом с `ParseCtx` и переписать все вызовы.
#[allow(clippy::too_many_arguments)]
fn parse_block_into(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    out: &mut Vec<BlockItem>,
) -> Result<()> {
    match local_name(element.name().into_inner()) {
        b"p" => {
            let paragraph = if empty {
                Paragraph {
                    id: ctx.id(),
                    ..Paragraph::default()
                }
            } else {
                parse_paragraph(reader, rels, ctx, part, xml_path)?
            };
            out.push(BlockItem::Paragraph(paragraph));
        }
        b"tbl" => out.push(BlockItem::Table(parse_table(
            reader, empty, rels, ctx, part, xml_path,
        )?)),
        b"sectPr" => out.push(BlockItem::SectPr(parse_sect_pr(
            reader, empty, rels, ctx, part, xml_path,
        )?)),
        b"AlternateContent" if !empty => {
            // Ветка `mc:Choice` разворачивается на месте блока, как будто
            // её содержимое и было телом (ADR-0014 §1).
            match resolve_alternate_content(reader, element, ctx, part, xml_path)? {
                AlternateContent::Choice(fragment) => {
                    out.extend(parse_block_fragment(&fragment, rels, ctx, part, xml_path)?);
                }
                AlternateContent::Unsupported(xml) => {
                    ctx.warn_at(
                        WarningKind::UnknownElement,
                        part,
                        Some(xml_path),
                        "`mc:AlternateContent` has no supported `mc:Choice`, kept as unknown",
                    )?;
                    out.push(BlockItem::Unknown { id: ctx.id(), xml });
                }
            }
        }
        _ => {
            let id = ctx.id();
            let xml = capture_any(reader, element, empty, ctx, part)?;
            ctx.warn_at(
                WarningKind::UnknownElement,
                part,
                Some(xml_path),
                format!(
                    "`{}` is not supported, kept as unknown",
                    element_name(element)
                ),
            )?;
            out.push(BlockItem::Unknown { id, xml });
        }
    }
    Ok(())
}

/// Разобрать `word/document.xml`: `w:document/w:body` → [`Body`].
///
/// # Errors
/// [`Error::malformed`] — в части нет `w:document/w:body` или поток оборвался;
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
pub(crate) fn parse(
    body: &[u8],
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Body> {
    let mut reader = XmlReader::preserving(body, part);
    let mut in_document = false;
    while let Some(event) = reader.next_significant()? {
        let Event::Start(start) = event else {
            continue;
        };
        match local_name(start.name().into_inner()) {
            b"document" => in_document = true,
            b"body" if in_document => {
                let id = ctx.id();
                let items = parse_blocks(&mut reader, rels, ctx, part, BODY_PATH)?;
                // Секции собираются после разбора: конец секции — это `w:sectPr`,
                // и порядок концов в теле и есть порядок секций (план §3.1).
                let sections = collect_sections(&items, ctx);
                return Ok(Body {
                    id,
                    items,
                    sections,
                });
            }
            _ => {}
        }
    }
    Err(Error::malformed(part, "`w:document/w:body` is missing"))
}

// ---------------------------------------------------------------------------
// Секции
// ---------------------------------------------------------------------------

/// Умолчание `w:pgSz` по ширине: письмо 8,5" (ECMA-376 §17.6.13).
const DEFAULT_PAGE_WIDTH: i32 = 12240;

/// Умолчание `w:pgSz` по высоте: письмо 11".
const DEFAULT_PAGE_HEIGHT: i32 = 15840;

/// Умолчание `w:pgMar`: поля в один дюйм (§17.6.11).
const DEFAULT_MARGIN: i32 = 1440;

/// Умолчание `w:pgMar/@w:header` и `@w:footer` — полдюйма.
const DEFAULT_HEADER_MARGIN: i32 = 720;

/// Умолчание `w:cols/@w:space` — полдюйма (§17.6.4).
const DEFAULT_COLUMN_SPACE: i32 = 720;

/// Свойства секции по умолчанию — то, что `WordprocessingML` подставляет молча.
///
/// `w:sectPr` вправе не назвать ни одного свойства: секция без `w:pgSz` — это
/// письмо с полями в дюйм, а не страница нулевого размера.
#[must_use]
fn default_section_properties() -> SectionProperties {
    SectionProperties {
        page_size: PageSize {
            width: Twips::new(DEFAULT_PAGE_WIDTH),
            height: Twips::new(DEFAULT_PAGE_HEIGHT),
        },
        orientation: Orientation::Portrait,
        margins: Margins {
            top: Twips::new(DEFAULT_MARGIN),
            right: Twips::new(DEFAULT_MARGIN),
            bottom: Twips::new(DEFAULT_MARGIN),
            left: Twips::new(DEFAULT_MARGIN),
            header: Some(Twips::new(DEFAULT_HEADER_MARGIN)),
            footer: Some(Twips::new(DEFAULT_HEADER_MARGIN)),
            gutter: Some(Twips::new(0)),
        },
        columns: Columns {
            count: 1,
            space: Twips::new(DEFAULT_COLUMN_SPACE),
            equal_width: true,
            separator: false,
            defs: Vec::new(),
        },
        title_pg: false,
        header_default: None,
        header_first: None,
        header_even: None,
        footer_default: None,
        footer_first: None,
        footer_even: None,
        section_type: None,
        unknown: Vec::new(),
    }
}

/// Собрать секции тела: по одной на каждый `w:sectPr` в порядке концов.
///
/// Конец секции — `w:sectPr`: у последней секции он завершает `w:body`, у
/// предыдущих стоит в `w:pPr` абзаца, которым секция кончается. Секция без
/// явного `w:sectPr` не заводится: её свойств в файле просто нет.
fn collect_sections(items: &[BlockItem], ctx: &mut ParseCtx) -> Vec<Section> {
    let mut sections = Vec::new();
    for item in items {
        let properties = match item {
            BlockItem::Paragraph(paragraph) => paragraph.section_break.as_deref(),
            BlockItem::SectPr(properties) => Some(properties),
            _ => None,
        };
        if let Some(properties) = properties {
            sections.push(section_from(properties.clone(), ctx));
        }
    }
    sections
}

/// Секция из разобранных свойств: удобные поля повторяют сырые (план §3.1).
fn section_from(properties: SectionProperties, ctx: &mut ParseCtx) -> Section {
    Section {
        id: ctx.id(),
        header_default: properties.header_default.clone(),
        header_first: properties.header_first.clone(),
        header_even: properties.header_even.clone(),
        footer_default: properties.footer_default.clone(),
        footer_first: properties.footer_first.clone(),
        footer_even: properties.footer_even.clone(),
        title_pg: properties.title_pg,
        page_size: properties.page_size.clone(),
        orientation: properties.orientation,
        margins: properties.margins.clone(),
        columns: properties.columns.clone(),
        properties,
    }
}

/// Разобрать `w:sectPr` — свойства секции.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_sect_pr(
    reader: &mut XmlReader<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<SectionProperties> {
    let mut properties = default_section_properties();
    if empty {
        return Ok(properties);
    }
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:sectPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(properties),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"pgSz" => {
                if let Some(width) = attr_i32(&attrs, "w", ctx, part)? {
                    properties.page_size.width = Twips::new(width);
                }
                if let Some(height) = attr_i32(&attrs, "h", ctx, part)? {
                    properties.page_size.height = Twips::new(height);
                }
                properties.orientation = parse_orientation(&attrs, ctx, part)?;
            }
            b"pgMar" => apply_page_margins(&attrs, &mut properties.margins, ctx, part)?,
            b"cols" => {
                if let Some(count) = attr_u32(&attrs, "num", ctx, part)? {
                    properties.columns.count = count;
                }
                if let Some(space) = attr_i32(&attrs, "space", ctx, part)? {
                    properties.columns.space = Twips::new(space);
                }
                // `w:equalWidth` и `w:sep` — атрибуты `ST_OnOff`, а не элементы
                // с `w:val`: `attr_toggle` их не читает.
                properties.columns.equal_width = attr_on(&attrs, "equalWidth", true);
                properties.columns.separator = attr_on(&attrs, "sep", false);
                if !empty {
                    properties.columns.defs = parse_columns(reader, ctx, part, xml_path)?;
                }
            }
            b"titlePg" => {
                properties.title_pg = toggle_is_on(attr_toggle(&attrs, ctx, part, "w:titlePg")?);
            }
            b"headerReference" | b"footerReference" => {
                let is_header = local_name(element.name().into_inner()) == b"headerReference";
                let reference = parse_part_ref(&attrs, rels, ctx, part, &element_name(element))?;
                let Some(reference) = reference else {
                    continue;
                };
                let kind = find(&attrs, "type").unwrap_or("default");
                let slot = match (is_header, kind) {
                    (true, "default") => &mut properties.header_default,
                    (true, "first") => &mut properties.header_first,
                    (true, "even") => &mut properties.header_even,
                    (false, "default") => &mut properties.footer_default,
                    (false, "first") => &mut properties.footer_first,
                    (false, "even") => &mut properties.footer_even,
                    _ => {
                        ctx.warn(
                            WarningKind::InvalidAttribute,
                            part,
                            format!(
                                "`w:type` `{kind}` of a header/footer reference is unknown, ignored"
                            ),
                        )?;
                        continue;
                    }
                };
                *slot = Some(reference);
            }
            b"type" => {
                properties.section_type = parse_section_type(find(&attrs, "val"), ctx, part)?;
            }
            b"docGrid" => {
                // `w:docGrid` Word пишет почти в каждый документ, а модели он
                // ничего не даёт: предупреждать о нём значило бы шуметь на
                // целом классе файлов. XML сохраняется в `unknown`.
                let xml = capture_any(reader, element, empty, ctx, part)?;
                properties.unknown.push((local_name_of(element), xml));
            }
            _ => {
                let xml = capture_any(reader, element, empty, ctx, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:sectPr` is not supported, kept as unknown",
                        element_name(element)
                    ),
                )?;
                properties.unknown.push((local_name_of(element), xml));
            }
        }
    }
}

/// Дописать поля страницы из `w:pgMar`.
///
/// Незаданное поле остаётся умолчанием схемы: `w:pgMar` вправе назвать не все.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn apply_page_margins(
    attrs: &[Attr<'_>],
    margins: &mut Margins,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<()> {
    for (name, slot) in [
        ("top", &mut margins.top),
        ("right", &mut margins.right),
        ("bottom", &mut margins.bottom),
        ("left", &mut margins.left),
    ] {
        if let Some(value) = attr_i32(attrs, name, ctx, part)? {
            *slot = Twips::new(value);
        }
    }
    for (name, slot) in [
        ("header", &mut margins.header),
        ("footer", &mut margins.footer),
        ("gutter", &mut margins.gutter),
    ] {
        if let Some(value) = attr_i32(attrs, name, ctx, part)? {
            *slot = Some(Twips::new(value));
        }
    }
    Ok(())
}

/// Разобрать `w:cols`: явные ширины колонок (`w:col`).
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_columns(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Vec<ColumnDef>> {
    let mut defs = Vec::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:cols`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(defs),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        if local_name(element.name().into_inner()) == b"col" {
            defs.push(ColumnDef {
                width: Twips::new(attr_i32(&attrs, "w", ctx, part)?.unwrap_or(0)),
                space: Twips::new(attr_i32(&attrs, "space", ctx, part)?.unwrap_or(0)),
            });
            continue;
        }
        capture_any(reader, element, empty, ctx, part)?;
        ctx.warn_at(
            WarningKind::UnknownElement,
            part,
            Some(xml_path),
            format!(
                "`{}` in `w:cols` is not supported, ignored",
                element_name(element)
            ),
        )?;
    }
}

/// Ссылка на колонтитул: `r:id` → часть пакета через связи.
///
/// # Errors
/// То же, что у [`parse_blocks`]. Отсутствие связи — предупреждение, а не отказ:
/// часть со ссылкой на потерянный колонтитул теряет только колонтитул.
fn parse_part_ref(
    attrs: &[Attr<'_>],
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    element: &str,
) -> Result<Option<PartRef>> {
    let Some(rel_id) = find(attrs, "id") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("`{element}` has no `r:id`, ignored"),
        )?;
        return Ok(None);
    };
    let Some(relation) = rels.get(rel_id) else {
        ctx.warn(
            WarningKind::MissingPart,
            part,
            format!("`{element}`: relationship `{rel_id}` is not declared, ignored"),
        )?;
        return Ok(None);
    };
    let external = relation.target_mode.as_deref() == Some("External");
    // Цель разрешается от самой части: `word/header1.xml`, а не `header1.xml`.
    let target = relation
        .part(part)
        .unwrap_or_else(|| relation.target.clone());
    Ok(Some(PartRef {
        rel_id: rel_id.to_owned(),
        target,
        external,
    }))
}

/// Ориентация страницы (`w:pgSz/@w:orient`).
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_orientation(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Orientation> {
    match find(attrs, "orient") {
        None | Some("portrait") => Ok(Orientation::Portrait),
        Some("landscape") => Ok(Orientation::Landscape),
        Some(other) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:pgSz`: `w:orient` `{other}` is unknown, the page counts as portrait"),
            )?;
            Ok(Orientation::Portrait)
        }
    }
}

/// Вид разрыва перед секцией (`w:type`, `ST_SectionMark`).
///
/// `nextColumn` в модели не выражен: колонок в ней пока нет, и секция с ним
/// считается начинающейся с новой страницы — как и умолчание.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_section_type(
    raw: Option<&str>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<SectionType>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let kind = match raw {
        "nextPage" => SectionType::NextPage,
        "continuous" => SectionType::Continuous,
        "evenPage" => SectionType::EvenPage,
        "oddPage" => SectionType::OddPage,
        _ => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:type`: `{raw}` is unknown, the section starts on a new page"),
            )?;
            return Ok(None);
        }
    };
    Ok(Some(kind))
}

/// Логический атрибут `ST_OnOff` (`w:cols/@w:equalWidth`, `@w:sep`).
///
/// Отсутствие атрибута — умолчание схемы, а не `false`: у `w:equalWidth` оно `true`.
fn attr_on(attrs: &[Attr<'_>], name: &str, default: bool) -> bool {
    find(attrs, name).map_or(default, is_true)
}

// ---------------------------------------------------------------------------
// Блоки и абзацы
// ---------------------------------------------------------------------------

/// Разобрать фрагмент `mc:Choice` как последовательность блоков.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_block_fragment(
    fragment: &str,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Vec<BlockItem>> {
    let wrapped = wrap_fragment(fragment);
    let mut reader = XmlReader::preserving(wrapped.as_bytes(), part);
    // Единственный `End` фрагмента — `End` синтетического корня: на нём и
    // остановится `parse_blocks`, а `End` самого `mc:AlternateContent` уже
    // вычитан `resolve_alternate_content`.
    expect_fragment_root(&mut reader, part)?;
    parse_blocks(&mut reader, rels, ctx, part, xml_path)
}

/// Разобрать `w:p`: свойства, знак абзаца и inline-содержимое.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_paragraph(
    reader: &mut XmlReader<'_>,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Paragraph> {
    let id = ctx.id();
    let mut ppr = RawPPr::default();
    let mut section_break = None;
    let mut runs = Vec::new();
    // Поле собирается из соседних run'ов, а не внутри одного: состояние живёт
    // в цикле абзаца, пока он читает эти run'ы.
    let mut state = InlineState::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:p`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => break,
            _ => continue,
        };
        match local_name(element.name().into_inner()) {
            b"pPr" => {
                let (parsed, properties) = if empty {
                    (RawPPr::default(), None)
                } else {
                    parse_ppr(reader, rels, ctx, part, xml_path)?
                };
                ppr = parsed;
                section_break = properties;
            }
            _ => {
                parse_inline_step(
                    reader, element, empty, rels, ctx, part, xml_path, 0, &mut state, &mut runs,
                )?;
            }
        }
    }
    state.finish(ctx, part, xml_path, &mut runs)?;
    // `mark_rpr` — свойства знака абзаца: к runs они не применяются (ADR-0013 §2).
    let mark_rpr = ppr.r_pr.clone().unwrap_or_default();
    let style_ref = ppr.style.clone();
    let numbering_ref = ppr.num_pr.as_ref().and_then(|num| num.num_id);
    Ok(Paragraph {
        id,
        ppr: Box::new(ppr),
        mark_rpr: Box::new(mark_rpr),
        runs,
        style_ref,
        numbering_ref,
        section_break: section_break.map(Box::new),
    })
}

// ---------------------------------------------------------------------------
// Таблицы
// ---------------------------------------------------------------------------

/// Собранные части таблицы (`w:tbl`).
///
/// Части копятся в структуру, а не в локальные переменные: `w:tblPr` и
/// `w:tblGrid` приходят до строк, но в ветке `mc:Choice` могут прийти и внутри
/// неё, а обходить `mc:AlternateContent` дважды не хочется.
struct TableParts {
    /// Стиль таблицы (`w:tblPr/w:tblStyle`).
    style_ref: Option<StyleId>,
    /// Сетка столбцов (`w:tblGrid`).
    grid: Vec<GridCol>,
    /// Строки (`w:tr`).
    rows: Vec<Row>,
    /// Раскладка (`w:tblPr/w:tblLayout`).
    layout: TableLayout,
    /// Предпочтительная ширина (`w:tblPr/w:tblW`).
    width: Option<TableWidth>,
    /// Границы таблицы (`w:tblPr/w:tblBorders`).
    borders: TableBorders,
    /// Признаки оформления (`w:tblPr/w:tblLook`).
    look: TableLook,
    /// Выравнивание таблицы (`w:tblPr/w:jc`).
    jc: Option<Justification>,
    /// Отступ таблицы от поля (`w:tblPr/w:tblInd`).
    indent: Option<Twips>,
    /// Умолчания полей ячеек (`w:tblPr/w:tblCellMar`).
    cell_margins: CellMargins,
}

impl Default for TableParts {
    fn default() -> Self {
        Self {
            style_ref: None,
            grid: Vec::new(),
            rows: Vec::new(),
            // `w:tblLayout` нет — WordprocessingML предписывает `autofit`.
            layout: TableLayout::Autofit,
            width: None,
            borders: TableBorders::default(),
            look: TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: CellMargins::default(),
        }
    }
}

/// Разобрать `w:tbl`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_table(
    reader: &mut XmlReader<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Table> {
    // ID выделяется до разбора содержимого: узлы нумеруются в порядке документа,
    // и таблица в нём раньше своих строк.
    let id = ctx.id();
    let mut parts = TableParts::default();
    if !empty {
        parse_table_children(reader, rels, ctx, part, xml_path, &mut parts)?;
    }
    Ok(Table {
        id,
        style_ref: parts.style_ref,
        grid: parts.grid,
        rows: parts.rows,
        layout: parts.layout,
        width: parts.width,
        borders: parts.borders,
        look: parts.look,
        jc: parts.jc,
        indent: parts.indent,
        cell_margins: parts.cell_margins,
    })
}

/// Разобрать детей `w:tbl` до его `End` — или до `End` фрагмента `mc:Choice`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_table_children(
    reader: &mut XmlReader<'_>,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    parts: &mut TableParts,
) -> Result<()> {
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tbl`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(()),
            _ => continue,
        };
        match local_name(element.name().into_inner()) {
            b"tblPr" => {
                if !empty {
                    parse_tbl_pr(reader, ctx, part, xml_path, parts)?;
                }
            }
            b"tblGrid" => {
                if !empty {
                    parts.grid = parse_tbl_grid(reader, ctx, part, xml_path)?;
                }
            }
            b"tr" => parts
                .rows
                .push(parse_row(reader, empty, rels, ctx, part, xml_path)?),
            b"AlternateContent" if !empty => {
                match resolve_alternate_content(reader, element, ctx, part, xml_path)? {
                    AlternateContent::Choice(fragment) => {
                        // Дети ветки — те же `w:tblPr`, `w:tblGrid` и `w:tr`,
                        // поэтому разбор идёт тем же накопителем.
                        let wrapped = wrap_fragment(&fragment);
                        let mut fragment_reader = XmlReader::preserving(wrapped.as_bytes(), part);
                        expect_fragment_root(&mut fragment_reader, part)?;
                        parse_table_children(
                            &mut fragment_reader,
                            rels,
                            ctx,
                            part,
                            xml_path,
                            parts,
                        )?;
                    }
                    // Таблица не хранит XML неизвестных детей: ветку негде
                    // сохранить, остаётся предупреждение.
                    AlternateContent::Unsupported(_) => {
                        ctx.warn_at(
                            WarningKind::UnknownElement,
                            part,
                            Some(xml_path),
                            "`mc:AlternateContent` has no supported `mc:Choice`, ignored",
                        )?;
                    }
                }
            }
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:tbl` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:tblPr`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_tbl_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    parts: &mut TableParts,
) -> Result<()> {
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tblPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(()),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"tblStyle" => parts.style_ref = find(&attrs, "val").map(StyleId::new),
            b"tblLayout" => parts.layout = parse_table_layout(&attrs, ctx, part)?,
            b"tblW" => parts.width = parse_table_width(&attrs, ctx, part)?,
            b"tblBorders" => {
                if !empty {
                    parts.borders = parse_table_borders(reader, ctx, part, xml_path)?;
                }
            }
            b"tblLook" => parts.look = parse_table_look(&attrs),
            b"jc" => parts.jc = parse_justification(&attrs, ctx, part)?,
            b"tblInd" => parts.indent = attr_i32(&attrs, "w", ctx, part)?.map(Twips::new),
            b"tblCellMar" => {
                if !empty {
                    parts.cell_margins = parse_cell_margins(reader, ctx, part, xml_path)?;
                }
            }
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:tblPr` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:tblGrid`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_tbl_grid(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Vec<GridCol>> {
    let mut grid = Vec::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tblGrid`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(grid),
            _ => continue,
        };
        if local_name(element.name().into_inner()) == b"gridCol" {
            let attrs = attributes(element, part)?;
            let width = attr_i32(&attrs, "w", ctx, part)?.unwrap_or_default();
            grid.push(GridCol {
                width: Twips::new(width),
            });
        } else {
            skip_element(reader, empty, part)?;
            ctx.warn_at(
                WarningKind::UnknownElement,
                part,
                Some(xml_path),
                format!(
                    "`{}` in `w:tblGrid` is not supported, ignored",
                    element_name(element)
                ),
            )?;
        }
    }
}

/// Разобрать `w:tr`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_row(
    reader: &mut XmlReader<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Row> {
    let mut row = Row {
        id: ctx.id(),
        cells: Vec::new(),
        height: None,
        cant_split: false,
        header: false,
    };
    if empty {
        return Ok(row);
    }
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(row),
            _ => continue,
        };
        match local_name(element.name().into_inner()) {
            b"trPr" => {
                if !empty {
                    parse_tr_pr(reader, ctx, part, xml_path, &mut row)?;
                }
            }
            b"tc" => row
                .cells
                .push(parse_cell(reader, empty, rels, ctx, part, xml_path)?),
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:tr` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:trPr`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_tr_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    row: &mut Row,
) -> Result<()> {
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:trPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(()),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"trHeight" => row.height = parse_row_height(&attrs, ctx, part)?,
            b"cantSplit" => {
                row.cant_split = toggle_is_on(attr_toggle(&attrs, ctx, part, "w:cantSplit")?);
            }
            b"tblHeader" => {
                row.header = toggle_is_on(attr_toggle(&attrs, ctx, part, "w:tblHeader")?);
            }
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:trPr` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:tc`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_cell(
    reader: &mut XmlReader<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Cell> {
    let mut cell = Cell {
        id: ctx.id(),
        grid_span: 1,
        v_merge: None,
        width: None,
        margins: CellMargins::default(),
        v_align: CellVAlign::Top,
        borders: Box::default(),
        shading: None,
        items: Vec::new(),
    };
    if empty {
        return Ok(cell);
    }
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tc`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(cell),
            _ => continue,
        };
        if local_name(element.name().into_inner()) == b"tcPr" {
            if !empty {
                parse_tc_pr(reader, ctx, part, xml_path, &mut cell)?;
            }
        } else {
            // Содержимое ячейки — те же блоки, что и в теле, вплоть до
            // вложенной таблицы; `NodeId` для них общий с телом (см. `parse_block_into`).
            parse_block_into(
                reader,
                element,
                empty,
                rels,
                ctx,
                part,
                xml_path,
                &mut cell.items,
            )?;
        }
    }
}

/// Разобрать `w:tcPr`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_tc_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    cell: &mut Cell,
) -> Result<()> {
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tcPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(()),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"gridSpan" => {
                if let Some(span) = attr_u32(&attrs, "val", ctx, part)? {
                    if span == 0 {
                        ctx.warn(
                            WarningKind::InvalidAttribute,
                            part,
                            "`w:gridSpan` is zero, counted as one",
                        )?;
                    } else {
                        cell.grid_span = span;
                    }
                }
            }
            b"vMerge" => cell.v_merge = Some(parse_v_merge(&attrs, ctx, part)?),
            b"tcW" => cell.width = parse_cell_width(&attrs, ctx, part)?,
            b"tcMar" => {
                if !empty {
                    cell.margins = parse_cell_margins(reader, ctx, part, xml_path)?;
                }
            }
            b"vAlign" => cell.v_align = parse_cell_v_align(&attrs, ctx, part)?,
            b"tcBorders" => {
                if !empty {
                    *cell.borders = parse_cell_borders(reader, ctx, part, xml_path)?;
                }
            }
            b"shd" => cell.shading = Some(parse_shading(&attrs, ctx, part)?),
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:tcPr` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// `w:vMerge`: `restart` начинает объединение, всё прочее — продолжает.
///
/// У `w:vMerge` без `w:val` умолчание схемы — `continue` (`ST_Merge`,
/// ECMA-376 §17.4.85): так Word и пишет продолжение объединения, оставляя
/// `w:val="restart"` только первой ячейке области.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_v_merge(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<VMerge> {
    match find(attrs, "val") {
        Some("restart") => Ok(VMerge::Restart),
        None | Some("continue") => Ok(VMerge::Continue),
        Some(other) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:vMerge`: unknown `w:val` `{other}`, counted as `continue`"),
            )?;
            Ok(VMerge::Continue)
        }
    }
}

/// Раскладка из `w:tblLayout/@w:type`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_table_layout(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<TableLayout> {
    match find(attrs, "type") {
        None | Some("autofit") => Ok(TableLayout::Autofit),
        Some("fixed") => Ok(TableLayout::Fixed),
        Some(other) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:tblLayout`: unknown `w:type` `{other}`, counted as `autofit`"),
            )?;
            Ok(TableLayout::Autofit)
        }
    }
}

/// Ширина из `w:tblW`.
///
/// `w:pct` записан в пятидесятых долях процента (`ST_MeasurementOrPercent`),
/// поэтому модель получает проценты: `5000` — это `100.0`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_table_width(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<TableWidth>> {
    match find(attrs, "type") {
        // Без `w:type` значение читается как twips: так пишет Word для `dxa`.
        None | Some("dxa") => {
            Ok(attr_i32(attrs, "w", ctx, part)?.map(|w| TableWidth::Dxa(Twips::new(w))))
        }
        Some("auto") => Ok(Some(TableWidth::Auto)),
        Some("pct") => Ok(attr_f32(attrs, "w", ctx, part)?.map(|pct| TableWidth::Pct(pct / 50.0))),
        Some(other) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:tblW`: unknown `w:type` `{other}`, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Ширина из `w:tcW`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_cell_width(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<CellWidth>> {
    match find(attrs, "type") {
        Some("nil") => Ok(Some(CellWidth::Nil)),
        Some("pct") => Ok(attr_f32(attrs, "w", ctx, part)?.map(|pct| CellWidth::Pct(pct / 50.0))),
        None | Some("dxa") => {
            Ok(attr_i32(attrs, "w", ctx, part)?.map(|w| CellWidth::Dxa(Twips::new(w))))
        }
        Some(other) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:tcW`: unknown `w:type` `{other}`, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Высота строки из `w:trHeight`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_row_height(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<RowHeight>> {
    let Some(value) = attr_i32(attrs, "val", ctx, part)? else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:trHeight` has no `w:val`, ignored",
        )?;
        return Ok(None);
    };
    let rule = match find(attrs, "hRule") {
        None | Some("auto") => HeightRule::Auto,
        Some("atLeast") => HeightRule::AtLeast,
        Some("exact") => HeightRule::Exact,
        Some(other) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:trHeight`: unknown `w:hRule` `{other}`, counted as `auto`"),
            )?;
            HeightRule::Auto
        }
    };
    Ok(Some(RowHeight {
        value: Twips::new(value),
        rule,
    }))
}

/// Вертикальное выравнивание ячейки из `w:vAlign`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_cell_v_align(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<CellVAlign> {
    let Some(raw) = find(attrs, "val") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:vAlign` has no `w:val`, counted as `top`",
        )?;
        return Ok(CellVAlign::Top);
    };
    Ok(match raw {
        "top" => CellVAlign::Top,
        "center" => CellVAlign::Center,
        "bottom" => CellVAlign::Bottom,
        other => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:vAlign`: unknown `w:val` `{other}`, counted as `top`"),
            )?;
            CellVAlign::Top
        }
    })
}

/// Признаки оформления из `w:tblLook`.
///
/// Современный Word пишет их атрибутами (`w:firstRow="1"`), старый — битовой
/// маской `ST_TblLook` в `w:val` (`04A0`); поддержаны оба вида, атрибуты важнее.
#[must_use]
fn parse_table_look(attrs: &[Attr<'_>]) -> TableLook {
    let mask = find(attrs, "val")
        .and_then(|raw| u32::from_str_radix(raw.trim(), 16).ok())
        .unwrap_or(0);
    let flag = |name: &str, bit: u32| match find(attrs, name) {
        Some(value) => is_true(value),
        None => mask & bit != 0,
    };
    TableLook {
        first_row: flag("firstRow", 0x0020),
        last_row: flag("lastRow", 0x0040),
        first_column: flag("firstColumn", 0x0080),
        last_column: flag("lastColumn", 0x0100),
        no_h_band: flag("noHBand", 0x0200),
        no_v_band: flag("noVBand", 0x0400),
    }
}

/// Разобрать `w:tblBorders`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_table_borders(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<TableBorders> {
    let mut borders = TableBorders::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tblBorders`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(borders),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"top" => borders.top = parse_border(&attrs, ctx, part)?,
            b"left" | b"start" => borders.left = parse_border(&attrs, ctx, part)?,
            b"bottom" => borders.bottom = parse_border(&attrs, ctx, part)?,
            b"right" | b"end" => borders.right = parse_border(&attrs, ctx, part)?,
            b"insideH" => borders.inside_h = parse_border(&attrs, ctx, part)?,
            b"insideV" => borders.inside_v = parse_border(&attrs, ctx, part)?,
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:tblBorders` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:tcBorders`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_cell_borders(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<CellBorders> {
    let mut borders = CellBorders::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tcBorders`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(borders),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"top" => borders.top = parse_border(&attrs, ctx, part)?,
            b"left" | b"start" => borders.left = parse_border(&attrs, ctx, part)?,
            b"bottom" => borders.bottom = parse_border(&attrs, ctx, part)?,
            b"right" | b"end" => borders.right = parse_border(&attrs, ctx, part)?,
            // Диагоналей и границ `insideH`/`insideV` в модели нет: у ячейки
            // они не выражаются, а молча терять их нельзя.
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:tcBorders` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:tblCellMar` и `w:tcMar`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_cell_margins(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<CellMargins> {
    let mut margins = CellMargins::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside cell margins",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(margins),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        let value = attr_i32(&attrs, "w", ctx, part)?.map(Twips::new);
        match local_name(element.name().into_inner()) {
            b"top" => margins.top = value,
            b"left" | b"start" => margins.left = value,
            b"bottom" => margins.bottom = value,
            b"right" | b"end" => margins.right = value,
            _ => {
                skip_element(reader, empty, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in cell margins is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Тумблер как булево: `On` и `Inherit` — «да», `Off` — явное «нет».
#[must_use]
fn toggle_is_on(toggle: Option<Toggle>) -> bool {
    !matches!(toggle, Some(Toggle::Off))
}

// ---------------------------------------------------------------------------
// Inline-содержимое абзаца
// ---------------------------------------------------------------------------

/// Предел вложенности inline-контейнеров (`w:hyperlink`, `w:fldSimple`).
///
/// Вложенность задаётся входом, а разбор рекурсивный: без предела патологический
/// документ уронил бы разбор стеком.
const MAX_INLINE_DEPTH: usize = 16;

/// Вид `w:fldChar` (`w:fldCharType`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum FieldMark {
    /// `begin`: начало составного поля.
    Begin,
    /// `separate`: дальше идёт результат поля.
    Separated,
    /// `end`: конец составного поля.
    End,
}

/// Вид `w:fldChar` по `w:fldCharType`; незнакомое значение — не разметка поля.
fn parse_field_mark(raw: &str) -> Option<FieldMark> {
    match raw {
        "begin" => Some(FieldMark::Begin),
        "separate" => Some(FieldMark::Separated),
        "end" => Some(FieldMark::End),
        _ => None,
    }
}

/// Найденное в run'е для составного поля.
///
/// `w:fldChar` и `w:instrText` в `RunContent` не выражаются, а поле собирается
/// из нескольких соседних run'ов — поэтому разбор run'а отдаёт найденное наружу.
#[derive(Default)]
struct FieldParts {
    /// Первый (и обычно единственный) `w:fldChar` в run'е.
    mark: Option<FieldMark>,
    /// Текст `w:instrText`, склеенный по всем вхождениям в run.
    instruction: String,
    /// XML фрагментов поля: если поле не закроется, они возвращаются в run'ы
    /// как [`RunContent::Unknown`], чтобы не пропали.
    stray: Vec<(NodeId, String)>,
    /// `w:fldChar/@w:dirty`: результат поля устарел.
    dirty: bool,
}

/// Run внутри собираемого поля вместе с его разметкой поля.
struct FieldRun {
    /// Сам run: его содержимое — часть результата поля.
    run: Run,
    /// XML `w:fldChar`/`w:instrText` этого run'а — на случай, если поле не закроется.
    stray: Vec<(NodeId, String)>,
}

/// Элемент собираемого поля по порядку.
///
/// Оба варианта — боксы: сам [`Inline`] велик, а элементов в поле много.
enum FieldItem {
    /// Run поля.
    Run(Box<FieldRun>),
    /// Прочее inline-содержимое результата (`w:hyperlink`, закладка, рисунок).
    Inline(Box<Inline>),
}

/// Сборка составного поля: `begin` … `instrText` … `separate` … `end`.
///
/// Поле живёт не в одном элементе, а в последовательности соседних run'ов, и
/// конца может не прийти вовсе. Поэтому содержимое копится здесь, а не в run'е:
/// несостоявшееся поле возвращается в поток теми же run'ами, а его разметка —
/// как [`RunContent::Unknown`].
struct FieldBuilder {
    /// Идентификатор узла будущего поля.
    id: NodeId,
    /// Инструкция поля из `w:instrText`.
    instruction: String,
    /// Встретился `w:fldChar w:fldCharType="end"`.
    closed: bool,
    /// `w:fldChar/@w:dirty`: результат поля устарел.
    dirty: bool,
    /// Содержимое поля по порядку.
    items: Vec<FieldItem>,
}

impl FieldBuilder {
    /// Поле, начатое run'ом с `w:fldChar w:fldCharType="begin"`.
    fn new(id: NodeId, run: Run, parts: FieldParts) -> Self {
        Self {
            id,
            instruction: parts.instruction,
            closed: false,
            dirty: parts.dirty,
            items: vec![FieldItem::Run(Box::new(FieldRun {
                run,
                stray: parts.stray,
            }))],
        }
    }

    /// Дописать run поля.
    fn push_run(&mut self, run: Run, parts: FieldParts) {
        match parts.mark {
            // Вложенное поле модель не выражает: его разметка остаётся в `stray`.
            Some(FieldMark::End) => self.closed = true,
            Some(FieldMark::Begin | FieldMark::Separated) => {}
            None => self.instruction.push_str(&parts.instruction),
        }
        self.items.push(FieldItem::Run(Box::new(FieldRun {
            run,
            stray: parts.stray,
        })));
    }

    /// Собранное поле: содержимое результата, без разметки поля.
    fn finish(self) -> Field {
        let mut result = Vec::new();
        for item in self.items {
            match item {
                FieldItem::Inline(inline) => result.push(*inline),
                FieldItem::Run(field_run) if !field_run.run.content.is_empty() => {
                    result.push(Inline::Run(field_run.run));
                }
                FieldItem::Run(_) => {}
            }
        }
        Field {
            id: self.id,
            kind: FieldKind::Complex,
            instruction: self.instruction,
            result,
            dirty: self.dirty,
        }
    }

    /// Вернуть несостоявшееся поле в поток как обычное содержимое.
    fn abandon(self, out: &mut Vec<Inline>) {
        for item in self.items {
            match item {
                FieldItem::Inline(inline) => out.push(*inline),
                FieldItem::Run(field_run) => {
                    out.push(Inline::Run(reinsert_strays(field_run.run, field_run.stray)));
                }
            }
        }
    }
}

/// Состояние inline-содержимого: собираемое составное поле, если оно есть.
///
/// Живёт в том цикле, который читает соседние элементы абзаца: конец поля —
/// такой же соседний элемент, а `End` объемлющего элемента застаёт поле
/// недособранным. Поэтому разбор поля не читает поток сам, а лишь копит run'ы.
#[derive(Default)]
struct InlineState {
    /// Незакрытое поле.
    field: Option<FieldBuilder>,
}

impl InlineState {
    /// Дописать run и положить в `out` поле, если оно только что закрылось.
    ///
    /// # Errors
    /// То же, что у [`parse_blocks`].
    fn push_run(
        &mut self,
        run: Run,
        parts: FieldParts,
        ctx: &mut ParseCtx,
        part: &str,
        xml_path: &str,
        out: &mut Vec<Inline>,
    ) -> Result<()> {
        let Some(builder) = self.field.as_mut() else {
            if parts.mark == Some(FieldMark::Begin) {
                self.field = Some(FieldBuilder::new(ctx.id(), run, parts));
                return Ok(());
            }
            return push_stray_run(run, parts, ctx, part, xml_path, out);
        };
        builder.push_run(run, parts);
        if builder.closed {
            let builder = self.field.take().expect("the field is being built");
            out.push(Inline::Field(builder.finish()));
        }
        Ok(())
    }

    /// Положить разобранное inline-содержимое в поле или в поток.
    fn push_inlines(&mut self, items: Vec<Inline>, out: &mut Vec<Inline>) {
        match self.field.as_mut() {
            Some(builder) => builder.items.extend(
                items
                    .into_iter()
                    .map(|inline| FieldItem::Inline(Box::new(inline))),
            ),
            None => out.extend(items),
        }
    }

    /// Закрыть поток: несостоявшееся поле возвращается в него как есть.
    ///
    /// # Errors
    /// То же, что у [`parse_blocks`].
    fn finish(
        &mut self,
        ctx: &mut ParseCtx,
        part: &str,
        xml_path: &str,
        out: &mut Vec<Inline>,
    ) -> Result<()> {
        let Some(builder) = self.field.take() else {
            return Ok(());
        };
        ctx.warn_at(
            WarningKind::UnknownElement,
            part,
            Some(xml_path),
            "a complex field has no `w:fldChar w:fldCharType=\"end\"`, its fragments are kept as unknown",
        )?;
        builder.abandon(out);
        Ok(())
    }
}

/// Вернуть run в поток, сохранив разметку непарного поля как `Unknown`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn push_stray_run(
    run: Run,
    parts: FieldParts,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    out: &mut Vec<Inline>,
) -> Result<()> {
    if parts.stray.is_empty() {
        out.push(Inline::Run(run));
        return Ok(());
    }
    ctx.warn_at(
        WarningKind::UnknownElement,
        part,
        Some(xml_path),
        "`w:fldChar`/`w:instrText` outside a complex field, kept as unknown",
    )?;
    out.push(Inline::Run(reinsert_strays(run, parts.stray)));
    Ok(())
}

/// Run с разметкой поля, возвращённой в содержимое как `RunContent::Unknown`.
fn reinsert_strays(mut run: Run, stray: Vec<(NodeId, String)>) -> Run {
    for (id, xml) in stray {
        run.content.push(RunContent::Unknown { id, xml });
    }
    run
}

/// Разобрать один элемент inline-содержимого с учётом собираемого поля.
///
/// # Errors
/// То же, что у [`parse_blocks`].
#[allow(clippy::too_many_arguments)]
fn parse_inline_step(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
    state: &mut InlineState,
    out: &mut Vec<Inline>,
) -> Result<()> {
    // Run — единственный элемент, который может продолжаться в соседнем run'е:
    // `w:fldChar begin` открывает поле, разметка которого лежит в следующих.
    if local_name(element.name().into_inner()) == b"r" && !empty {
        let (run, parts) = parse_run(reader, rels, ctx, part, xml_path)?;
        return state.push_run(run, parts, ctx, part, xml_path, out);
    }
    let mut parsed = Vec::new();
    parse_inline_element(
        reader,
        element,
        empty,
        rels,
        ctx,
        part,
        xml_path,
        depth,
        &mut parsed,
    )?;
    state.push_inlines(parsed, out);
    Ok(())
}

/// Разобрать inline-содержимое до `End` объемлющего элемента.
///
/// # Errors
/// То же, что у [`parse_blocks`].
#[allow(clippy::too_many_arguments)]
fn parse_inline_content(
    reader: &mut XmlReader<'_>,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
    out: &mut Vec<Inline>,
) -> Result<()> {
    let mut state = InlineState::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside inline content",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return state.finish(ctx, part, xml_path, out),
            _ => continue,
        };
        parse_inline_step(
            reader, element, empty, rels, ctx, part, xml_path, depth, &mut state, out,
        )?;
    }
}

/// Разобрать один элемент inline-уровня и дописать его в `out`.
///
/// Составное поле здесь не собирается: одиночный run не видит соседей. Начало
/// поля находит [`parse_inline_step`], а разметка непарного поля — [`parse_run`].
///
/// # Errors
/// То же, что у [`parse_blocks`].
// Список аргументов длиннее порога, как и у соседних разборщиков содержимого:
// к их набору добавляются связи части — без них не разрешить цель ссылки.
// Длинная функция — та же карта соответствия «элемент → разбор», что и ниже:
// разнести её по функциям значило бы разорвать эту карту.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn parse_inline_element(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
    out: &mut Vec<Inline>,
) -> Result<()> {
    match local_name(element.name().into_inner()) {
        b"r" => {
            let run = if empty {
                Run {
                    id: ctx.id(),
                    ..Run::default()
                }
            } else {
                // Соседних run'ов отсюда не видно, поэтому разметку поля собрать
                // нельзя; но и терять её не за что — она возвращается в run как
                // `Unknown`.
                let (run, parts) = parse_run(reader, rels, ctx, part, xml_path)?;
                reinsert_strays(run, parts.stray)
            };
            out.push(Inline::Run(run));
        }
        b"hyperlink" if depth < MAX_INLINE_DEPTH => out.push(Inline::Hyperlink(parse_hyperlink(
            reader, element, empty, rels, ctx, part, xml_path, depth,
        )?)),
        b"fldSimple" if depth < MAX_INLINE_DEPTH => out.push(Inline::Field(parse_simple_field(
            reader, element, empty, rels, ctx, part, xml_path, depth,
        )?)),
        b"hyperlink" | b"fldSimple" => {
            let id = ctx.id();
            let xml = capture_any(reader, element, empty, ctx, part)?;
            ctx.warn_at(
                WarningKind::DeepNesting,
                part,
                Some(xml_path),
                format!(
                    "`{}` is nested deeper than {MAX_INLINE_DEPTH}, kept as unknown",
                    element_name(element)
                ),
            )?;
            out.push(Inline::Unknown { id, xml });
        }
        b"bookmarkStart" => {
            let attrs = attributes(element, part)?;
            let id = ctx.id();
            let bookmark_id = i64::from(attr_i32(&attrs, "id", ctx, part)?.unwrap_or(0));
            let name = find(&attrs, "name").unwrap_or_default().to_owned();
            ctx.start_bookmark(bookmark_id);
            out.push(Inline::Bookmark(Bookmark {
                id,
                name,
                bookmark_id,
            }));
        }
        b"bookmarkEnd" => {
            let attrs = attributes(element, part)?;
            skip_element(reader, empty, part)?;
            let bookmark_id = i64::from(attr_i32(&attrs, "id", ctx, part)?.unwrap_or(0));
            if !ctx.end_bookmark(bookmark_id) {
                ctx.warn_at(
                    WarningKind::OrphanBookmark,
                    part,
                    Some(xml_path),
                    format!("`w:bookmarkEnd` `{bookmark_id}` has no `w:bookmarkStart`, ignored"),
                )?;
            }
        }
        // Правки не разбираются намеренно: содержимое сохраняется целиком, иначе
        // удалённый текст выглядел бы как обычный (план §7).
        b"ins" | b"del" => {
            let id = ctx.id();
            let xml = capture_any(reader, element, empty, ctx, part)?;
            ctx.warn_at(
                WarningKind::UnknownElement,
                part,
                Some(xml_path),
                format!(
                    "`{}` (a track change) is not supported, kept as unknown",
                    element_name(element)
                ),
            )?;
            out.push(Inline::Unknown { id, xml });
        }
        b"AlternateContent" if !empty => {
            match resolve_alternate_content(reader, element, ctx, part, xml_path)? {
                AlternateContent::Choice(fragment) => {
                    out.extend(parse_inline_fragment(
                        &fragment, rels, ctx, part, xml_path, depth,
                    )?);
                }
                AlternateContent::Unsupported(xml) => {
                    ctx.warn_at(
                        WarningKind::UnknownElement,
                        part,
                        Some(xml_path),
                        "`mc:AlternateContent` has no supported `mc:Choice`, kept as unknown",
                    )?;
                    out.push(Inline::Unknown { id: ctx.id(), xml });
                }
            }
        }
        b"drawing" => out.push(Inline::Drawing(parse_drawing(
            reader, empty, rels, ctx, part, xml_path,
        )?)),
        // Разметка, не несущая содержимого модели.
        b"proofErr" | b"lastRenderedPageBreak" => {
            skip_element(reader, empty, part)?;
        }
        _ => {
            let id = ctx.id();
            let xml = capture_any(reader, element, empty, ctx, part)?;
            ctx.warn_at(
                WarningKind::UnknownElement,
                part,
                Some(xml_path),
                format!(
                    "`{}` is not supported in a paragraph, kept as unknown",
                    element_name(element)
                ),
            )?;
            out.push(Inline::Unknown { id, xml });
        }
    }
    Ok(())
}

/// Разобрать `w:hyperlink`: цель, подсказка и содержимое.
///
/// Цель внешней ссылки — адрес, а не часть пакета: `Relationship::part` её не
/// строит, поэтому для `TargetMode="External"` берётся сам `Target`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
#[allow(clippy::too_many_arguments)]
fn parse_hyperlink(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
) -> Result<Hyperlink> {
    let id = ctx.id();
    let attrs = attributes(element, part)?;
    let rel_id = find(&attrs, "id").map(str::to_owned);
    let target = if let Some(rel_id) = rel_id.as_deref() {
        if let Some(relation) = rels.get(rel_id) {
            Some(
                relation
                    .part(part)
                    .unwrap_or_else(|| relation.target.clone()),
            )
        } else {
            ctx.warn(
                WarningKind::MissingPart,
                part,
                format!("`w:hyperlink`: relationship `{rel_id}` is not declared"),
            )?;
            None
        }
    } else {
        None
    };
    let mut runs = Vec::new();
    if !empty {
        parse_inline_content(reader, rels, ctx, part, xml_path, depth + 1, &mut runs)?;
    }
    Ok(Hyperlink {
        id,
        // Ссылка с `r:id` ведёт за пределы документа, с `w:anchor` — внутрь него.
        external: rel_id.is_some(),
        rel_id,
        anchor: find(&attrs, "anchor").map(str::to_owned),
        tooltip: find(&attrs, "tooltip").map(str::to_owned),
        target,
        runs,
    })
}

/// Разобрать `w:fldSimple`: простое поле с инструкцией в атрибуте.
///
/// # Errors
/// То же, что у [`parse_blocks`].
#[allow(clippy::too_many_arguments)]
fn parse_simple_field(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
) -> Result<Field> {
    let attrs = attributes(element, part)?;
    let id = ctx.id();
    // `w:instr` сохраняется как есть, вместе с окружающими пробелами: это
    // инструкция поля, а не текст документа, и её формат задаёт сам код поля.
    let instruction = find(&attrs, "instr").unwrap_or_default().to_owned();
    let dirty = attr_on(&attrs, "dirty", false);
    let mut result = Vec::new();
    if !empty {
        parse_inline_content(reader, rels, ctx, part, xml_path, depth + 1, &mut result)?;
    }
    Ok(Field {
        id,
        kind: FieldKind::Simple,
        instruction,
        result,
        dirty,
    })
}

/// Разобрать фрагмент `mc:Choice` как inline-содержимое.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_inline_fragment(
    fragment: &str,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
) -> Result<Vec<Inline>> {
    let wrapped = wrap_fragment(fragment);
    let mut reader = XmlReader::preserving(wrapped.as_bytes(), part);
    // Единственный `End` фрагмента — `End` синтетического корня: на нём и
    // остановится `parse_inline_content`, а `End` самого `mc:AlternateContent`
    // уже вычитан `resolve_alternate_content`.
    expect_fragment_root(&mut reader, part)?;
    let mut out = Vec::new();
    parse_inline_content(&mut reader, rels, ctx, part, xml_path, depth + 1, &mut out)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Run и его содержимое
// ---------------------------------------------------------------------------

/// Разобрать `w:r`: свойства знака и содержимое.
///
/// Возвращает ещё и найденную разметку составного поля: `w:fldChar`/`w:instrText`
/// в [`RunContent`] не выражаются, а поле собирается из соседних run'ов — run
/// лишь отдаёт её вызывающему.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_run(
    reader: &mut XmlReader<'_>,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<(Run, FieldParts)> {
    let id = ctx.id();
    let mut rpr = RawRPr::default();
    let mut content = Vec::new();
    let mut field = FieldParts::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:r`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => break,
            _ => continue,
        };
        if local_name(element.name().into_inner()) == b"rPr" {
            rpr = if empty {
                RawRPr::default()
            } else {
                parse_rpr(reader, ctx, part, xml_path)?
            };
        } else {
            parse_run_content(
                reader,
                element,
                empty,
                rels,
                ctx,
                part,
                xml_path,
                &mut field,
                &mut content,
            )?;
        }
    }
    let style_ref = rpr.style.clone();
    Ok((
        Run {
            id,
            rpr: Box::new(rpr),
            style_ref,
            content,
        },
        field,
    ))
}

/// Разобрать один элемент содержимого run'а и дописать его в `out`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
#[allow(clippy::too_many_arguments)]
fn parse_run_content(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    field: &mut FieldParts,
    out: &mut Vec<RunContent>,
) -> Result<()> {
    match local_name(element.name().into_inner()) {
        b"t" => {
            let text = if empty {
                String::new()
            } else {
                read_text(reader, part)?
            };
            out.push(RunContent::Text(text));
        }
        b"tab" => out.push(RunContent::Tab),
        b"br" => out.push(RunContent::Break(parse_break(&attributes(element, part)?))),
        b"cr" => out.push(RunContent::Break(BreakKind::Line)),
        b"sym" => {
            let attrs = attributes(element, part)?;
            if let Some((font, ch)) = parse_symbol(&attrs, ctx, part)? {
                out.push(RunContent::Symbol { font, char: ch });
            }
        }
        // Неразрывный дефис и мягкий перенос — знаки, а не разметка: модель
        // хранит их как текст, иначе плоский текст разошёлся бы с исходным.
        b"noBreakHyphen" => out.push(RunContent::Text("\u{2011}".to_owned())),
        b"softHyphen" => out.push(RunContent::Text("\u{00ad}".to_owned())),
        // Разметка составного поля в `RunContent` не выражается: run отдаёт её
        // наружу, а сам элемент кладёт в `stray` — иначе он пропал бы, не
        // закрой поле соседний run.
        b"fldChar" => {
            let attrs = attributes(element, part)?;
            if field.mark.is_none() {
                field.mark = find(&attrs, "fldCharType").and_then(parse_field_mark);
            }
            field.dirty |= attr_on(&attrs, "dirty", false);
            field
                .stray
                .push((ctx.id(), capture_any(reader, element, empty, ctx, part)?));
        }
        b"instrText" => {
            let id = ctx.id();
            let xml = capture_any(reader, element, empty, ctx, part)?;
            field.instruction.push_str(&fragment_text(&xml, part)?);
            field.stray.push((id, xml));
        }
        b"AlternateContent" if !empty => {
            let fragment = match resolve_alternate_content(reader, element, ctx, part, xml_path)? {
                AlternateContent::Choice(fragment) => fragment,
                AlternateContent::Unsupported(xml) => {
                    ctx.warn_at(
                        WarningKind::UnknownElement,
                        part,
                        Some(xml_path),
                        "`mc:AlternateContent` has no supported `mc:Choice`, kept as unknown",
                    )?;
                    out.push(RunContent::Unknown { id: ctx.id(), xml });
                    return Ok(());
                }
            };
            out.extend(parse_run_fragment(
                &fragment, rels, ctx, part, xml_path, field,
            )?);
        }
        b"drawing" => out.push(RunContent::Drawing(parse_drawing(
            reader, empty, rels, ctx, part, xml_path,
        )?)),
        b"lastRenderedPageBreak" => skip_element(reader, empty, part)?,
        _ => {
            let id = ctx.id();
            let xml = capture_any(reader, element, empty, ctx, part)?;
            ctx.warn_at(
                WarningKind::UnknownElement,
                part,
                Some(xml_path),
                format!(
                    "`{}` is not supported in a run, kept as unknown",
                    element_name(element)
                ),
            )?;
            out.push(RunContent::Unknown { id, xml });
        }
    }
    Ok(())
}

/// Разобрать фрагмент `mc:Choice` как содержимое run'а.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_run_fragment(
    fragment: &str,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    field: &mut FieldParts,
) -> Result<Vec<RunContent>> {
    let wrapped = wrap_fragment(fragment);
    let mut reader = XmlReader::preserving(wrapped.as_bytes(), part);
    expect_fragment_root(&mut reader, part)?;
    let mut out = Vec::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside a run fragment",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(out),
            _ => continue,
        };
        parse_run_content(
            &mut reader,
            element,
            empty,
            rels,
            ctx,
            part,
            xml_path,
            field,
            &mut out,
        )?;
    }
}

// ---------------------------------------------------------------------------
// Рисунки
// ---------------------------------------------------------------------------

/// Предел глубины обхода `w:drawing`.
///
/// Сам рисунок вложен неглубоко (`a:graphic/pic:pic/…`), а обход рекурсивный:
/// без предела патологический вход уронил бы разбор стеком.
const MAX_DRAWING_DEPTH: usize = 24;

/// Собранные свойства `wp:inline`/`wp:anchor`.
///
/// Нужное разбросано по ветвям рисунка: `wp:docPr` рядом с `wp:extent`, а
/// ссылка на картинку — в `a:graphic/pic:pic/pic:blipFill/a:blip`, глубина
/// которой заранее не известна. Поэтому поддерево обходится целиком.
#[derive(Default)]
struct DrawingParts {
    /// Встретился `wp:inline` или `wp:anchor`.
    found: bool,
    /// Встретился именно `wp:inline`; иначе рисунок плавающий.
    inline: bool,
    /// `wp:anchor/@behindDoc`: рисунок лежит под текстом.
    behind_text: bool,
    /// `wp:extent`.
    extent: Option<Extent>,
    /// `wp:docPr/@name`.
    name: Option<String>,
    /// `wp:docPr/@descr`.
    description: Option<String>,
    /// `a:blip/@r:embed` или `@r:link`.
    rel_id: Option<String>,
    /// `wp:positionH`.
    horizontal: Option<PositionH>,
    /// `wp:positionV`.
    vertical: Option<PositionV>,
    /// `wp:wrap*`.
    wrap: Option<WrapKind>,
}

/// Разобрать `w:drawing`: `wp:inline` или `wp:anchor`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_drawing(
    reader: &mut XmlReader<'_>,
    empty: bool,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<InlineOrAnchor> {
    let id = ctx.id();
    let mut parts = DrawingParts::default();
    if !empty {
        scan_drawing(reader, ctx, part, xml_path, 0, &mut parts)?;
    }
    if !parts.found {
        // Ни `wp:inline`, ни `wp:anchor`: у рисунка нечего разбирать (так
        // выглядит, например, `w:drawing` с одной лишь фигурой VML в Fallback).
        ctx.warn_at(
            WarningKind::UnknownElement,
            part,
            Some(xml_path),
            "`w:drawing` has no `wp:inline` or `wp:anchor`, kept as an empty drawing",
        )?;
        return Ok(InlineOrAnchor {
            id,
            inline: None,
            anchor: None,
        });
    }
    let rel_id = parts.rel_id.unwrap_or_default();
    let image = InlineImage {
        id: ctx.id(),
        part: image_part(rels, part, &rel_id, ctx, xml_path)?,
        rel_id,
        name: parts.name,
        description: parts.description,
        extent: parts.extent.unwrap_or(Extent { cx: 0, cy: 0 }),
    };
    if parts.inline {
        return Ok(InlineOrAnchor {
            id,
            inline: Some(image),
            anchor: None,
        });
    }
    Ok(InlineOrAnchor {
        id,
        inline: None,
        anchor: Some(Anchor {
            id: ctx.id(),
            extent: image.extent.clone(),
            horizontal: parts.horizontal.unwrap_or_else(default_position_h),
            vertical: parts.vertical.unwrap_or_else(default_position_v),
            wrap: parts.wrap.unwrap_or(WrapKind::None),
            behind_text: parts.behind_text,
            image,
        }),
    })
}

/// Цель связи картинки, приведённая к пути части.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn image_part(
    rels: &Relationships,
    source: &str,
    rel_id: &str,
    ctx: &mut ParseCtx,
    xml_path: &str,
) -> Result<Option<String>> {
    if rel_id.is_empty() {
        return Ok(None);
    }
    if let Some(rel) = rels.get(rel_id) {
        Ok(rel.part(source))
    } else {
        // Битая ссылка — не отказ: рисунок останется без части, но
        // документ откроется (ADR-0016 §2).
        ctx.warn_at(
            WarningKind::MissingPart,
            source,
            Some(xml_path),
            format!("the relationship `{rel_id}` of the image is missing"),
        )?;
        Ok(None)
    }
}

/// Обойти детей `w:drawing`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn scan_drawing(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
    parts: &mut DrawingParts,
) -> Result<()> {
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:drawing`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(()),
            _ => continue,
        };
        scan_drawing_element(reader, element, empty, ctx, part, xml_path, depth, parts)?;
    }
}

/// Разобрать один элемент внутри `w:drawing` и спуститься в его детей.
///
/// # Errors
/// То же, что у [`parse_blocks`].
// Опять же порог аргументов: обход рекурсивный, и состояние (`parts`, `depth`)
// с окружением разбора иначе не передать.
#[allow(clippy::too_many_arguments)]
fn scan_drawing_element(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    depth: usize,
    parts: &mut DrawingParts,
) -> Result<()> {
    if depth >= MAX_DRAWING_DEPTH {
        ctx.warn_at(
            WarningKind::DeepNesting,
            part,
            Some(xml_path),
            format!("`w:drawing` is nested deeper than {MAX_DRAWING_DEPTH}, the rest is skipped"),
        )?;
        return skip_element(reader, empty, part);
    }
    let attrs = attributes(element, part)?;
    match local_name(element.name().into_inner()) {
        b"inline" => {
            parts.found = true;
            parts.inline = true;
        }
        b"anchor" => {
            parts.found = true;
            parts.inline = false;
            parts.behind_text = find(&attrs, "behindDoc").is_some_and(is_true);
        }
        b"extent" => {
            // `wp:extent` — размер рисунка; `a:ext` (в `a:xfrm`) сюда не попадает.
            parts.extent = Some(Extent {
                cx: attr_i64(&attrs, "cx", ctx, part)?.unwrap_or_default(),
                cy: attr_i64(&attrs, "cy", ctx, part)?.unwrap_or_default(),
            });
        }
        b"docPr" => {
            parts.name = find(&attrs, "name").map(str::to_owned);
            parts.description = find(&attrs, "descr").map(str::to_owned);
        }
        b"blip" => {
            parts.rel_id = find(&attrs, "embed")
                .or_else(|| find(&attrs, "link"))
                .map(str::to_owned);
        }
        b"wrapNone" => parts.wrap = Some(WrapKind::None),
        b"wrapSquare" => parts.wrap = Some(WrapKind::Square),
        b"wrapTight" => parts.wrap = Some(WrapKind::Tight),
        b"wrapThrough" => parts.wrap = Some(WrapKind::Through),
        b"wrapTopAndBottom" => parts.wrap = Some(WrapKind::TopAndBottom),
        // Позиция — единственная ветвь с собственными детьми: `wp:align`,
        // `wp:posOffset` или `wp:pct` лежат внутри неё, а не атрибутами.
        b"positionH" => {
            parts.horizontal = Some(parse_position_h(
                reader, empty, &attrs, ctx, part, xml_path,
            )?);
            return Ok(());
        }
        b"positionV" => {
            parts.vertical = Some(parse_position_v(
                reader, empty, &attrs, ctx, part, xml_path,
            )?);
            return Ok(());
        }
        _ => {}
    }
    if empty {
        return Ok(());
    }
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:drawing`",
            ));
        };
        match &event {
            Event::Start(child) => {
                scan_drawing_element(reader, child, false, ctx, part, xml_path, depth + 1, parts)?;
            }
            Event::Empty(child) => {
                scan_drawing_element(reader, child, true, ctx, part, xml_path, depth + 1, parts)?;
            }
            Event::End(_) => return Ok(()),
            _ => {}
        }
    }
}

/// Позиция по горизонтали: `wp:positionH`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_position_h(
    reader: &mut XmlReader<'_>,
    empty: bool,
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<PositionH> {
    let value = if empty {
        PositionValue::default()
    } else {
        parse_position_value(reader, ctx, part, xml_path)?
    };
    Ok(PositionH {
        relative_from: parse_rel_from_h(find(attrs, "relativeFrom"), ctx, part)?,
        align: value.align.as_deref().map(parse_align_h),
        offset: value.offset,
        percent: value.percent,
    })
}

/// Позиция по вертикали: `wp:positionV`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_position_v(
    reader: &mut XmlReader<'_>,
    empty: bool,
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<PositionV> {
    let value = if empty {
        PositionValue::default()
    } else {
        parse_position_value(reader, ctx, part, xml_path)?
    };
    Ok(PositionV {
        relative_from: parse_rel_from_v(find(attrs, "relativeFrom"), ctx, part)?,
        align: value.align.as_deref().map(parse_align_v),
        offset: value.offset,
        percent: value.percent,
    })
}

/// Значение позиции: `wp:align`, `wp:posOffset` или `wp:pct` — ровно одно.
#[derive(Default)]
struct PositionValue {
    /// `wp:align`.
    align: Option<String>,
    /// `wp:posOffset` — смещение в EMU.
    offset: Option<i64>,
    /// `wp:pct` — доля в тысячных долях процента; хранится как есть.
    percent: Option<i32>,
}

/// Разобрать детей `wp:positionH`/`wp:positionV`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_position_value(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<PositionValue> {
    let mut value = PositionValue::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `wp:positionH`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(value),
            _ => continue,
        };
        match local_name(element.name().into_inner()) {
            b"align" => {
                value.align = Some(if empty {
                    String::new()
                } else {
                    read_text(reader, part)?
                });
            }
            b"posOffset" => {
                value.offset = read_number(reader, empty, ctx, part, xml_path, "wp:posOffset")?;
            }
            b"pct" => {
                if let Some(raw) = read_number(reader, empty, ctx, part, xml_path, "wp:pct")? {
                    match i32::try_from(raw) {
                        Ok(percent) => value.percent = Some(percent),
                        Err(_) => ctx.warn_at(
                            WarningKind::InvalidAttribute,
                            part,
                            Some(xml_path),
                            format!("`wp:pct`: `{raw}` is out of range, ignored"),
                        )?,
                    }
                }
            }
            _ => skip_element(reader, empty, part)?,
        }
    }
}

/// База отсчёта `wp:positionH/@relativeFrom`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_rel_from_h(raw: Option<&str>, ctx: &mut ParseCtx, part: &str) -> Result<RelFromH> {
    let Some(raw) = raw else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`wp:positionH` has no `relativeFrom`, counted as `margin`",
        )?;
        return Ok(RelFromH::Margin);
    };
    Ok(match raw {
        "margin" => RelFromH::Margin,
        "page" => RelFromH::Page,
        "column" => RelFromH::Column,
        "character" => RelFromH::Character,
        "leftMargin" => RelFromH::LeftMargin,
        "rightMargin" => RelFromH::RightMargin,
        "insideMargin" => RelFromH::InsideMargin,
        "outsideMargin" => RelFromH::OutsideMargin,
        other => RelFromH::Other(other.to_owned()),
    })
}

/// База отсчёта `wp:positionV/@relativeFrom`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_rel_from_v(raw: Option<&str>, ctx: &mut ParseCtx, part: &str) -> Result<RelFromV> {
    let Some(raw) = raw else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`wp:positionV` has no `relativeFrom`, counted as `margin`",
        )?;
        return Ok(RelFromV::Margin);
    };
    Ok(match raw {
        "margin" => RelFromV::Margin,
        "page" => RelFromV::Page,
        "paragraph" => RelFromV::Paragraph,
        "line" => RelFromV::Line,
        "topMargin" => RelFromV::TopMargin,
        "bottomMargin" => RelFromV::BottomMargin,
        "insideMargin" => RelFromV::InsideMargin,
        "outsideMargin" => RelFromV::OutsideMargin,
        other => RelFromV::Other(other.to_owned()),
    })
}

/// `wp:align` по горизонтали.
#[must_use]
fn parse_align_h(raw: &str) -> AlignH {
    match raw {
        "left" => AlignH::Left,
        "center" => AlignH::Center,
        "right" => AlignH::Right,
        "inside" => AlignH::Inside,
        "outside" => AlignH::Outside,
        other => AlignH::Other(other.to_owned()),
    }
}

/// `wp:align` по вертикали.
#[must_use]
fn parse_align_v(raw: &str) -> AlignV {
    match raw {
        "top" => AlignV::Top,
        "center" => AlignV::Center,
        "bottom" => AlignV::Bottom,
        "inside" => AlignV::Inside,
        "outside" => AlignV::Outside,
        other => AlignV::Other(other.to_owned()),
    }
}

/// Позиция плавающего рисунка, если `wp:positionH` в разметке нет.
#[must_use]
fn default_position_h() -> PositionH {
    PositionH {
        relative_from: RelFromH::Margin,
        align: None,
        offset: None,
        percent: None,
    }
}

/// Позиция плавающего рисунка, если `wp:positionV` в разметке нет.
#[must_use]
fn default_position_v() -> PositionV {
    PositionV {
        relative_from: RelFromV::Margin,
        align: None,
        offset: None,
        percent: None,
    }
}

/// Атрибут-число в `i64`: EMU (`wp:extent/@cx`) не помещаются в `i32`.
///
/// # Errors
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn attr_i64(attrs: &[Attr<'_>], name: &str, ctx: &mut ParseCtx, part: &str) -> Result<Option<i64>> {
    let Some(raw) = find(attrs, name) else {
        return Ok(None);
    };
    if let Ok(value) = raw.trim().parse::<i64>() {
        Ok(Some(value))
    } else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("`{name}`: `{raw}` is not a number, ignored"),
        )?;
        Ok(None)
    }
}

/// Число из текста элемента: `wp:posOffset` и `wp:pct` записаны текстом.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn read_number(
    reader: &mut XmlReader<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
    element: &str,
) -> Result<Option<i64>> {
    if empty {
        return Ok(None);
    }
    let raw = read_text(reader, part)?;
    let raw = raw.trim();
    if let Ok(value) = raw.parse::<i64>() {
        Ok(Some(value))
    } else {
        ctx.warn_at(
            WarningKind::InvalidAttribute,
            part,
            Some(xml_path),
            format!("`{element}`: `{raw}` is not a number, ignored"),
        )?;
        Ok(None)
    }
}

/// Вид разрыва из `w:br/@w:type`.
///
/// Отсутствие атрибута — разрыв строки; вид, которого нет в модели,
/// сохраняется как [`BreakKind::Unsupported`], а не подменяется строкой.
#[must_use]
fn parse_break(attrs: &[Attr<'_>]) -> BreakKind {
    match find(attrs, "type") {
        None | Some("textWrapping") => BreakKind::Line,
        Some("page") => BreakKind::Page,
        Some("column") => BreakKind::Column,
        Some(other) => BreakKind::Unsupported(other.to_owned()),
    }
}

/// Символ `w:sym`: шрифт-символов и код знака.
///
/// `# Errors`
/// [`Error::TooManyWarnings`] — предупреждений стало больше порога.
fn parse_symbol(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<(String, char)>> {
    let Some(raw) = find(attrs, "char") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:sym` has no `w:char`, the symbol is skipped",
        )?;
        return Ok(None);
    };
    // Код символа записан hex-числом (`F0E0`), а не десятичным: десятичный
    // разбор молча дал бы чужой знак.
    let Some(value) = u32::from_str_radix(raw.trim(), 16)
        .ok()
        .and_then(char::from_u32)
    else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("`w:sym`: `{raw}` is not a valid character code, the symbol is skipped"),
        )?;
        return Ok(None);
    };
    let Some(font) = find(attrs, "font") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:sym` has no `w:font`, the symbol is skipped",
        )?;
        return Ok(None);
    };
    Ok(Some((font.to_owned(), value)))
}

/// Прочитать текст `w:t`, разворачивая ссылки на сущности (`&amp;`, `&#x41;`).
///
/// Начиная с quick-xml 0.41 ссылка приходит отдельным событием, и `&amp;` иначе
/// потерялся бы: текст склеивается из нескольких событий.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше `End` элемента или байты не UTF-8.
fn read_text(reader: &mut XmlReader<'_>, part: &str) -> Result<String> {
    let mut text = String::new();
    let mut depth: u32 = 0;
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:t`",
            ));
        };
        if let Some(chunk) = event_text(&event) {
            text.push_str(&utf8(chunk, part)?);
            continue;
        }
        match event {
            Event::GeneralRef(reference) => text.push_str(&resolve_reference(&reference, part)?),
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(text),
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
}

/// Текст элемента по уже сохранённому XML, с развёрнутыми сущностями.
///
/// `w:instrText` читается целиком — его XML нужен [`FieldParts::stray`], — а
/// инструкции нужен текст: один проход отдаёт что-то одно, поэтому фрагмент
/// разбирается ещё раз.
///
/// # Errors
/// То же, что у [`read_text`].
fn fragment_text(xml: &str, part: &str) -> Result<String> {
    let mut reader = XmlReader::preserving(xml.as_bytes(), part);
    match reader.next_significant()? {
        Some(Event::Start(_)) => read_text(&mut reader, part),
        _ => Ok(String::new()),
    }
}

/// Байты текстового события: текста или CDATA.
fn event_text<'a>(event: &'a Event<'_>) -> Option<&'a [u8]> {
    match event {
        Event::Text(text) => Some(&text[..]),
        Event::CData(data) => Some(&data[..]),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Свойства абзаца
// ---------------------------------------------------------------------------

/// Разобрать `w:pPr`.
///
/// Незнакомые элементы не отбрасываются: их XML ложится в `RawPPr::unknown` —
/// раскладке и отладке нужно видеть неподдержанное свойство (ADR-0014 §3).
///
/// Возвращает свойства вместе с `w:sectPr`, если он здесь есть: в `RawPPr` ему
/// места нет, а абзац хранит его отдельно — это конец секции, а не свойство знака.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_ppr(
    reader: &mut XmlReader<'_>,
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<(RawPPr, Option<SectionProperties>)> {
    let mut ppr = RawPPr::default();
    let mut section_break = None;
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:pPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok((ppr, section_break)),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"pStyle" => ppr.style = find(&attrs, "val").map(StyleId::new),
            b"numPr" => {
                ppr.num_pr = Some(if empty {
                    NumPr::default()
                } else {
                    parse_num_pr(reader, ctx, part, xml_path)?
                });
            }
            b"spacing" => ppr.spacing = Some(parse_spacing(&attrs, ctx, part)?),
            b"ind" => ppr.ind = Some(parse_ind(&attrs, ctx, part)?),
            b"jc" => ppr.jc = parse_justification(&attrs, ctx, part)?,
            b"keepNext" => ppr.keep_next = attr_toggle(&attrs, ctx, part, "w:keepNext")?,
            b"keepLines" => ppr.keep_lines = attr_toggle(&attrs, ctx, part, "w:keepLines")?,
            b"pageBreakBefore" => {
                ppr.page_break_before = attr_toggle(&attrs, ctx, part, "w:pageBreakBefore")?;
            }
            b"widowControl" => {
                ppr.widow_control = attr_toggle(&attrs, ctx, part, "w:widowControl")?;
            }
            b"outlineLvl" => {
                ppr.outline_lvl =
                    attr_range(&attrs, "val", MAX_OUTLINE_LVL, ctx, part, "w:outlineLvl")?;
            }
            b"pBdr" => {
                ppr.p_bdr = Some(if empty {
                    ParagraphBorders::default()
                } else {
                    parse_borders(reader, ctx, part, xml_path)?
                });
            }
            b"shd" => ppr.shd = Some(parse_shading(&attrs, ctx, part)?),
            b"tabs" => {
                if !empty {
                    ppr.tabs = parse_tabs(reader, ctx, part, xml_path)?;
                }
            }
            b"rPr" => {
                ppr.r_pr = Some(if empty {
                    RawRPr::default()
                } else {
                    parse_rpr(reader, ctx, part, xml_path)?
                });
            }
            b"sectPr" => {
                // `w:sectPr` в `w:pPr` завершает секцию, а не открывает её:
                // секция тянется до этого абзаца включительно.
                section_break = Some(parse_sect_pr(reader, empty, rels, ctx, part, xml_path)?);
            }
            _ => {
                let xml = capture_any(reader, element, empty, ctx, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:pPr` is not supported, kept as unknown",
                        element_name(element)
                    ),
                )?;
                ppr.unknown.push((local_name_of(element), xml));
            }
        }
    }
}

/// Разобрать `w:numPr`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_num_pr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<NumPr> {
    let mut num_pr = NumPr::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:numPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(num_pr),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"ilvl" => num_pr.ilvl = attr_range(&attrs, "val", MAX_ILVL, ctx, part, "w:ilvl")?,
            b"numId" => num_pr.num_id = attr_u32(&attrs, "val", ctx, part)?.map(NumId::new),
            _ => {
                // У `NumPr` нет поля `unknown`: неподдержанное свойство уровня
                // списка терять не жалко, но предупредить о нём нужно.
                capture_any(reader, element, empty, ctx, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:numPr` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать `w:spacing` абзаца (`w:pPr/w:spacing`).
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_spacing(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<ParagraphSpacing> {
    Ok(ParagraphSpacing {
        before: attr_i32(attrs, "before", ctx, part)?.map(Twips::new),
        after: attr_i32(attrs, "after", ctx, part)?.map(Twips::new),
        line: attr_i32(attrs, "line", ctx, part)?.map(LineSpacing::new),
        line_rule: find(attrs, "lineRule").map(parse_line_rule),
        before_lines: attr_u32(attrs, "beforeLines", ctx, part)?,
        after_lines: attr_u32(attrs, "afterLines", ctx, part)?,
        before_autospacing: find(attrs, "beforeAutospacing").is_some_and(is_true),
        after_autospacing: find(attrs, "afterAutospacing").is_some_and(is_true),
    })
}

/// Разобрать `w:ind`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_ind(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Ind> {
    // `start`/`end` — логические написания `left`/`right`: пишет и так, и так.
    let left = attr_i32(attrs, "left", ctx, part)?.or(attr_i32(attrs, "start", ctx, part)?);
    let right = attr_i32(attrs, "right", ctx, part)?.or(attr_i32(attrs, "end", ctx, part)?);
    Ok(Ind {
        left: left.map(Twips::new),
        right: right.map(Twips::new),
        first_line: attr_i32(attrs, "firstLine", ctx, part)?.map(Twips::new),
        hanging: attr_i32(attrs, "hanging", ctx, part)?.map(Twips::new),
    })
}

/// Разобрать `w:pBdr`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_borders(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<ParagraphBorders> {
    let mut borders = ParagraphBorders::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:pBdr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(borders),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"top" => borders.top = parse_border(&attrs, ctx, part)?,
            b"left" => borders.left = parse_border(&attrs, ctx, part)?,
            b"bottom" => borders.bottom = parse_border(&attrs, ctx, part)?,
            b"right" => borders.right = parse_border(&attrs, ctx, part)?,
            b"between" => borders.between = parse_border(&attrs, ctx, part)?,
            b"bar" => borders.bar = parse_border(&attrs, ctx, part)?,
            _ => {
                capture_any(reader, element, empty, ctx, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:pBdr` is not supported, ignored",
                        element_name(element)
                    ),
                )?;
            }
        }
    }
}

/// Разобрать одну границу (`w:top`, `w:left`, …).
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_border(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<Border>> {
    let Some(raw) = find(attrs, "val") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "a border has no `w:val`, the border is skipped",
        )?;
        return Ok(None);
    };
    Ok(Some(Border {
        val: parse_border_style(raw),
        sz: attr_u32(attrs, "sz", ctx, part)?,
        space: attr_u32(attrs, "space", ctx, part)?,
        color: parse_color(find(attrs, "color"), ctx, part)?,
    }))
}

/// Разобрать `w:tabs`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_tabs(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Vec<TabStop>> {
    let mut tabs = Vec::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:tabs`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(tabs),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        if local_name(element.name().into_inner()) == b"tab" {
            if let Some(stop) = parse_tab_stop(&attrs, ctx, part)? {
                tabs.push(stop);
            }
        } else {
            capture_any(reader, element, empty, ctx, part)?;
            ctx.warn_at(
                WarningKind::UnknownElement,
                part,
                Some(xml_path),
                format!(
                    "`{}` in `w:tabs` is not supported, ignored",
                    element_name(element)
                ),
            )?;
        }
    }
}

/// Разобрать одну позицию табуляции (`w:tabs/w:tab`).
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_tab_stop(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<TabStop>> {
    let Some(pos) = attr_i32(attrs, "pos", ctx, part)? else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:tab` has no `w:pos`, the stop is skipped",
        )?;
        return Ok(None);
    };
    Ok(Some(TabStop {
        val: Twips::new(pos),
        kind: find(attrs, "val").map_or(TabStopKind::Left, parse_tab_kind),
        leader: find(attrs, "leader").map_or(TabLeader::None, parse_tab_leader),
    }))
}

/// Разобрать `w:shd`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_shading(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Shading> {
    let raw = find(attrs, "val");
    if raw.is_none() {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:shd` has no `w:val`, counted as `nil`",
        )?;
    }
    Ok(Shading {
        val: raw.map_or(ShadingPattern::Nil, parse_shading_pattern),
        color: parse_color(find(attrs, "color"), ctx, part)?,
        fill: parse_color(find(attrs, "fill"), ctx, part)?,
    })
}

// ---------------------------------------------------------------------------
// Свойства знака
// ---------------------------------------------------------------------------

/// Разобрать `w:rPr`.
///
/// Незнакомые элементы ложатся в `RawRPr::unknown` — см. [`parse_ppr`].
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_rpr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<RawRPr> {
    let mut rpr = RawRPr::default();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside `w:rPr`",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(rpr),
            _ => continue,
        };
        let attrs = attributes(element, part)?;
        match local_name(element.name().into_inner()) {
            b"rStyle" => rpr.style = find(&attrs, "val").map(StyleId::new),
            b"rFonts" => rpr.r_fonts = Some(parse_rfonts(&attrs)),
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
            b"color" => rpr.color = parse_color(find(&attrs, "val"), ctx, part)?,
            b"sz" => rpr.sz = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            b"szCs" => rpr.sz_cs = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            b"highlight" => rpr.highlight = parse_highlight(&attrs, ctx, part)?,
            b"u" => rpr.u = Some(parse_underline(&attrs)),
            b"vertAlign" => rpr.vert_align = parse_vert_align(&attrs, ctx, part)?,
            b"spacing" => {
                rpr.spacing = Some(CharacterSpacing {
                    value: attr_i32(&attrs, "val", ctx, part)?.map(Twips::new),
                });
            }
            b"position" => rpr.position = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            _ => {
                let xml = capture_any(reader, element, empty, ctx, part)?;
                ctx.warn_at(
                    WarningKind::UnknownElement,
                    part,
                    Some(xml_path),
                    format!(
                        "`{}` in `w:rPr` is not supported, kept as unknown",
                        element_name(element)
                    ),
                )?;
                rpr.unknown.push((local_name_of(element), xml));
            }
        }
    }
}

/// Разобрать `w:rFonts`.
#[must_use]
fn parse_rfonts(attrs: &[Attr<'_>]) -> RFonts {
    RFonts {
        ascii: find(attrs, "ascii").map(str::to_owned),
        h_ansi: find(attrs, "hAnsi").map(str::to_owned),
        east_asia: find(attrs, "eastAsia").map(str::to_owned),
        cs: find(attrs, "cs").map(str::to_owned),
        hint: find(attrs, "hint").map(parse_font_hint),
    }
}

/// Разобрать цвет из значения `w:val`/`w:color`/`w:fill`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_color(raw: Option<&str>, ctx: &mut ParseCtx, part: &str) -> Result<Option<Color>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let raw = raw.trim();
    if let Some(value) = u32::from_str_radix(raw, 16).ok().filter(|_| raw.len() == 6) {
        return Ok(Some(Color::Rgb(value)));
    }
    match raw {
        "auto" => Ok(Some(Color::Auto)),
        "none" => Ok(Some(Color::None)),
        other => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`{other}` is not an `RRGGBB` color, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Разобрать `w:highlight`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_highlight(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<Highlight>> {
    let Some(raw) = find(attrs, "val") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:highlight` has no `w:val`, ignored",
        )?;
        return Ok(None);
    };
    Ok(Some(match raw {
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
        "darkGray" => Highlight::DarkGray,
        "lightGray" => Highlight::LightGray,
        "none" => Highlight::None,
        other => Highlight::Other(other.to_owned()),
    }))
}

/// Разобрать `w:u`; отсутствие `w:val` — одиночная линия (значение по схеме).
#[must_use]
fn parse_underline(attrs: &[Attr<'_>]) -> Underline {
    match find(attrs, "val") {
        None | Some("single") => Underline::Single,
        Some("words") => Underline::Words,
        Some("double") => Underline::Double,
        Some("thick") => Underline::Thick,
        Some("dotted") => Underline::Dotted,
        Some("dottedHeavy") => Underline::DottedHeavy,
        Some("dash") => Underline::Dash,
        Some("dashedHeavy") => Underline::DashedHeavy,
        Some("dashLong") => Underline::DashLong,
        Some("dashLongHeavy") => Underline::DashLongHeavy,
        Some("dotDash") => Underline::DotDash,
        Some("dashDotHeavy") => Underline::DashDotHeavy,
        Some("dotDotDash") => Underline::DotDotDash,
        Some("dashDotDotHeavy") => Underline::DashDotDotHeavy,
        Some("wave") => Underline::Wave,
        Some("wavyHeavy") => Underline::WavyHeavy,
        Some("wavyDouble") => Underline::WavyDouble,
        Some("none") => Underline::None,
        Some(other) => Underline::Other(other.to_owned()),
    }
}

/// Разобрать `w:vertAlign`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_vert_align(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<VertAlign>> {
    let align = match find(attrs, "val") {
        Some("baseline") => VertAlign::Baseline,
        Some("superscript") => VertAlign::Superscript,
        Some("subscript") => VertAlign::Subscript,
        other => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!(
                    "`w:vertAlign`: `{}` is unknown, ignored",
                    other.unwrap_or_default()
                ),
            )?;
            return Ok(None);
        }
    };
    Ok(Some(align))
}

/// Разобрать `w:jc`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_justification(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<Justification>> {
    let Some(raw) = find(attrs, "val") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:jc` has no `w:val`, ignored",
        )?;
        return Ok(None);
    };
    Ok(Some(match raw {
        "left" => Justification::Left,
        "center" => Justification::Center,
        "right" => Justification::Right,
        "both" => Justification::Both,
        "distribute" => Justification::Distribute,
        "start" => Justification::Start,
        "end" => Justification::End,
        other => Justification::Other(other.to_owned()),
    }))
}

// ---------------------------------------------------------------------------
// Перечисления OOXML → модель
// ---------------------------------------------------------------------------

/// `ST_LineSpacingRule`: как понимать `w:line`.
#[must_use]
fn parse_line_rule(raw: &str) -> LineSpacingRule {
    match raw {
        "auto" => LineSpacingRule::Auto,
        "exact" => LineSpacingRule::Exact,
        "atLeast" => LineSpacingRule::AtLeast,
        other => LineSpacingRule::Other(other.to_owned()),
    }
}

/// `ST_Hint`: каким шрифтом набирать символ.
#[must_use]
fn parse_font_hint(raw: &str) -> FontHint {
    match raw {
        "default" => FontHint::Default,
        "eastAsia" => FontHint::EastAsia,
        "cs" => FontHint::Cs,
        other => FontHint::Other(other.to_owned()),
    }
}

/// `ST_Border`: стиль линии границы.
#[must_use]
fn parse_border_style(raw: &str) -> BorderStyle {
    match raw {
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
        other => BorderStyle::Other(other.to_owned()),
    }
}

/// `ST_Shd`: узор заливки.
#[must_use]
fn parse_shading_pattern(raw: &str) -> ShadingPattern {
    match raw {
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
        other => ShadingPattern::Other(other.to_owned()),
    }
}

/// `ST_TabJc`: выравнивание текста на позиции табуляции.
#[must_use]
fn parse_tab_kind(raw: &str) -> TabStopKind {
    match raw {
        "bar" => TabStopKind::Bar,
        "center" => TabStopKind::Center,
        "clear" => TabStopKind::Clear,
        "decimal" => TabStopKind::Decimal,
        "end" => TabStopKind::End,
        "num" => TabStopKind::Num,
        "start" => TabStopKind::Start,
        "left" => TabStopKind::Left,
        "right" => TabStopKind::Right,
        other => TabStopKind::Other(other.to_owned()),
    }
}

/// `ST_TabTlc`: заполнитель промежутка до позиции табуляции.
#[must_use]
fn parse_tab_leader(raw: &str) -> TabLeader {
    match raw {
        "none" => TabLeader::None,
        "dot" => TabLeader::Dot,
        "hyphen" => TabLeader::Hyphen,
        "middleDot" => TabLeader::MiddleDot,
        "heavy" => TabLeader::Heavy,
        "underscore" => TabLeader::Underscore,
        other => TabLeader::Other(other.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Общие мелочи
// ---------------------------------------------------------------------------

/// Атрибут-число в диапазоне `0..=max` (`w:ilvl`, `w:outlineLvl`).
///
/// # Errors
/// [`Error::malformed`] — атрибут не читается; [`Error::TooManyWarnings`] — порог.
fn attr_range(
    attrs: &[Attr<'_>],
    name: &str,
    max: u8,
    ctx: &mut ParseCtx,
    part: &str,
    element: &str,
) -> Result<Option<u8>> {
    let Some(value) = attr_i32(attrs, name, ctx, part)? else {
        return Ok(None);
    };
    match u8::try_from(value) {
        Ok(value) if value <= max => Ok(Some(value)),
        _ => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`{element}`: `{value}` is outside 0..={max}, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Сохранить элемент целиком: пустой — из самого события, начатый — дочитав поддерево.
///
/// # Errors
/// То же, что у [`capture_element`] и [`utf8`].
fn capture_any(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<String> {
    if empty {
        capture_empty(element, part)
    } else {
        capture_element(reader, element, ctx, part)
    }
}

/// Сериализовать пустой элемент (`<w:foo a="b"/>`) обратно в XML.
///
/// # Errors
/// [`Error::malformed`] — байты тега не UTF-8.
fn capture_empty(element: &BytesStart<'_>, part: &str) -> Result<String> {
    let mut xml = String::new();
    xml.push('<');
    xml.push_str(&utf8(element, part)?);
    xml.push_str("/>");
    Ok(xml)
}

/// Имя элемента как в XML, вместе с префиксом (`w:tbl`) — для сообщений.
fn element_name<'a>(element: &'a BytesStart<'_>) -> Cow<'a, str> {
    String::from_utf8_lossy(element.name().into_inner())
}

/// Локальное имя элемента строкой (`tbl`) — ключ `unknown`.
fn local_name_of(element: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(local_name(element.name().into_inner())).into_owned()
}

/// Пропустить незначащий элемент целиком.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше `End` элемента.
fn skip_element(reader: &mut XmlReader<'_>, empty: bool, part: &str) -> Result<()> {
    if empty {
        return Ok(());
    }
    let mut depth: u32 = 0;
    loop {
        match reader.next_significant()? {
            Some(Event::Start(_)) => depth += 1,
            Some(Event::End(_)) if depth == 0 => return Ok(()),
            Some(Event::End(_)) => depth -= 1,
            Some(_) => {}
            None => {
                return Err(Error::malformed(
                    part,
                    "unexpected end of input inside a skipped element",
                ));
            }
        }
    }
}

/// Проверить, что следующий элемент — синтетический корень обёртки фрагмента.
///
/// # Errors
/// [`Error::malformed`] — фрагмент пуст или начинается не с корня.
fn expect_fragment_root(reader: &mut XmlReader<'_>, part: &str) -> Result<()> {
    match reader.next_significant()? {
        Some(Event::Start(root)) if local_name(root.name().into_inner()) == FRAGMENT_ROOT => Ok(()),
        _ => Err(Error::malformed(part, "malformed `mc:Choice` fragment")),
    }
}

/// Байты события как строка.
///
/// # Errors
/// [`Error::malformed`] — байты не UTF-8.
fn utf8(bytes: &[u8], part: &str) -> Result<String> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|e| Error::malformed(part, format!("event bytes are not UTF-8: {e}")))
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use doc_converter_core::{NodeId, ParseWarning};

    use crate::model::Toggle;

    /// Имя части — как в пакете: с ним приходят и предупреждения.
    const PART: &str = "word/document.xml";

    /// Обернуть тело в минимальный `w:document`; префиксы ридер не разыменовывает,
    /// поэтому объявлений пространств имён хватает символических.
    fn document_xml(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <w:document xmlns:w="w" xmlns:r="r" xmlns:mc="mc" xmlns:wps="wps">
            <w:body>{body}</w:body></w:document>"#
        )
    }

    /// Разобрать синтетический `w:body` и отдать модель вместе с предупреждениями.
    fn parse_xml(body: &str) -> (Body, Vec<ParseWarning>) {
        parse_part(document_xml(body).as_bytes())
    }

    /// Разобрать часть `word/document.xml` без связей.
    fn parse_part(xml: &[u8]) -> (Body, Vec<ParseWarning>) {
        parse_part_with(xml, &Relationships::default())
    }

    /// Разобрать часть вместе с её связями: без них не разрешить цели ссылок.
    fn parse_part_with(xml: &[u8], rels: &Relationships) -> (Body, Vec<ParseWarning>) {
        let mut ctx = ParseCtx::new();
        let body = parse(xml, rels, &mut ctx, PART).expect("the document parses");
        let warnings = ctx.warnings().to_vec();
        (body, warnings)
    }

    /// Связи части из троек `(r:id, тип, цель)`: тип берётся полным URI, как в XML.
    fn rels_from(entries: &[(&str, &str, &str)]) -> Relationships {
        let mut xml = String::from("<Relationships>");
        for (id, rel_type, target) in entries {
            let _ = write!(
                xml,
                r#"<Relationship Id="{id}" Type="{rel_type}" Target="{target}"/>"#
            );
        }
        xml.push_str("</Relationships>");
        let map = doc_converter_core::rels::RelMap::parse(xml.as_bytes()).expect("rels parse");
        Relationships::from_map(map)
    }

    /// Связи с внешними целями (`TargetMode="External"`): так записаны адреса
    /// гиперссылок — цель такой связи не разрешается в часть пакета.
    fn external_rels(entries: &[(&str, &str, &str)]) -> Relationships {
        let mut xml = String::from("<Relationships>");
        for (id, rel_type, target) in entries {
            let _ = write!(
                xml,
                r#"<Relationship Id="{id}" Type="{rel_type}" Target="{target}" TargetMode="External"/>"#
            );
        }
        xml.push_str("</Relationships>");
        let map = doc_converter_core::rels::RelMap::parse(xml.as_bytes()).expect("rels parse");
        Relationships::from_map(map)
    }

    /// Плоский текст абзаца — то, что сверяют сайдкары фикстур.
    fn paragraph_text(paragraph: &Paragraph) -> String {
        let mut text = String::new();
        for inline in &paragraph.runs {
            let Inline::Run(run) = inline else {
                continue;
            };
            for content in &run.content {
                match content {
                    RunContent::Text(chunk) => text.push_str(chunk),
                    RunContent::Tab => text.push('\t'),
                    RunContent::Break(BreakKind::Line) => text.push('\n'),
                    _ => {}
                }
            }
        }
        text
    }

    /// Абзацы тела по порядку.
    fn paragraphs(body: &Body) -> Vec<&Paragraph> {
        body.items
            .iter()
            .filter_map(|item| match item {
                BlockItem::Paragraph(paragraph) => Some(paragraph),
                _ => None,
            })
            .collect()
    }

    /// Единственный run единственного абзаца — для коротких проверок содержимого.
    fn only_run(body: &Body) -> &Run {
        let paragraph = paragraphs(body).first().copied().expect("a paragraph");
        match &paragraph.runs[0] {
            Inline::Run(run) => run,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn paragraph_keeps_text_and_ids() {
        let (body, warnings) = parse_xml("<w:p><w:r><w:t>Hello</w:t></w:r></w:p>");

        assert!(warnings.is_empty(), "{warnings:?}");
        let paragraph = paragraphs(&body).first().copied().expect("a paragraph");
        assert_eq!(paragraph_text(paragraph), "Hello");
        // Тело получило ID 1, абзац — 2, run — 3 (ADR-0019 §2).
        assert_eq!(body.id, NodeId::new(1));
        assert_eq!(paragraph.id, NodeId::new(2));
        assert_eq!(only_run(&body).id, NodeId::new(3));
    }

    #[test]
    fn whitespace_and_entities_survive_the_round_trip() {
        let (body, warnings) =
            parse_xml(r#"<w:p><w:r><w:t xml:space="preserve"> a  &amp; b </w:t></w:r></w:p>"#);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(paragraph_text(paragraphs(&body)[0]), " a  & b ");
    }

    #[test]
    fn character_references_are_resolved() {
        let (body, _) = parse_xml("<w:p><w:r><w:t>&#x41;&#66;</w:t></w:r></w:p>");

        assert_eq!(paragraph_text(paragraphs(&body)[0]), "AB");
    }

    #[test]
    fn ppr_keeps_style_numbering_and_formatting() {
        let (body, warnings) = parse_xml(
            r#"<w:p>
                <w:pPr>
                    <w:pStyle w:val="Heading1"/>
                    <w:numPr><w:ilvl w:val="1"/><w:numId w:val="3"/></w:numPr>
                    <w:spacing w:before="240" w:after="120" w:line="360" w:lineRule="auto"
                               w:beforeLines="50" w:afterAutospacing="1"/>
                    <w:ind w:start="720" w:end="360" w:firstLine="240"/>
                    <w:jc w:val="center"/>
                    <w:keepNext/><w:keepLines w:val="0"/><w:pageBreakBefore w:val="true"/>
                    <w:widowControl w:val="off"/>
                    <w:outlineLvl w:val="2"/>
                    <w:shd w:val="solid" w:color="auto" w:fill="FFFF00"/>
                    <w:tabs>
                        <w:tab w:val="center" w:pos="2880" w:leader="dot"/>
                        <w:tab w:val="clear" w:pos="4320"/>
                    </w:tabs>
                    <w:pBdr>
                        <w:top w:val="single" w:sz="4" w:space="1" w:color="FF0000"/>
                        <w:bar w:val="dotted"/>
                    </w:pBdr>
                    <w:rPr><w:b/></w:rPr>
                </w:pPr>
            </w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let paragraph = paragraphs(&body)[0];
        assert_eq!(
            paragraph.style_ref.as_ref().map(StyleId::as_str),
            Some("Heading1")
        );
        assert_eq!(
            paragraph.ppr.style.as_ref().map(StyleId::as_str),
            Some("Heading1")
        );
        assert_eq!(paragraph.numbering_ref, Some(NumId::new(3)));
        let num_pr = paragraph.ppr.num_pr.as_ref().expect("`w:numPr` is kept");
        assert_eq!(num_pr.ilvl, Some(1));
        let spacing = paragraph.ppr.spacing.as_ref().expect("`w:spacing` is kept");
        assert_eq!(spacing.before, Some(Twips::new(240)));
        assert_eq!(spacing.after, Some(Twips::new(120)));
        assert_eq!(spacing.line, Some(LineSpacing::new(360)));
        assert_eq!(spacing.line_rule, Some(LineSpacingRule::Auto));
        assert_eq!(spacing.before_lines, Some(50));
        assert!(spacing.after_autospacing);
        let ind = paragraph.ppr.ind.as_ref().expect("`w:ind` is kept");
        assert_eq!(ind.left, Some(Twips::new(720)));
        assert_eq!(ind.right, Some(Twips::new(360)));
        assert_eq!(ind.first_line, Some(Twips::new(240)));
        assert_eq!(paragraph.ppr.jc, Some(Justification::Center));
        assert_eq!(paragraph.ppr.keep_next, Some(Toggle::On));
        assert_eq!(paragraph.ppr.keep_lines, Some(Toggle::Off));
        assert_eq!(paragraph.ppr.page_break_before, Some(Toggle::On));
        assert_eq!(paragraph.ppr.widow_control, Some(Toggle::Off));
        assert_eq!(paragraph.ppr.outline_lvl, Some(2));
        let shading = paragraph.ppr.shd.as_ref().expect("`w:shd` is kept");
        assert_eq!(shading.val, ShadingPattern::Solid);
        assert_eq!(shading.fill, Some(Color::Rgb(0x00ff_ff00)));
        assert_eq!(paragraph.ppr.tabs.len(), 2);
        assert_eq!(paragraph.ppr.tabs[0].kind, TabStopKind::Center);
        assert_eq!(paragraph.ppr.tabs[0].leader, TabLeader::Dot);
        assert_eq!(paragraph.ppr.tabs[0].val, Twips::new(2880));
        assert_eq!(paragraph.ppr.tabs[1].kind, TabStopKind::Clear);
        let borders = paragraph.ppr.p_bdr.as_ref().expect("`w:pBdr` is kept");
        let top = borders.top.as_ref().expect("the top border is kept");
        assert_eq!(top.val, BorderStyle::Single);
        assert_eq!(top.sz, Some(4));
        assert_eq!(top.color, Some(Color::Rgb(0x00ff_0000)));
        assert_eq!(
            borders.bar.as_ref().map(|border| border.val.clone()),
            Some(BorderStyle::Dotted)
        );
        // `w:rPr` внутри `w:pPr` — свойства знака абзаца, а не runs.
        assert_eq!(paragraph.mark_rpr.b, Some(Toggle::On));
        assert_eq!(paragraph.ppr.r_pr, Some((*paragraph.mark_rpr).clone()));
    }

    #[test]
    fn rpr_reads_all_supported_properties() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:r>
                <w:rPr>
                    <w:rStyle w:val="Strong"/>
                    <w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="MS Mincho"
                              w:cs="Arial" w:hint="eastAsia"/>
                    <w:b/><w:i w:val="0"/><w:caps/><w:smallCaps/><w:strike/><w:dstrike/>
                    <w:vanish/><w:outline/><w:shadow/><w:emboss/><w:imprint/>
                    <w:color w:val="00FF00"/><w:sz w:val="24"/><w:szCs w:val="28"/>
                    <w:highlight w:val="yellow"/><w:u w:val="double"/>
                    <w:vertAlign w:val="superscript"/><w:spacing w:val="20"/>
                    <w:position w:val="-6"/>
                </w:rPr>
                <w:t>x</w:t>
            </w:r></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let run = only_run(&body);
        assert_eq!(run.style_ref.as_ref().map(StyleId::as_str), Some("Strong"));
        let rpr = &run.rpr;
        let fonts = rpr.r_fonts.as_ref().expect("`w:rFonts` is kept");
        assert_eq!(fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(fonts.east_asia.as_deref(), Some("MS Mincho"));
        assert_eq!(fonts.hint, Some(FontHint::EastAsia));
        assert_eq!(rpr.b, Some(Toggle::On));
        assert_eq!(rpr.i, Some(Toggle::Off));
        assert_eq!(rpr.caps, Some(Toggle::On));
        assert_eq!(rpr.small_caps, Some(Toggle::On));
        assert_eq!(rpr.strike, Some(Toggle::On));
        assert_eq!(rpr.dstrike, Some(Toggle::On));
        assert_eq!(rpr.vanish, Some(Toggle::On));
        assert_eq!(rpr.outline, Some(Toggle::On));
        assert_eq!(rpr.shadow, Some(Toggle::On));
        assert_eq!(rpr.emboss, Some(Toggle::On));
        assert_eq!(rpr.imprint, Some(Toggle::On));
        assert_eq!(rpr.color, Some(Color::Rgb(0x0000_ff00)));
        assert_eq!(rpr.sz, Some(HalfPoint::new(24)));
        assert_eq!(rpr.sz_cs, Some(HalfPoint::new(28)));
        assert_eq!(rpr.highlight, Some(Highlight::Yellow));
        assert_eq!(rpr.u, Some(Underline::Double));
        assert_eq!(rpr.vert_align, Some(VertAlign::Superscript));
        let spacing = rpr.spacing.as_ref().expect("`w:spacing` is kept");
        assert_eq!(spacing.value, Some(Twips::new(20)));
        assert_eq!(rpr.position, Some(HalfPoint::new(-6)));
    }

    #[test]
    fn breaks_tabs_and_symbols_are_read() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:r>
                <w:br/><w:cr/><w:br w:type="line"/><w:br w:type="page"/>
                <w:br w:type="column"/><w:br w:type="textWrapping"/><w:br w:type="weird"/>
                <w:tab/><w:sym w:font="Wingdings" w:char="F0E0"/>
            </w:r></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let content = &only_run(&body).content;
        assert_eq!(
            content,
            &vec![
                RunContent::Break(BreakKind::Line),
                RunContent::Break(BreakKind::Line),
                RunContent::Break(BreakKind::Unsupported("line".to_owned())),
                RunContent::Break(BreakKind::Page),
                RunContent::Break(BreakKind::Column),
                RunContent::Break(BreakKind::Line),
                RunContent::Break(BreakKind::Unsupported("weird".to_owned())),
                RunContent::Tab,
                RunContent::Symbol {
                    font: "Wingdings".to_owned(),
                    char: '\u{f0e0}',
                },
            ]
        );
    }

    #[test]
    fn a_broken_symbol_is_skipped_with_a_warning() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:r>
                <w:sym w:font="Wingdings" w:char="D800"/>
                <w:sym w:font="Wingdings" w:char="not hex"/>
                <w:sym w:char="F0E0"/>
            </w:r></w:p>"#,
        );

        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|warning| warning.kind == WarningKind::InvalidAttribute));
        assert!(only_run(&body).content.is_empty());
    }

    #[test]
    fn hyphens_are_text() {
        let (body, warnings) = parse_xml("<w:p><w:r><w:noBreakHyphen/><w:softHyphen/></w:r></w:p>");

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(paragraph_text(paragraphs(&body)[0]), "\u{2011}\u{00ad}");
    }

    #[test]
    fn unknown_blocks_and_runs_are_kept_as_xml() {
        let (body, warnings) = parse_xml(
            r#"<w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt>
               <w:p><w:r><w:pict/><w:footnoteReference w:id="1"/></w:r></w:p>"#,
        );

        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|warning| warning.kind == WarningKind::UnknownElement));
        match &body.items[0] {
            BlockItem::Unknown { xml, .. } => {
                assert_eq!(xml, "<w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt>");
            }
            other => panic!("expected an unknown block, got {other:?}"),
        }
        let content = &only_run(&body).content;
        assert_eq!(content.len(), 2);
        match &content[0] {
            RunContent::Unknown { xml, .. } => assert_eq!(xml, "<w:pict/>"),
            other => panic!("expected an unknown run content, got {other:?}"),
        }
    }

    #[test]
    fn unknown_properties_stay_in_the_unknown_lists() {
        let (body, warnings) =
            parse_xml("<w:p><w:pPr><w:contextualSpacing/><w:rPr><w:rtl/></w:rPr></w:pPr></w:p>");

        assert_eq!(warnings.len(), 2, "{warnings:?}");
        let paragraph = paragraphs(&body)[0];
        assert_eq!(paragraph.ppr.unknown.len(), 1);
        assert_eq!(paragraph.ppr.unknown[0].0, "contextualSpacing");
        assert_eq!(paragraph.ppr.unknown[0].1, "<w:contextualSpacing/>");
        let rpr = paragraph.ppr.r_pr.as_ref().expect("`w:rPr` is kept");
        assert_eq!(rpr.unknown, vec![("rtl".to_owned(), "<w:rtl/>".to_owned())]);
    }

    #[test]
    fn body_section_properties_become_a_section() {
        let (body, warnings) = parse_xml(
            r#"<w:p/><w:sectPr>
                 <w:type w:val="continuous"/>
                 <w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>
                 <w:pgMar w:top="720" w:header="360"/>
                 <w:cols w:num="2" w:space="425" w:equalWidth="0" w:sep="1">
                   <w:col w:w="4000" w:space="425"/>
                 </w:cols>
                 <w:titlePg/>
               </w:sectPr>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let BlockItem::SectPr(properties) = &body.items[1] else {
            panic!(
                "expected section properties in the body, got {:?}",
                body.items[1]
            );
        };
        assert_eq!(properties.orientation, Orientation::Landscape);
        assert_eq!(properties.page_size.width.value(), 16838);
        assert_eq!(properties.page_size.height.value(), 11906);
        assert_eq!(properties.margins.top.value(), 720);
        // Незаданные поля берут умолчание схемы, а не ноль.
        assert_eq!(properties.margins.right.value(), DEFAULT_MARGIN);
        assert_eq!(properties.margins.header, Some(Twips::new(360)));
        assert_eq!(
            properties.margins.footer,
            Some(Twips::new(DEFAULT_HEADER_MARGIN))
        );
        assert_eq!(properties.columns.count, 2);
        assert!(!properties.columns.equal_width);
        assert!(properties.columns.separator);
        assert_eq!(properties.columns.defs.len(), 1);
        assert_eq!(properties.columns.defs[0].width.value(), 4000);
        assert!(properties.title_pg);
        assert_eq!(properties.section_type, Some(SectionType::Continuous));

        // Секция собирается из тех же свойств: удобные поля их повторяют.
        assert_eq!(body.sections.len(), 1);
        let section = &body.sections[0];
        assert_eq!(section.properties, *properties);
        assert_eq!(section.orientation, Orientation::Landscape);
        assert_eq!(section.page_size, properties.page_size);
        assert_eq!(section.margins, properties.margins);
        assert_eq!(section.columns, properties.columns);
        assert!(section.title_pg);
    }

    #[test]
    fn a_section_break_in_a_paragraph_ends_its_section() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr>
                 <w:r><w:t>one</w:t></w:r></w:p>
               <w:p><w:r><w:t>two</w:t></w:r></w:p>
               <w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/></w:sectPr>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        // Разрыв стоит в `w:pPr`, а не среди блоков: конец секции абзац не разрывает.
        assert_eq!(body.items.len(), 3);
        let properties = paragraphs(&body)[0]
            .section_break
            .as_ref()
            .expect("the first paragraph ends a section");
        assert_eq!(properties.page_size.width.value(), 11906);

        // Порядок секций — порядок концов: сначала разрыв в абзаце, потом конец тела.
        assert_eq!(body.sections.len(), 2);
        assert_eq!(body.sections[0].orientation, Orientation::Portrait);
        assert_eq!(body.sections[0].page_size.width.value(), 11906);
        assert_eq!(body.sections[1].orientation, Orientation::Landscape);
        assert_eq!(body.sections[1].page_size.width.value(), 16838);
    }

    #[test]
    fn header_and_footer_references_resolve_through_rels() {
        let rels = rels_from(&[
            (
                "rIdHeader1",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header",
                "header1.xml",
            ),
            (
                "rIdFooter1",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
                "footer1.xml",
            ),
        ]);
        let (body, warnings) = parse_part_with(
            document_xml(
                r#"<w:sectPr>
                     <w:headerReference w:type="default" r:id="rIdHeader1"/>
                     <w:headerReference w:type="first" r:id="rIdHeader1"/>
                     <w:footerReference w:type="even" r:id="rIdFooter1"/>
                   </w:sectPr>"#,
            )
            .as_bytes(),
            &rels,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let properties = &body.sections[0].properties;
        let header = properties
            .header_default
            .as_ref()
            .expect("a default header");
        assert_eq!(header.rel_id, "rIdHeader1");
        assert_eq!(header.target, "word/header1.xml");
        assert!(!header.external);
        let first = properties
            .header_first
            .as_ref()
            .expect("a first-page header");
        assert_eq!(first.target, "word/header1.xml");
        assert!(properties.header_even.is_none());
        let footer = properties.footer_even.as_ref().expect("an even footer");
        assert_eq!(footer.target, "word/footer1.xml");
        // Удобные поля секции повторяют ссылки.
        assert_eq!(body.sections[0].header_default.as_ref(), Some(header));
    }

    #[test]
    fn an_undeclared_header_reference_warns_and_is_dropped() {
        let (body, warnings) = parse_xml(
            r#"<w:sectPr><w:headerReference w:type="default" r:id="rId404"/></w:sectPr>"#,
        );

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::MissingPart);
        assert!(body.sections[0].header_default.is_none());
    }

    #[test]
    fn doc_grid_is_kept_without_a_warning() {
        // `w:docGrid` есть почти в каждом документе Word: предупреждение о нём
        // было бы шумом, но XML обязан сохраниться.
        let (body, warnings) = parse_xml(
            r#"<w:sectPr><w:pgSz w:w="11906"/><w:docGrid w:linePitch="360"/></w:sectPr>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let BlockItem::SectPr(properties) = &body.items[0] else {
            panic!("expected section properties, got {:?}", body.items[0]);
        };
        assert_eq!(
            properties.unknown,
            vec![(
                "docGrid".to_owned(),
                r#"<w:docGrid w:linePitch="360"/>"#.to_owned()
            )]
        );
    }

    #[test]
    fn an_unsupported_section_property_warns_and_is_kept() {
        let (body, warnings) = parse_xml(
            r#"<w:sectPr><w:pgSz w:w="11906"/><w:textDirection w:val="tbRl"/></w:sectPr>"#,
        );

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::UnknownElement);
        let BlockItem::SectPr(properties) = &body.items[0] else {
            panic!("expected section properties, got {:?}", body.items[0]);
        };
        assert_eq!(properties.unknown.len(), 1);
        assert_eq!(properties.unknown[0].0, "textDirection");
    }

    #[test]
    fn a_section_without_properties_gets_the_schema_defaults() {
        let (body, warnings) = parse_xml("<w:sectPr/>");

        assert!(warnings.is_empty(), "{warnings:?}");
        let section = &body.sections[0];
        assert_eq!(section.orientation, Orientation::Portrait);
        assert_eq!(section.page_size.width, Twips::new(DEFAULT_PAGE_WIDTH));
        assert_eq!(section.page_size.height, Twips::new(DEFAULT_PAGE_HEIGHT));
        assert_eq!(section.columns.count, 1);
        assert!(section.columns.equal_width);
    }

    #[test]
    fn ignored_inline_markup_does_not_produce_warnings() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:proofErr w:type="spellStart"/><w:lastRenderedPageBreak/>
               <w:r><w:t>text</w:t></w:r></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(paragraph_text(paragraphs(&body)[0]), "text");
    }

    #[test]
    fn a_hyperlink_takes_its_external_target_from_rels() {
        let rels = external_rels(&[(
            "rId4",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
            "https://example.com/page",
        )]);
        let (body, warnings) = parse_part_with(
            document_xml(
                r#"<w:p><w:hyperlink r:id="rId4" w:tooltip="hint">
                     <w:r><w:t>link</w:t></w:r>
                   </w:hyperlink></w:p>"#,
            )
            .as_bytes(),
            &rels,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let paragraph = paragraphs(&body)[0];
        match &paragraph.runs[0] {
            Inline::Hyperlink(link) => {
                assert_eq!(link.rel_id.as_deref(), Some("rId4"));
                // Цель внешней ссылки — это адрес, а не часть пакета.
                assert_eq!(link.target.as_deref(), Some("https://example.com/page"));
                assert_eq!(link.tooltip.as_deref(), Some("hint"));
                assert!(link.external);
                assert_eq!(link.anchor, None);
                // Содержимое ссылки — обычные run'ы, текст не теряется.
                match &link.runs[0] {
                    Inline::Run(run) => {
                        assert_eq!(run.content, vec![RunContent::Text("link".to_owned())]);
                    }
                    other => panic!("expected a run, got {other:?}"),
                }
            }
            other => panic!("expected a hyperlink, got {other:?}"),
        }
    }

    #[test]
    fn an_internal_hyperlink_keeps_its_anchor() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:hyperlink w:anchor="mark"><w:r><w:t>jump</w:t></w:r></w:hyperlink></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        match &paragraphs(&body)[0].runs[0] {
            Inline::Hyperlink(link) => {
                // `w:anchor` ведёт внутрь документа: связи и внешней цели нет.
                assert!(!link.external);
                assert_eq!(link.anchor.as_deref(), Some("mark"));
                assert_eq!(link.rel_id, None);
                assert_eq!(link.target, None);
            }
            other => panic!("expected a hyperlink, got {other:?}"),
        }
    }

    #[test]
    fn a_hyperlink_without_a_relationship_warns() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:hyperlink r:id="rId404"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>"#,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::MissingPart);
        match &paragraphs(&body)[0].runs[0] {
            Inline::Hyperlink(link) => {
                assert_eq!(link.rel_id.as_deref(), Some("rId404"));
                assert_eq!(link.target, None);
            }
            other => panic!("expected a hyperlink, got {other:?}"),
        }
    }

    #[test]
    fn a_bookmark_pair_is_kept_and_an_orphan_end_warns() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:bookmarkStart w:id="3" w:name="mark"/><w:r><w:t>text</w:t></w:r>
               <w:bookmarkEnd w:id="3"/><w:bookmarkEnd w:id="9"/></w:p>"#,
        );

        let paragraph = paragraphs(&body)[0];
        // Закладка — начало с именем; парный `w:bookmarkEnd` в модель не попадает,
        // а непарный не попадает ещё и с предупреждением.
        assert_eq!(paragraph.runs.len(), 2);
        match &paragraph.runs[0] {
            Inline::Bookmark(bookmark) => {
                assert_eq!(bookmark.name, "mark");
                assert_eq!(bookmark.bookmark_id, 3);
            }
            other => panic!("expected a bookmark, got {other:?}"),
        }
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::OrphanBookmark);
        assert_eq!(paragraph_text(paragraph), "text");
    }

    #[test]
    fn a_simple_field_keeps_its_instruction_and_result() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:fldSimple w:instr=" PAGE " w:dirty="true"><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        match &paragraphs(&body)[0].runs[0] {
            Inline::Field(field) => {
                assert_eq!(field.kind, FieldKind::Simple);
                // Инструкция сохраняется как есть, вместе с пробелами.
                assert_eq!(field.instruction, " PAGE ");
                assert!(field.dirty);
                assert_eq!(field.result.len(), 1);
            }
            other => panic!("expected a field, got {other:?}"),
        }
    }

    #[test]
    fn a_complex_field_is_assembled_from_neighbouring_runs() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>
               <w:r><w:instrText xml:space="preserve"> REF mark \h </w:instrText></w:r>
               <w:r><w:fldChar w:fldCharType="separate"/></w:r>
               <w:r><w:t>5</w:t></w:r>
               <w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );

        // Разметка поля в модель не попадает и предупреждений не оставляет.
        assert!(warnings.is_empty(), "{warnings:?}");
        let paragraph = paragraphs(&body)[0];
        assert_eq!(paragraph.runs.len(), 1);
        match &paragraph.runs[0] {
            Inline::Field(field) => {
                assert_eq!(field.kind, FieldKind::Complex);
                assert_eq!(field.instruction, " REF mark \\h ");
                assert!(!field.dirty);
                // В результате — только содержимое run'ов, без самой разметки.
                assert_eq!(field.result.len(), 1);
                match &field.result[0] {
                    Inline::Run(run) => {
                        assert_eq!(run.content, vec![RunContent::Text("5".to_owned())]);
                    }
                    other => panic!("expected a run, got {other:?}"),
                }
            }
            other => panic!("expected a complex field, got {other:?}"),
        }
    }

    #[test]
    fn an_unclosed_complex_field_returns_its_fragments_as_unknown() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>
               <w:r><w:instrText>PAGE</w:instrText></w:r></w:p>"#,
        );

        // Поля не вышло: run'ы возвращаются в поток, а их разметка — как `Unknown`.
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnknownElement);
        let paragraph = paragraphs(&body)[0];
        assert_eq!(paragraph.runs.len(), 2);
        for inline in &paragraph.runs {
            let Inline::Run(run) = inline else {
                panic!("expected a run, got {inline:?}");
            };
            assert!(
                matches!(run.content.as_slice(), [RunContent::Unknown { .. }]),
                "{:?}",
                run.content
            );
        }
    }

    #[test]
    fn tracked_changes_are_kept_as_unknown_xml() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:ins w:id="1" w:author="a"><w:r><w:t>added</w:t></w:r></w:ins>
               <w:del w:id="2" w:author="a"><w:r><w:delText>gone</w:delText></w:r></w:del></w:p>"#,
        );

        assert_eq!(warnings.len(), 2);
        assert!(warnings
            .iter()
            .all(|warning| warning.kind == WarningKind::UnknownElement));
        let paragraph = paragraphs(&body)[0];
        assert_eq!(paragraph.runs.len(), 2);
        match &paragraph.runs[0] {
            Inline::Unknown { xml, .. } => assert_eq!(
                xml,
                r#"<w:ins w:id="1" w:author="a"><w:r><w:t>added</w:t></w:r></w:ins>"#
            ),
            other => panic!("expected an unknown inline, got {other:?}"),
        }
        // Удалённый текст остаётся внутри XML и не притворяется обычным текстом.
        match &paragraph.runs[1] {
            Inline::Unknown { xml, .. } => assert!(xml.contains("<w:delText>gone</w:delText>")),
            other => panic!("expected an unknown inline, got {other:?}"),
        }
        assert_eq!(paragraph_text(paragraph), "");
    }

    #[test]
    fn a_pict_is_kept_as_unknown() {
        let (body, warnings) =
            parse_xml(r#"<w:p><w:r><w:pict><v:shape id="s1"/></w:pict></w:r></w:p>"#);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnknownElement);
        let run = only_run(&body);
        match &run.content[0] {
            RunContent::Unknown { xml, .. } => {
                assert_eq!(xml, r#"<w:pict><v:shape id="s1"/></w:pict>"#);
            }
            other => panic!("expected unknown run content, got {other:?}"),
        }
    }

    #[test]
    fn alternate_content_choice_is_inlined_on_both_levels() {
        let (body, warnings) = parse_xml(
            r#"<mc:AlternateContent>
                 <mc:Choice Requires="wps"><w:p><w:r><w:t>chosen</w:t></w:r></w:p></mc:Choice>
                 <mc:Fallback><w:p><w:r><w:t>fallback</w:t></w:r></w:p></mc:Fallback>
               </mc:AlternateContent>
               <w:p><w:r><mc:AlternateContent>
                 <mc:Choice Requires="wps"><w:t>inline</w:t></mc:Choice>
                 <mc:Fallback><w:t>fallback</w:t></mc:Fallback>
               </mc:AlternateContent></w:r></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(paragraph_text(paragraphs(&body)[0]), "chosen");
        assert_eq!(paragraph_text(paragraphs(&body)[1]), "inline");
    }

    #[test]
    fn nested_alternate_content_is_resolved_once() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:r><mc:AlternateContent>
                 <mc:Choice Requires="wps"><mc:AlternateContent>
                   <mc:Choice Requires="w14"><w:t>nested</w:t></mc:Choice>
                   <mc:Fallback><w:t>nested fallback</w:t></mc:Fallback>
                 </mc:AlternateContent></mc:Choice>
                 <mc:Fallback><w:t>fallback</w:t></mc:Fallback>
               </mc:AlternateContent></w:r></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(paragraph_text(paragraphs(&body)[0]), "nested");
    }

    #[test]
    fn unsupported_alternate_content_is_kept_with_a_warning() {
        let (body, warnings) = parse_xml(
            r#"<mc:AlternateContent>
                 <mc:Choice Requires="v"><w:p><w:r><w:t>shape</w:t></w:r></w:p></mc:Choice>
                 <mc:Fallback><w:p><w:r><w:t>fallback</w:t></w:r></w:p></mc:Fallback>
               </mc:AlternateContent>"#,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnknownElement);
        match &body.items[0] {
            BlockItem::Unknown { xml, .. } => {
                assert!(xml.starts_with("<mc:AlternateContent>"), "{xml}");
                assert!(xml.contains("mc:Fallback"), "{xml}");
            }
            other => panic!("expected an unknown block, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_paragraph_and_run_stay_empty() {
        let (body, warnings) = parse_xml("<w:p/><w:p><w:r/></w:p>");

        assert!(warnings.is_empty(), "{warnings:?}");
        let paragraph = paragraphs(&body)[1];
        // Тело — 1, первый абзац — 2, второй — 3, его run — 4.
        assert_eq!(paragraph.id, NodeId::new(3));
        let Inline::Run(run) = &paragraph.runs[0] else {
            panic!("expected a run, got {:?}", paragraph.runs[0]);
        };
        assert_eq!(run.id, NodeId::new(4));
        assert!(run.content.is_empty());
    }

    #[test]
    fn a_document_without_a_body_is_malformed() {
        let err = parse(
            b"<w:document/>",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
        )
        .expect_err("a document without `w:body` does not parse");

        assert!(err.to_string().contains("w:document/w:body"), "{err}");
    }

    // -----------------------------------------------------------------------
    // Таблицы
    // -----------------------------------------------------------------------

    /// Таблицы тела по порядку.
    fn tables(body: &Body) -> Vec<&Table> {
        body.items
            .iter()
            .filter_map(|item| match item {
                BlockItem::Table(table) => Some(table),
                _ => None,
            })
            .collect()
    }

    /// Текст ячейки: абзацы склеиваются через пробел, вложенные таблицы
    /// пропускаются — так же считает сайкар (`nested`, `alignment_widths`).
    fn cell_text(cell: &Cell) -> String {
        let paragraphs = cell
            .items
            .iter()
            .filter_map(|item| match item {
                BlockItem::Paragraph(paragraph) => Some(paragraph_text(paragraph)),
                _ => None,
            })
            .collect::<Vec<_>>();
        paragraphs.join(" ")
    }

    #[test]
    fn table_properties_rows_and_cells_are_read() {
        let (body, warnings) = parse_xml(
            r#"<w:tbl>
                 <w:tblPr>
                   <w:tblStyle w:val="TableGrid"/>
                   <w:tblW w:w="5000" w:type="dxa"/>
                   <w:tblLayout w:type="fixed"/>
                   <w:tblBorders>
                     <w:top w:val="single" w:sz="8" w:space="0" w:color="2F5496"/>
                     <w:insideV w:val="dotted"/>
                   </w:tblBorders>
                   <w:tblLook w:firstRow="1" w:noVBand="1"/>
                   <w:jc w:val="center"/>
                   <w:tblInd w:w="120"/>
                   <w:tblCellMar><w:left w:w="108"/><w:top w:w="0"/></w:tblCellMar>
                 </w:tblPr>
                 <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="3000"/></w:tblGrid>
                 <w:tr>
                   <w:trPr>
                     <w:trHeight w:val="400" w:hRule="atLeast"/><w:cantSplit/><w:tblHeader/>
                   </w:trPr>
                   <w:tc>
                     <w:tcPr>
                       <w:tcW w:w="2000" w:type="dxa"/>
                       <w:gridSpan w:val="2"/>
                       <w:vMerge w:val="restart"/>
                       <w:vAlign w:val="center"/>
                       <w:tcBorders><w:bottom w:val="double" w:sz="6" w:color="C00000"/></w:tcBorders>
                       <w:shd w:val="clear" w:fill="DEEAF6"/>
                       <w:tcMar><w:right w:w="57"/></w:tcMar>
                     </w:tcPr>
                     <w:p><w:r><w:t>A1</w:t></w:r></w:p>
                   </w:tc>
                 </w:tr>
               </w:tbl>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let table = tables(&body)[0];
        assert_eq!(
            table.style_ref.as_ref().map(StyleId::as_str),
            Some("TableGrid")
        );
        assert_eq!(table.layout, TableLayout::Fixed);
        assert_eq!(table.width, Some(TableWidth::Dxa(Twips::new(5000))));
        assert_eq!(table.jc, Some(Justification::Center));
        assert_eq!(table.indent, Some(Twips::new(120)));
        assert!(table.look.first_row);
        assert!(table.look.no_v_band);
        assert!(!table.look.last_row && !table.look.no_h_band);
        assert_eq!(table.cell_margins.left, Some(Twips::new(108)));
        assert_eq!(table.cell_margins.top, Some(Twips::new(0)));
        assert_eq!(
            table
                .grid
                .iter()
                .map(|col| col.width.value())
                .collect::<Vec<_>>(),
            vec![2000, 3000]
        );
        assert_eq!(
            table.borders.top.as_ref().map(|b| b.val.clone()),
            Some(BorderStyle::Single)
        );
        assert_eq!(
            table.borders.inside_v.as_ref().map(|b| b.val.clone()),
            Some(BorderStyle::Dotted)
        );

        let row = &table.rows[0];
        assert_eq!(
            row.height,
            Some(RowHeight {
                value: Twips::new(400),
                rule: HeightRule::AtLeast,
            })
        );
        assert!(row.cant_split);
        assert!(row.header);

        let cell = &row.cells[0];
        assert_eq!(cell.grid_span, 2);
        assert_eq!(cell.v_merge, Some(VMerge::Restart));
        assert_eq!(cell.v_align, CellVAlign::Center);
        assert_eq!(cell.width, Some(CellWidth::Dxa(Twips::new(2000))));
        assert_eq!(cell.margins.right, Some(Twips::new(57)));
        let bottom = cell.borders.bottom.as_ref().expect("`w:bottom` is kept");
        assert_eq!(bottom.val, BorderStyle::Double);
        assert_eq!(bottom.sz, Some(6));
        assert_eq!(bottom.color, Some(Color::Rgb(0x00C0_0000)));
        assert_eq!(
            cell.shading.as_ref().and_then(|shd| shd.fill),
            Some(Color::Rgb(0x00DE_EAF6))
        );
        assert_eq!(cell_text(cell), "A1");
    }

    #[test]
    fn a_v_merge_without_a_value_continues_the_merge() {
        // Так это и пишет Word: `restart` — первой ячейке области, а
        // продолжениям — `w:vMerge` без `w:val` (умолчание ST_Merge).
        let (body, warnings) = parse_xml(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr></w:tc></w:tr>
               <w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr></w:tc></w:tr></w:tbl>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let table = tables(&body)[0];
        assert_eq!(table.rows[0].cells[0].v_merge, Some(VMerge::Restart));
        assert_eq!(table.rows[1].cells[0].v_merge, Some(VMerge::Continue));
    }

    #[test]
    fn table_look_reads_the_legacy_bitmask() {
        let (body, warnings) = parse_xml(
            r#"<w:tbl><w:tblPr><w:tblLook w:val="04A0"/></w:tblPr><w:tr><w:tc/></w:tr></w:tbl>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let body_tables = tables(&body);
        let look = &body_tables[0].look;
        // Сайкар `tables/tbl_look` ждёт от `04A0` ровно эти признаки.
        assert!(look.first_row && look.first_column && look.no_v_band);
        assert!(!look.last_row && !look.last_column && !look.no_h_band);
    }

    #[test]
    fn nested_tables_get_their_own_ids() {
        let (body, warnings) = parse_xml(
            r"<w:tbl><w:tr><w:tc>
                 <w:p><w:r><w:t>outer</w:t></w:r></w:p>
                 <w:tbl><w:tr><w:tc><w:p><w:r><w:t>inner</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
               </w:tc></w:tr></w:tbl>",
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        let outer = tables(&body)[0];
        let items = &outer.rows[0].cells[0].items;
        assert_eq!(items.len(), 2, "{items:?}");
        let nested = match &items[1] {
            BlockItem::Table(table) => table,
            other => panic!("expected a nested table, got {other:?}"),
        };
        assert_eq!(cell_text(&nested.rows[0].cells[0]), "inner");

        // Пространство `NodeId` общее: идентификаторы вложенной таблицы не
        // совпадают ни с внешней, ни с её строкой и ячейкой.
        let mut ids = vec![
            outer.id,
            outer.rows[0].id,
            outer.rows[0].cells[0].id,
            nested.id,
        ];
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4, "идентификаторы узлов уникальны");
    }

    #[test]
    fn unsupported_table_parts_warn_and_do_not_break_the_table() {
        let (body, warnings) = parse_xml(
            r#"<w:tbl>
                 <w:tblPr><w:tblpPr w:leftFromText="10"/></w:tblPr>
                 <w:tr><w:tc><w:tcPr><w:textDirection w:val="btLr"/></w:tcPr>
                   <w:p><w:r><w:t>kept</w:t></w:r></w:p></w:tc></w:tr>
               </w:tbl>"#,
        );

        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|warning| warning.kind == WarningKind::UnknownElement));
        let table = tables(&body)[0];
        assert_eq!(cell_text(&table.rows[0].cells[0]), "kept");
    }

    // -----------------------------------------------------------------------
    // Фикстуры
    // -----------------------------------------------------------------------

    /// Каталог фикстур документа.
    fn fixtures_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
    }

    /// Фикстуры категории: имя, `word/document.xml`, связи части и сайкар.
    fn fixture_cases(category: &str) -> Vec<(String, Vec<u8>, Relationships, serde_json::Value)> {
        let dir = fixtures_root().join(category);
        let mut cases = Vec::new();
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{} не читается: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("docx") {
                continue;
            }
            let name = path
                .file_stem()
                .expect("fixture name")
                .to_string_lossy()
                .into_owned();
            let sidecar = std::fs::read_to_string(path.with_extension("json"))
                .unwrap_or_else(|e| panic!("{name}: сайкар не читается: {e}"));
            let sidecar: serde_json::Value =
                serde_json::from_str(&sidecar).unwrap_or_else(|e| panic!("{name}: сайкар: {e}"));
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut archive = doc_converter_core::Archive::new(bytes)
                .unwrap_or_else(|e| panic!("{name}: пакет не открывается: {e}"));
            let xml = archive
                .read("word/document.xml")
                .unwrap_or_else(|e| panic!("{name}: `word/document.xml`: {e}"));
            let mut rels_ctx = ParseCtx::new();
            let rels = crate::rels::load(&mut archive, PART, &mut rels_ctx)
                .unwrap_or_else(|e| panic!("{name}: связи части: {e}"));
            cases.push((name, xml, rels, sidecar));
        }
        cases.sort_by(|a, b| a.0.cmp(&b.0));
        cases
    }

    /// Фикстуры четырёх категорий: абзацы, текст и отсутствие предупреждений
    /// сверяются с сайкаром — он и есть ожидание S12.
    #[test]
    fn fixtures_match_their_sidecars() {
        let mut checked = 0;
        for category in ["basic", "simple", "edge_cases", "formatting"] {
            for (name, xml, _rels, sidecar) in fixture_cases(category) {
                let path = format!("{category}/{name}");
                let (body, warnings) = parse_part(&xml);
                assert!(warnings.is_empty(), "{path}: {warnings:?}");

                let paragraphs = paragraphs(&body);
                let expected_count = sidecar["metadata"]["expectedParagraphs"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("{path}: сайкар без `expectedParagraphs`"));
                assert_eq!(
                    u64::try_from(paragraphs.len()).expect("the count fits u64"),
                    expected_count,
                    "{path}: число абзацев"
                );

                let expected = sidecar["content"]["paragraphs"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{path}: сайкар без `content.paragraphs`"));
                assert_eq!(paragraphs.len(), expected.len(), "{path}: сайкар");
                for (index, (paragraph, expected)) in paragraphs.iter().zip(expected).enumerate() {
                    let want = expected["text"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{path}: абзац {index} без `text`"));
                    assert_eq!(paragraph_text(paragraph), want, "{path}: абзац {index}");
                }

                // Каталог `metadata.sections` описывает секции фикстуры; у кого
                // его нет, тот сверяется только по абзацам.
                if let Some(expected) = sidecar["metadata"]["sections"].as_array() {
                    assert_eq!(body.sections.len(), expected.len(), "{path}: число секций");
                    for (index, (section, expected)) in
                        body.sections.iter().zip(expected).enumerate()
                    {
                        let want = match expected["orientation"].as_str() {
                            Some("portrait") => Orientation::Portrait,
                            Some("landscape") => Orientation::Landscape,
                            other => panic!("{path}: секция {index}: ориентация {other:?}"),
                        };
                        assert_eq!(section.orientation, want, "{path}: секция {index}");
                        let width = expected["widthTwips"]
                            .as_i64()
                            .unwrap_or_else(|| panic!("{path}: секция {index} без `widthTwips`"));
                        assert_eq!(
                            i64::from(section.page_size.width.value()),
                            width,
                            "{path}: секция {index}: ширина"
                        );
                        let height = expected["heightTwips"]
                            .as_i64()
                            .unwrap_or_else(|| panic!("{path}: секция {index} без `heightTwips`"));
                        assert_eq!(
                            i64::from(section.page_size.height.value()),
                            height,
                            "{path}: секция {index}: высота"
                        );
                    }
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 23, "в четырёх категориях 23 фикстуры");
    }

    /// Координаты `{row, col}` из сайкара: `col` — номер ячейки в строке.
    fn sidecar_at(path: &str, entry: &serde_json::Value) -> (usize, usize) {
        let index = |key: &str| {
            usize::try_from(
                entry[key]
                    .as_u64()
                    .unwrap_or_else(|| panic!("{path}: в сайкаре нет `{key}`")),
            )
            .expect("the index fits usize")
        };
        (index("row"), index("col"))
    }

    /// Сверить таблицу с ожиданием сайкара.
    ///
    /// `cells` сайкара — текст ячеек по строкам (ячейка с `gridSpan` занимает
    /// одну позицию), координатные списки (`gridSpan`, `vMerge`, `shading`,
    /// `cellBorders`) — по номеру ячейки, `nested` — вложенная таблица.
    fn check_table(path: &str, table: &Table, expected: &serde_json::Value) {
        check_table_shape(path, table, expected);
        check_table_spans(path, table, expected);
        check_table_widths(path, table, expected);
        check_table_format(path, table, expected);
        check_table_borders(path, table, expected);
        check_table_nested(path, table, expected);
    }

    /// Число строк, столбцов и текст ячеек.
    fn check_table_shape(path: &str, table: &Table, expected: &serde_json::Value) {
        let count = |key: &str| {
            usize::try_from(
                expected[key]
                    .as_u64()
                    .unwrap_or_else(|| panic!("{path}: в сайкаре нет `{key}`")),
            )
            .expect("the count fits usize")
        };
        assert_eq!(table.rows.len(), count("rows"), "{path}: число строк");
        // Ширина в столбцах — сумма `gridSpan` самой широкой строки:
        // `w:tblGrid` есть не во всех фикстурах.
        let cols = table
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.grid_span).sum::<u32>())
            .max()
            .unwrap_or(0);
        assert_eq!(
            usize::try_from(cols).expect("the count fits usize"),
            count("cols"),
            "{path}: число столбцов"
        );

        let cells = expected["cells"]
            .as_array()
            .unwrap_or_else(|| panic!("{path}: сайкар без `cells`"));
        assert_eq!(table.rows.len(), cells.len(), "{path}: строк в `cells`");
        for (index, (row, want)) in table.rows.iter().zip(cells).enumerate() {
            let want = want
                .as_array()
                .unwrap_or_else(|| panic!("{path}: в сайкаре не список ячеек"))
                .iter()
                .map(|cell| {
                    cell.as_str()
                        .unwrap_or_else(|| panic!("{path}: текст ячейки не строка"))
                        .to_owned()
                })
                .collect::<Vec<_>>();
            let got = row.cells.iter().map(cell_text).collect::<Vec<_>>();
            assert_eq!(got, want, "{path}: строка {index}");
        }
    }

    /// Объединения ячеек: `w:gridSpan` и `w:vMerge`.
    fn check_table_spans(path: &str, table: &Table, expected: &serde_json::Value) {
        if let Some(spans) = expected["gridSpan"].as_array() {
            for entry in spans {
                let (row, col) = sidecar_at(path, entry);
                assert_eq!(
                    u64::from(table.rows[row].cells[col].grid_span),
                    entry["span"].as_u64().expect("`span`"),
                    "{path}: `w:gridSpan` в строке {row}"
                );
            }
        }

        if let Some(merges) = expected["vMerge"].as_array() {
            for entry in merges {
                let (row, col) = sidecar_at(path, entry);
                let want = match entry["val"].as_str().expect("`val`") {
                    "restart" => VMerge::Restart,
                    other => {
                        assert_eq!(other, "continue", "{path}: неизвестный `vMerge`");
                        VMerge::Continue
                    }
                };
                assert_eq!(
                    table.rows[row].cells[col].v_merge,
                    Some(want),
                    "{path}: `w:vMerge` в строке {row}"
                );
            }
        }
    }

    /// Ширины: сетка `w:tblGrid` и `w:tcW` ячеек.
    fn check_table_widths(path: &str, table: &Table, expected: &serde_json::Value) {
        let number = |value: &serde_json::Value| {
            i32::try_from(
                value
                    .as_i64()
                    .unwrap_or_else(|| panic!("{path}: ширина не число")),
            )
            .expect("the width fits i32")
        };
        if let Some(grid) = expected["grid"].as_array() {
            let got = table
                .grid
                .iter()
                .map(|col| col.width.value())
                .collect::<Vec<_>>();
            let want = grid.iter().map(number).collect::<Vec<_>>();
            assert_eq!(got, want, "{path}: `w:tblGrid`");
        }

        if let Some(widths) = expected["widths"].as_array() {
            for (index, (row, want)) in table.rows.iter().zip(widths).enumerate() {
                for (cell, width) in row
                    .cells
                    .iter()
                    .zip(want.as_array().expect("a row of widths"))
                {
                    assert_eq!(
                        cell.width,
                        Some(CellWidth::Dxa(Twips::new(number(width)))),
                        "{path}: `w:tcW` в строке {index}"
                    );
                }
            }
        }
    }

    /// Выравнивание, стиль, `w:tblLook` и повтор шапки.
    fn check_table_format(path: &str, table: &Table, expected: &serde_json::Value) {
        if let Some(jc) = expected["jc"].as_str() {
            assert_eq!(
                serde_json::to_value(&table.jc).expect("justification is serializable"),
                serde_json::json!(jc),
                "{path}: `w:jc`"
            );
        }

        if let Some(aligns) = expected["vAlign"].as_array() {
            for (index, (row, want)) in table.rows.iter().zip(aligns).enumerate() {
                for (cell, align) in row
                    .cells
                    .iter()
                    .zip(want.as_array().expect("a row of aligns"))
                {
                    assert_eq!(
                        serde_json::to_value(cell.v_align).expect("align is serializable"),
                        *align,
                        "{path}: `w:vAlign` в строке {index}"
                    );
                }
            }
        }

        if let Some(style) = expected["style"].as_str() {
            assert_eq!(
                table.style_ref.as_ref().map(StyleId::as_str),
                Some(style),
                "{path}: `w:tblStyle`"
            );
        }

        if let Some(look) = expected["tblLook"].as_object() {
            let flag = |key: &str| {
                look.get(key)
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            };
            let got = [
                table.look.first_row,
                table.look.last_row,
                table.look.first_column,
                table.look.last_column,
                table.look.no_h_band,
                table.look.no_v_band,
            ];
            let want = [
                flag("firstRow"),
                flag("lastRow"),
                flag("firstColumn"),
                flag("lastColumn"),
                flag("noHBand"),
                flag("noVBand"),
            ];
            assert_eq!(got, want, "{path}: `w:tblLook`");
        }

        if let Some(header_rows) = expected["headerRows"].as_u64() {
            let got = table.rows.iter().filter(|row| row.header).count();
            assert_eq!(
                u64::try_from(got).expect("the count fits u64"),
                header_rows,
                "{path}: строк с `w:tblHeader`"
            );
        }
        if let Some(repeat) = expected["repeatHeader"].as_bool() {
            assert_eq!(
                table.rows.iter().any(|row| row.header),
                repeat,
                "{path}: `w:tblHeader`"
            );
        }
    }

    /// Границы таблицы, границы ячеек и заливка.
    fn check_table_borders(path: &str, table: &Table, expected: &serde_json::Value) {
        if let Some(borders) = expected["borders"].as_object() {
            let want = serde_json::Value::Object(borders.clone());
            if borders.get("sides").and_then(serde_json::Value::as_str) == Some("all") {
                let sides = [
                    &table.borders.top,
                    &table.borders.left,
                    &table.borders.bottom,
                    &table.borders.right,
                    &table.borders.inside_h,
                    &table.borders.inside_v,
                ];
                assert!(
                    sides.iter().all(|side| side.is_some()),
                    "{path}: границы заданы со всех сторон"
                );
            }
            check_border(
                path,
                table.borders.top.as_ref().expect("`w:top` is kept"),
                &want,
            );
        }

        if let Some(entries) = expected["cellBorders"].as_array() {
            for entry in entries {
                let (row, col) = sidecar_at(path, entry);
                let cell = &table.rows[row].cells[col];
                for (side, want) in entry.as_object().expect("an object") {
                    let border = match side.as_str() {
                        "row" | "col" => continue,
                        "top" => cell.borders.top.as_ref(),
                        "left" => cell.borders.left.as_ref(),
                        "bottom" => cell.borders.bottom.as_ref(),
                        "right" => cell.borders.right.as_ref(),
                        other => panic!("{path}: граница `{other}` в сайкаре не поддержана"),
                    }
                    .unwrap_or_else(|| panic!("{path}: в ячейке нет границы `{side}`"));
                    check_border(path, border, want);
                }
            }
        }

        if let Some(entries) = expected["shading"].as_array() {
            for entry in entries {
                let (row, col) = sidecar_at(path, entry);
                let cell = &table.rows[row].cells[col];
                assert_eq!(
                    cell.shading.as_ref().and_then(|shd| shd.fill),
                    Some(sidecar_color(path, &entry["fill"])),
                    "{path}: `w:shd/@fill` в строке {row}"
                );
            }
        }
    }

    /// Вложенная таблица в ячейке, на которую указывает `at`.
    fn check_table_nested(path: &str, table: &Table, expected: &serde_json::Value) {
        if expected["nested"].as_object().is_none() {
            return;
        }
        let (row, col) = sidecar_at(path, &expected["nested"]["at"]);
        let nested = table.rows[row].cells[col]
            .items
            .iter()
            .find_map(|item| match item {
                BlockItem::Table(table) => Some(table),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{path}: в ячейке {row}/{col} нет вложенной таблицы"));
        check_table(&format!("{path}: вложенная"), nested, &expected["nested"]);
    }

    /// Сверить границу с полями `{val, sz, color}` из сайкара.
    fn check_border(path: &str, border: &Border, expected: &serde_json::Value) {
        assert_eq!(
            serde_json::to_value(&border.val).expect("the style is serializable"),
            expected["val"],
            "{path}: стиль границы"
        );
        if let Some(sz) = expected["sz"].as_u64() {
            assert_eq!(
                border.sz,
                Some(u32::try_from(sz).expect("the size fits u32")),
                "{path}: толщина границы"
            );
        }
        if let Some(color) = expected["color"].as_str() {
            assert_eq!(
                border.color,
                Some(sidecar_color(path, &serde_json::Value::from(color))),
                "{path}: цвет границы"
            );
        }
    }

    /// Цвет `RRGGBB` из сайкара.
    fn sidecar_color(path: &str, value: &serde_json::Value) -> Color {
        let raw = value
            .as_str()
            .unwrap_or_else(|| panic!("{path}: цвет не строка"));
        Color::Rgb(
            u32::from_str_radix(raw, 16).unwrap_or_else(|_| panic!("{path}: `{raw}` не цвет")),
        )
    }

    /// Девять фикстур таблиц — против сайкаров: строки, ячейки, `gridSpan`,
    /// `vMerge`, ширины, выравнивание, оформление и вложенные таблицы.
    #[test]
    fn tables_fixtures_match_their_sidecars() {
        let mut checked = 0;
        for (name, xml, _rels, sidecar) in fixture_cases("tables") {
            let path = format!("tables/{name}");
            let (body, warnings) = parse_part(&xml);
            assert!(warnings.is_empty(), "{path}: {warnings:?}");

            let expected = sidecar["content"]["tables"]
                .as_array()
                .unwrap_or_else(|| panic!("{path}: сайкар без `content.tables`"));
            let tables = tables(&body);
            assert_eq!(tables.len(), expected.len(), "{path}: число таблиц");
            assert_eq!(
                u64::try_from(tables.len()).expect("the count fits u64"),
                sidecar["metadata"]["expectedTables"].as_u64().unwrap_or(0),
                "{path}: `expectedTables`"
            );
            for (index, (table, want)) in tables.iter().zip(expected).enumerate() {
                check_table(&format!("{path}: таблица {index}"), table, want);
            }
            checked += 1;
        }
        assert_eq!(checked, 9, "в категории `tables` девять фикстур");
    }

    // -----------------------------------------------------------------------
    // Снапшоты
    // -----------------------------------------------------------------------

    /// Пути частей, названные сайдкаром где угодно в `content`.
    fn sidecar_parts(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    match (key.as_str(), item.as_str()) {
                        ("part", Some(part)) => out.push(part.to_owned()),
                        _ => sidecar_parts(item, out),
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    sidecar_parts(item, out);
                }
            }
            _ => {}
        }
    }

    /// Колонтитулы секции — те же части, что называет сайдкар.
    ///
    /// Сайдкар может перечислять их и в `content.headers`, и в `content.fields`
    /// (поле живёт в колонтитуле) — поэтому части собираются рекурсивно.
    #[test]
    fn section_header_and_footer_references_match_their_sidecars() {
        let mut checked = 0;
        for (name, xml, rels, sidecar) in fixture_cases("headers_footers") {
            let path = format!("headers_footers/{name}");
            let (body, warnings) = parse_part_with(&xml, &rels);
            assert!(warnings.is_empty(), "{path}: {warnings:?}");
            let section = body
                .sections
                .first()
                .unwrap_or_else(|| panic!("{path}: у тела нет секции"));

            let mut found: Vec<String> = [
                &section.header_default,
                &section.header_first,
                &section.header_even,
                &section.footer_default,
                &section.footer_first,
                &section.footer_even,
            ]
            .into_iter()
            .flatten()
            .map(|reference| reference.target.clone())
            .collect();
            found.sort();

            let mut expected = Vec::new();
            sidecar_parts(&sidecar["content"], &mut expected);
            expected
                .retain(|part| part.starts_with("word/header") || part.starts_with("word/footer"));
            expected.sort();
            expected.dedup();

            assert_eq!(found, expected, "{path}: колонтитулы секции");
            if let Some(title_pg) = sidecar["content"]["titlePg"].as_bool() {
                assert_eq!(section.title_pg, title_pg, "{path}: `w:titlePg`");
            }
            checked += 1;
        }
        assert_eq!(checked, 5, "в категории `headers_footers` пять фикстур");
    }

    /// Снапшот модели: так видно и структуру, и каждое разобранное свойство.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn snapshot_breaks_and_tabs_fixture() {
        let (_, xml, _, _) = fixture_cases("basic")
            .into_iter()
            .find(|(name, ..)| name == "breaks_and_tabs")
            .expect("фикстура на месте");
        let (body, warnings) = parse_part(&xml);

        assert!(warnings.is_empty(), "{warnings:?}");
        insta::assert_json_snapshot!("breaks_and_tabs", body);
    }

    /// Снапшот свойств абзаца и знака: `w:pStyle`, `w:rStyle`, тоглы, `w:shd`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn snapshot_heading_fixture() {
        let (_, xml, _, _) = fixture_cases("formatting")
            .into_iter()
            .find(|(name, ..)| name == "heading_1")
            .expect("фикстура на месте");
        let (body, warnings) = parse_part(&xml);

        assert!(warnings.is_empty(), "{warnings:?}");
        insta::assert_json_snapshot!("heading_1", body);
    }

    /// Снапшот веток, которых нет в фикстурах: `w:sym`, дефисы, `Unknown`,
    /// `mc:AlternateContent`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn snapshot_rich_paragraph() {
        let (body, warnings) = parse_xml(
            r#"<w:p>
                 <w:pPr><w:jc w:val="both"/><w:contextualSpacing/></w:pPr>
                 <w:r>
                   <w:rPr><w:u/></w:rPr>
                   <w:t>one</w:t><w:br w:type="page"/><w:noBreakHyphen/><w:softHyphen/>
                   <w:sym w:font="Wingdings" w:char="F0E0"/><w:tab/>
                   <mc:AlternateContent>
                     <mc:Choice Requires="wps"><w:t>choice</w:t></mc:Choice>
                     <mc:Fallback><w:t>fallback</w:t></mc:Fallback>
                   </mc:AlternateContent>
                   <w:drawing/>
                 </w:r>
               </w:p>"#,
        );

        assert_eq!(warnings.len(), 2, "{warnings:?}");
        insta::assert_json_snapshot!("rich_paragraph", body);
    }

    /// Снапшот таблицы: видно и структуру ячеек, и каждое разобранное свойство.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn snapshot_table_fixture() {
        let (_, xml, _, _) = fixture_cases("tables")
            .into_iter()
            .find(|(name, ..)| name == "v_merge")
            .expect("фикстура на месте");
        let (body, warnings) = parse_part(&xml);

        assert!(warnings.is_empty(), "{warnings:?}");
        insta::assert_json_snapshot!("table_v_merge", body);
    }
}
