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
use doc_converter_core::WarningKind;
use quick_xml::events::{BytesStart, Event};

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::{
    BlockItem, Body, Border, BorderStyle, BreakKind, CharacterSpacing, Color, FontHint, HalfPoint,
    Highlight, Ind, Inline, Justification, LineSpacing, LineSpacingRule, NumId, NumPr, Paragraph,
    ParagraphBorders, ParagraphSpacing, RFonts, RawPPr, RawRPr, Relationships, Run, RunContent,
    Shading, ShadingPattern, StyleId, TabLeader, TabStop, TabStopKind, Twips, Underline, VertAlign,
};
use crate::xml::{
    attr_i32, attr_toggle, attr_u32, attributes, capture_element, find, is_true, local_name,
    resolve_alternate_content, resolve_reference, wrap_fragment, AlternateContent, Attr,
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
        match local_name(element.name().into_inner()) {
            b"p" => {
                let paragraph = if empty {
                    Paragraph {
                        id: ctx.id(),
                        ..Paragraph::default()
                    }
                } else {
                    parse_paragraph(reader, ctx, part, xml_path)?
                };
                items.push(BlockItem::Paragraph(paragraph));
            }
            b"sectPr" => {
                // TODO (S7b): завершающий `w:sectPr` тела → `BlockItem::SectPr`
                // и `Body::sections`. Пока сохраняется как есть и без
                // предупреждения: это не мусор, а отложенный разбор.
                let id = ctx.id();
                let xml = capture_any(reader, element, empty, ctx, part)?;
                items.push(BlockItem::Unknown { id, xml });
            }
            b"AlternateContent" if !empty => {
                // Ветка `mc:Choice` разворачивается на месте блока, как будто
                // её содержимое и было телом (ADR-0014 §1).
                match resolve_alternate_content(reader, element, ctx, part, xml_path)? {
                    AlternateContent::Choice(fragment) => {
                        items.extend(parse_block_fragment(&fragment, rels, ctx, part, xml_path)?);
                    }
                    AlternateContent::Unsupported(xml) => {
                        ctx.warn_at(
                            WarningKind::UnknownElement,
                            part,
                            Some(xml_path),
                            "`mc:AlternateContent` has no supported `mc:Choice`, kept as unknown",
                        )?;
                        items.push(BlockItem::Unknown { id: ctx.id(), xml });
                    }
                }
            }
            _ => {
                // TODO (S7b): `w:tbl` (таблицы) и прочие блочные элементы.
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
                items.push(BlockItem::Unknown { id, xml });
            }
        }
    }
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
                // TODO (S7b): `sections` соберёт разбор `w:sectPr` — концов
                // секций из `w:pPr` абзацев и завершающего `w:sectPr` тела.
                return Ok(Body {
                    id,
                    items,
                    sections: Vec::new(),
                });
            }
            _ => {}
        }
    }
    Err(Error::malformed(part, "`w:document/w:body` is missing"))
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
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Paragraph> {
    let id = ctx.id();
    let mut ppr = RawPPr::default();
    let mut runs = Vec::new();
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
                ppr = if empty {
                    RawPPr::default()
                } else {
                    parse_ppr(reader, ctx, part, xml_path)?
                };
            }
            _ => parse_inline_element(reader, element, empty, ctx, part, xml_path, &mut runs)?,
        }
    }
    // `mark_rpr` — свойства знака абзаца: к runs они не применяются (ADR-0013 §2).
    let mark_rpr = ppr.r_pr.clone().unwrap_or_default();
    let style_ref = ppr.style.clone();
    let numbering_ref = ppr.num_pr.as_ref().and_then(|num| num.num_id);
    Ok(Paragraph {
        id,
        ppr,
        mark_rpr,
        runs,
        style_ref,
        numbering_ref,
        // TODO (S7b): `w:sectPr` внутри `w:pPr` — конец секции, а не её начало.
        section_break: None,
    })
}

// ---------------------------------------------------------------------------
// Inline-содержимое абзаца
// ---------------------------------------------------------------------------

/// Разобрать один элемент inline-уровня и дописать его в `out`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_inline_element(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
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
                parse_run(reader, ctx, part, xml_path)?
            };
            out.push(Inline::Run(run));
        }
        b"AlternateContent" if !empty => {
            match resolve_alternate_content(reader, element, ctx, part, xml_path)? {
                AlternateContent::Choice(fragment) => {
                    out.extend(parse_inline_fragment(&fragment, ctx, part, xml_path)?);
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
        // Разметка, не несущая содержимого модели. TODO (S7b): `w:bookmarkEnd`
        // даёт `OrphanBookmark`, только если пары нет, — а пару видно лишь при
        // разборе `w:bookmarkStart`, который S7a оставляет неизвестным.
        b"proofErr" | b"lastRenderedPageBreak" | b"bookmarkEnd" => {
            skip_element(reader, empty, part)?;
        }
        _ => {
            // TODO (S7b): ссылки, закладки, поля, вставки и удаления.
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

/// Разобрать фрагмент `mc:Choice` как inline-содержимое.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_inline_fragment(
    fragment: &str,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Vec<Inline>> {
    let wrapped = wrap_fragment(fragment);
    let mut reader = XmlReader::preserving(wrapped.as_bytes(), part);
    expect_fragment_root(&mut reader, part)?;
    let mut out = Vec::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                "unexpected end of input inside an inline fragment",
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(out),
            _ => continue,
        };
        parse_inline_element(&mut reader, element, empty, ctx, part, xml_path, &mut out)?;
    }
}

// ---------------------------------------------------------------------------
// Run и его содержимое
// ---------------------------------------------------------------------------

/// Разобрать `w:r`: свойства знака и содержимое.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_run(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<Run> {
    let id = ctx.id();
    let mut rpr = RawRPr::default();
    let mut content = Vec::new();
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
            parse_run_content(reader, element, empty, ctx, part, xml_path, &mut content)?;
        }
    }
    let style_ref = rpr.style.clone();
    Ok(Run {
        id,
        rpr,
        style_ref,
        content,
    })
}

/// Разобрать один элемент содержимого run'а и дописать его в `out`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_run_content(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
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
            out.extend(parse_run_fragment(&fragment, ctx, part, xml_path)?);
        }
        b"lastRenderedPageBreak" => skip_element(reader, empty, part)?,
        _ => {
            // TODO (S7b): `w:drawing` (рисунки) и прочее содержимое run'а.
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
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
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
        parse_run_content(&mut reader, element, empty, ctx, part, xml_path, &mut out)?;
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
/// # Errors
/// То же, что у [`parse_blocks`].
fn parse_ppr(
    reader: &mut XmlReader<'_>,
    ctx: &mut ParseCtx,
    part: &str,
    xml_path: &str,
) -> Result<RawPPr> {
    let mut ppr = RawPPr::default();
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
            Event::End(_) => return Ok(ppr),
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
                // TODO (S7b): `w:sectPr` в `w:pPr` — конец секции (`sect_pr`).
                // Сохраняется без предупреждения: элемент известен, отложен.
                let xml = capture_any(reader, element, empty, ctx, part)?;
                ppr.unknown.push(("sectPr".to_owned(), xml));
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

    /// Разобрать часть `word/document.xml`.
    fn parse_part(xml: &[u8]) -> (Body, Vec<ParseWarning>) {
        let mut ctx = ParseCtx::new();
        let body =
            parse(xml, &Relationships::default(), &mut ctx, PART).expect("the document parses");
        let warnings = ctx.warnings().to_vec();
        (body, warnings)
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
        assert_eq!(paragraph.ppr.r_pr, Some(paragraph.mark_rpr.clone()));
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
            r#"<w:tbl><w:tr><w:tc/></w:tr></w:tbl>
               <w:p><w:r><w:drawing/><w:footnoteReference w:id="1"/></w:r></w:p>"#,
        );

        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|warning| warning.kind == WarningKind::UnknownElement));
        match &body.items[0] {
            BlockItem::Unknown { xml, .. } => {
                assert_eq!(xml, "<w:tbl><w:tr><w:tc/></w:tr></w:tbl>");
            }
            other => panic!("expected an unknown block, got {other:?}"),
        }
        let content = &only_run(&body).content;
        assert_eq!(content.len(), 2);
        match &content[0] {
            RunContent::Unknown { xml, .. } => assert_eq!(xml, "<w:drawing/>"),
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
    fn body_section_properties_are_deferred_without_a_warning() {
        let (body, warnings) = parse_xml("<w:p/><w:sectPr><w:pgSz w:w=\"11906\"/></w:sectPr>");

        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(body.sections.is_empty());
        match &body.items[1] {
            BlockItem::Unknown { xml, .. } => {
                assert_eq!(xml, r#"<w:sectPr><w:pgSz w:w="11906"/></w:sectPr>"#);
            }
            other => panic!("expected an unknown block, got {other:?}"),
        }
    }

    #[test]
    fn ignored_inline_markup_does_not_produce_warnings() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:proofErr w:type="spellStart"/><w:lastRenderedPageBreak/>
               <w:bookmarkEnd w:id="7"/><w:r><w:t>text</w:t></w:r></w:p>"#,
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(paragraph_text(paragraphs(&body)[0]), "text");
    }

    #[test]
    fn hyperlinks_are_unknown_but_their_text_is_not_lost() {
        let (body, warnings) = parse_xml(
            r#"<w:p><w:hyperlink r:id="rId4"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>"#,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnknownElement);
        // Содержимое ссылки сохраняется в XML узла `Unknown` — S7b разберёт его
        // на месте, а не потеряет текст.
        match &paragraphs(&body)[0].runs[0] {
            Inline::Unknown { xml, .. } => {
                assert_eq!(
                    xml,
                    r#"<w:hyperlink r:id="rId4"><w:r><w:t>link</w:t></w:r></w:hyperlink>"#
                );
            }
            other => panic!("expected an unknown inline, got {other:?}"),
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
    // Фикстуры
    // -----------------------------------------------------------------------

    /// Каталог фикстур документа.
    fn fixtures_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
    }

    /// Пары «имя, `word/document.xml`, сайкар» из категории фикстур.
    fn fixture_cases(category: &str) -> Vec<(String, Vec<u8>, serde_json::Value)> {
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
            cases.push((name, xml, sidecar));
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
            for (name, xml, sidecar) in fixture_cases(category) {
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
                checked += 1;
            }
        }
        assert_eq!(checked, 23, "в четырёх категориях 23 фикстуры");
    }

    // -----------------------------------------------------------------------
    // Снапшоты
    // -----------------------------------------------------------------------

    /// Снапшот модели: так видно и структуру, и каждое разобранное свойство.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn snapshot_breaks_and_tabs_fixture() {
        let (_, xml, _) = fixture_cases("basic")
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
        let (_, xml, _) = fixture_cases("formatting")
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
}
