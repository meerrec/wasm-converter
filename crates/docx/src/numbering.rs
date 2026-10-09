//! Разбор `word/numbering.xml` (слайс S9): абстрактные схемы уровней и ссылающиеся на них списки.
//!
//! Часть целиком про нумерацию: `w:abstractNum` задаёт уровни, `w:num` ссылается
//! на схему и может переопределить уровень через `w:lvlOverride`. От картинок-маркеров
//! сохраняются только ссылки (`w:lvlPicBulletId` уровня): само определение маркера —
//! это VML (`w:numPicBullet/w:pict`), а VML — non-goal (ADR-0014 §2).
//!
//! Всё восстановимое — предупреждения (ADR-0016): битый `w:ilvl`, дубликаты id,
//! незнакомый `w:suff` и лишние элементы. Фатально только то, что мешает читать
//! сам XML: [`Error::malformed`] и ошибки ридера.

// Вызывающего у `parse` ещё нет: его подключит сборка документа (слайс S12).
// До тех пор `dead_code` срабатывал бы на каждом элементе модуля; `allow`
// снимается вместе с подключением — как в `context.rs` и `xml.rs`.
#![allow(dead_code)]

use std::collections::BTreeMap;

use doc_converter_core::xml::XmlReader;
use doc_converter_core::WarningKind;
use quick_xml::events::{BytesStart, Event};

use crate::context::ParseCtx;
use crate::error::{Error, Result};
use crate::model::numbering::{
    AbstractNum, AbstractNumId, LevelSuffix, Lvl, LvlOverride, MultiLevelType, Num, NumFmt, NumId,
    NumberingTable,
};
use crate::model::raw::{
    Border, BorderStyle, CharacterSpacing, Color, FontHint, HalfPoint, Highlight, Ind,
    Justification, LineSpacing, LineSpacingRule, NumPr, ParagraphBorders, ParagraphSpacing, RFonts,
    RawPPr, RawRPr, Shading, ShadingPattern, StyleId, TabLeader, TabStop, TabStopKind, Twips,
    Underline, VertAlign,
};
use crate::xml::{
    attr_i32, attr_toggle, attr_u32, attributes, capture_element, find, is_true, local_name,
    resolve_reference, Attr,
};

/// Последний уровень нумерации: `w:ilvl` в `ECMA-376` — 0…8.
const MAX_ILVL: u8 = 8;

/// Значение `w:start`, когда элемент отсутствует.
///
/// `ECMA-376` §17.9.26 называет умолчание 0, но в модели [`Lvl::start`] описан
/// как «по умолчанию 1», а Word всегда пишет `w:start` — расходиться с моделью
/// на пустом месте незачем.
const DEFAULT_START: u32 = 1;

/// Разобрать `word/numbering.xml`.
///
/// Пустая таблица осмысленна: документ без нумерации — обычное дело. В конце
/// таблица проверяется [`NumberingTable::validate`] — `w:num` со ссылкой на
/// отсутствующий `w:abstractNum` даёт [`WarningKind::MissingAbstractNum`].
///
/// # Errors
/// [`Error::malformed`] — поток кончился посреди элемента, байты не UTF-8 или
/// атрибут не читается; [`Error::TooManyWarnings`] — предупреждений стало
/// больше порога [`doc_converter_core::Warnings::FATAL_THRESHOLD`].
pub(crate) fn parse(bytes: &[u8], ctx: &mut ParseCtx, part: &str) -> Result<NumberingTable> {
    let mut table = NumberingTable::default();
    let mut reader = XmlReader::new(bytes, part);
    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(element) => {
                table_element(&mut reader, &element, false, ctx, part, &mut table)?;
            }
            Event::Empty(element) => {
                table_element(&mut reader, &element, true, ctx, part, &mut table)?;
            }
            _ => {}
        }
    }

    table.validate(part, ctx)?;
    Ok(table)
}

/// Обработать элемент верхнего уровня `w:numbering`.
fn table_element(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    table: &mut NumberingTable,
) -> Result<()> {
    match local_name(element.name().as_ref()) {
        // Корень ничего не несёт: его содержимое читает сам цикл `parse`.
        b"numbering" => Ok(()),
        b"abstractNum" => parse_abstract_num(reader, element, empty, ctx, part, table),
        b"num" => parse_num(reader, element, empty, ctx, part, table),
        // Картинка-маркер — это VML (`w:pict`), а VML не поддержан (ADR-0014 §2):
        // ссылки на маркер (`w:lvlPicBulletId` уровня) сохраняются, определение — нет.
        b"numPicBullet" => skip_tail(reader, empty, part),
        _ => {
            ctx.warn(
                WarningKind::UnknownElement,
                part,
                format!(
                    "unknown element `{}` in `w:numbering`, skipped",
                    element_name(element)
                ),
            )?;
            skip_tail(reader, empty, part)
        }
    }
}

// ---------------------------------------------------------------------------
// Схемы и списки
// ---------------------------------------------------------------------------

/// Разобрать `w:abstractNum`.
///
/// # Errors
/// Как у [`parse`].
fn parse_abstract_num(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    table: &mut NumberingTable,
) -> Result<()> {
    let attrs = attributes(element, part)?;
    let Some(raw_id) = attr_u32(&attrs, "abstractNumId", ctx, part)? else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:abstractNum` without a valid `w:abstractNumId`, the scheme is skipped",
        )?;
        return skip_tail(reader, empty, part);
    };
    let id = AbstractNumId::new(raw_id);
    if table.abstract_nums.contains_key(&id) {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("duplicate `w:abstractNumId` {raw_id}: the first scheme wins"),
        )?;
        return skip_tail(reader, empty, part);
    }

    let mut scheme = AbstractNum {
        id,
        // Умолчания в схеме нет («любой вид на усмотрение потребителя», §17.9.13);
        // берём наименее ограничивающий — многоуровневый.
        multi_level_type: MultiLevelType::Multilevel,
        levels: BTreeMap::new(),
        num_style_link: None,
        style_link: None,
        nsid: None,
        tmpl: None,
    };
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        match local_name(child.name().as_ref()) {
            b"nsid" => scheme.nsid = find(&attrs, "val").map(str::to_owned),
            b"tmpl" => scheme.tmpl = find(&attrs, "val").map(str::to_owned),
            b"styleLink" => scheme.style_link = find(&attrs, "val").map(StyleId::new),
            b"numStyleLink" => scheme.num_style_link = find(&attrs, "val").map(StyleId::new),
            b"multiLevelType" => {
                if let Some(raw) = find(&attrs, "val") {
                    match multi_level_type(raw) {
                        Some(kind) => scheme.multi_level_type = kind,
                        None => ctx.warn(
                            WarningKind::InvalidAttribute,
                            part,
                            format!(
                                "`w:multiLevelType`: unrecognized `w:val` `{raw}`, \
                                 the scheme counts as `multilevel`"
                            ),
                        )?,
                    }
                }
            }
            b"lvl" => {
                let lvl = parse_lvl(reader, child, empty, ctx, part, None)?;
                insert_level(&mut scheme.levels, lvl, ctx, part)?;
            }
            // `w:name` — название схемы для UI; в модели поля нет.
            // TODO (Спринт 9): сохранять `w:name`, если он понадобится интерфейсу.
            b"name" => skip_tail(reader, empty, part)?,
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unknown element `{}` in `w:abstractNum`, skipped",
                        element_name(child)
                    ),
                )?;
                skip_tail(reader, empty, part)?;
            }
        }
        Ok(())
    })?;

    table.abstract_nums.insert(id, scheme);
    Ok(())
}

/// Положить уровень в карту по `w:ilvl`.
///
/// Повтор уровня не перезаписывает первый: ADR-0016 — «первый побеждает».
fn insert_level(
    levels: &mut BTreeMap<u8, Lvl>,
    lvl: Option<Lvl>,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<()> {
    let Some(lvl) = lvl else {
        // Уровень уже отбракован — с предупреждением, на своём месте.
        return Ok(());
    };
    if levels.contains_key(&lvl.ilvl) {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("duplicate `w:ilvl` {}: the first level wins", lvl.ilvl),
        )?;
        return Ok(());
    }
    levels.insert(lvl.ilvl, lvl);
    Ok(())
}

/// Разобрать `w:lvl` — уровня схемы или вложенного в `w:lvlOverride`.
///
/// `fallback_ilvl` — номер уровня переопределения: у вложенного `w:lvl` атрибут
/// `w:ilvl` схема требует, но если его нет, а переопределение назвало уровень,
/// терять уровень незачем.
///
/// `None` в ответе означает, что уровень отбракован и уже пропущен: у него нет
/// пригодного `w:ilvl`.
///
/// # Errors
/// Как у [`parse`].
fn parse_lvl(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    fallback_ilvl: Option<u8>,
) -> Result<Option<Lvl>> {
    let attrs = attributes(element, part)?;
    let level = match (attr_u32(&attrs, "ilvl", ctx, part)?, fallback_ilvl) {
        (Some(raw), _) => {
            match u8::try_from(raw) {
                Ok(value) if value <= MAX_ILVL => value,
                _ => {
                    ctx.warn(
                    WarningKind::InvalidAttribute,
                    part,
                    format!("`w:lvl`: `w:ilvl` {raw} is outside 0..={MAX_ILVL}, the level is skipped"),
                )?;
                    skip_tail(reader, empty, part)?;
                    return Ok(None);
                }
            }
        }
        (None, Some(fallback)) => fallback,
        (None, None) => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                "`w:lvl` without a valid `w:ilvl` is skipped",
            )?;
            skip_tail(reader, empty, part)?;
            return Ok(None);
        }
    };

    let mut lvl = Lvl {
        ilvl: level,
        start: DEFAULT_START,
        // Оба умолчания — из схемы: `w:numFmt` без элемента считается `decimal`,
        // `w:suff` — `tab` (§17.9.17, §17.9.24).
        num_fmt: NumFmt::Decimal,
        lvl_text: String::new(),
        lvl_jc: None,
        suff: LevelSuffix::Tab,
        ppr: RawPPr::default(),
        rpr: RawRPr::default(),
        restart: None,
        pstyle: None,
        is_lgl: false,
        pic_bullet_id: None,
        tentative: false,
    };
    read_children(reader, empty, part, |reader, child, empty| {
        if !parse_lvl_child(reader, child, empty, ctx, part, &mut lvl)? {
            ctx.warn(
                WarningKind::UnknownElement,
                part,
                format!(
                    "unknown element `{}` in `w:lvl`, skipped",
                    element_name(child)
                ),
            )?;
            skip_tail(reader, empty, part)?;
        }
        Ok(())
    })?;

    Ok(Some(lvl))
}

/// Разобрать один дочерний элемент `w:lvl`; `false` — элемент не распознан.
///
/// Вызывающий сам решает, что делать с незнакомым элементом: здесь про него
/// известно только имя.
///
/// # Errors
/// Как у [`parse`].
fn parse_lvl_child(
    reader: &mut XmlReader<'_>,
    child: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    lvl: &mut Lvl,
) -> Result<bool> {
    let attrs = attributes(child, part)?;
    match local_name(child.name().as_ref()) {
        b"start" => lvl.start = attr_u32(&attrs, "val", ctx, part)?.unwrap_or(DEFAULT_START),
        b"numFmt" => {
            if let Some(raw) = find(&attrs, "val") {
                // Незнакомый формат не теряется: [`NumFmt::Other`] хранит строку
                // как есть, поэтому предупреждать не о чем.
                lvl.num_fmt = NumFmt::from_ooxml(raw);
            }
        }
        b"lvlText" => {
            lvl.lvl_text = if let Some(value) = find(&attrs, "val") {
                skip_tail(reader, empty, part)?;
                value.to_owned()
            } else if empty {
                String::new()
            } else {
                // Схема требует `w:val`; если его нет, а текст есть — берём
                // текст: так делает часть чужих writer'ов.
                read_text(reader, part)?
            };
        }
        b"lvlJc" => lvl.lvl_jc = find(&attrs, "val").map(justification),
        b"suff" => {
            if let Some(raw) = find(&attrs, "val") {
                match level_suffix(raw) {
                    Some(suff) => lvl.suff = suff,
                    None => ctx.warn(
                        WarningKind::InvalidAttribute,
                        part,
                        format!("`w:suff`: unrecognized `w:val` `{raw}`, `tab` is assumed"),
                    )?,
                }
            }
        }
        b"lvlRestart" | b"restart" => {
            if let Some(raw) = attr_i32(&attrs, "val", ctx, part)? {
                match u8::try_from(raw) {
                    Ok(value) if value <= MAX_ILVL => lvl.restart = Some(value),
                    _ => ctx.warn(
                        WarningKind::InvalidAttribute,
                        part,
                        format!("`w:lvlRestart`: `w:val` {raw} is outside 0..={MAX_ILVL}, ignored"),
                    )?,
                }
            }
        }
        b"pStyle" => lvl.pstyle = find(&attrs, "val").map(StyleId::new),
        b"isLgl" => lvl.is_lgl = find(&attrs, "val").is_none_or(is_true),
        b"tentative" => lvl.tentative = find(&attrs, "val").is_none_or(is_true),
        // `w:lvlPicBulletId` — так элемент назван в схеме; короткое имя
        // встречается у чужих writer'ов.
        b"lvlPicBulletId" | b"picBulletId" => {
            lvl.pic_bullet_id = attr_u32(&attrs, "val", ctx, part)?;
        }
        b"pPr" => lvl.ppr = parse_ppr(reader, empty, ctx, part)?,
        b"rPr" => lvl.rpr = parse_rpr(reader, empty, ctx, part)?,
        // `w:legacy` — указание старому Word игнорировать уровень; в модели
        // поля нет.
        // TODO (Спринт 9): сохранять `w:legacy`, если раскладке он понадобится.
        b"legacy" => skip_tail(reader, empty, part)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Разобрать `w:num`.
///
/// # Errors
/// Как у [`parse`].
fn parse_num(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    table: &mut NumberingTable,
) -> Result<()> {
    let attrs = attributes(element, part)?;
    let Some(raw_id) = attr_u32(&attrs, "numId", ctx, part)? else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:num` without a valid `w:numId`, the list is skipped",
        )?;
        return skip_tail(reader, empty, part);
    };
    let id = NumId::new(raw_id);
    if table.nums.contains_key(&id) {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("duplicate `w:numId` {raw_id}: the first list wins"),
        )?;
        return skip_tail(reader, empty, part);
    }

    let mut num = Num {
        id,
        abstract_id: AbstractNumId::new(0),
        overrides: BTreeMap::new(),
        picture_bullet_id: None,
    };
    let mut abstract_linked = false;
    let mut abstract_seen = false;
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        match local_name(child.name().as_ref()) {
            b"abstractNumId" => {
                abstract_seen = true;
                let raw = attr_u32(&attrs, "val", ctx, part)?;
                match (raw, abstract_linked) {
                    (Some(raw), false) => {
                        num.abstract_id = AbstractNumId::new(raw);
                        abstract_linked = true;
                    }
                    (Some(raw), true) => ctx.warn(
                        WarningKind::InvalidAttribute,
                        part,
                        format!("duplicate `w:abstractNumId` {raw} in `w:num`: the first wins"),
                    )?,
                    // О мусорном `w:val` уже предупредил `attr_u32`.
                    (None, _) => {}
                }
            }
            b"lvlOverride" => parse_override(reader, child, empty, ctx, part, &mut num)?,
            // В `CT_Num` такого элемента нет, но поле есть в модели — принимаем
            // оба написания, чтобы не терять ссылку у чужих writer'ов.
            b"lvlPicBulletId" | b"picBulletId" => {
                num.picture_bullet_id = attr_u32(&attrs, "val", ctx, part)?;
            }
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unknown element `{}` in `w:num`, skipped",
                        element_name(child)
                    ),
                )?;
                skip_tail(reader, empty, part)?;
            }
        }
        Ok(())
    })?;

    // Список без схемы бессмыслен: абзацы со ссылкой на него получат
    // `MissingNumId`, а сам он не смог бы отдать ни одного уровня.
    if !abstract_linked {
        if !abstract_seen {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                "`w:num` without a valid `w:abstractNumId`, the list is skipped",
            )?;
        }
        return Ok(());
    }

    table.nums.insert(id, num);
    Ok(())
}

/// Разобрать `w:lvlOverride`.
///
/// # Errors
/// Как у [`parse`].
fn parse_override(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    num: &mut Num,
) -> Result<()> {
    let attrs = attributes(element, part)?;
    let Some(raw) = attr_u32(&attrs, "ilvl", ctx, part)? else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "`w:lvlOverride` without a valid `w:ilvl`, the override is skipped",
        )?;
        return skip_tail(reader, empty, part);
    };
    let Ok(ilvl) = u8::try_from(raw) else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("`w:lvlOverride`: `w:ilvl` {raw} is outside 0..={MAX_ILVL}, skipped"),
        )?;
        return skip_tail(reader, empty, part);
    };
    if ilvl > MAX_ILVL {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("`w:lvlOverride`: `w:ilvl` {raw} is outside 0..={MAX_ILVL}, skipped"),
        )?;
        return skip_tail(reader, empty, part);
    }

    let mut over = LvlOverride {
        start_override: None,
        lvl: None,
    };
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        match local_name(child.name().as_ref()) {
            b"startOverride" => {
                over.start_override = attr_u32(&attrs, "val", ctx, part)?;
            }
            b"lvl" => over.lvl = parse_lvl(reader, child, empty, ctx, part, Some(ilvl))?,
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unknown element `{}` in `w:lvlOverride`, skipped",
                        element_name(child)
                    ),
                )?;
                skip_tail(reader, empty, part)?;
            }
        }
        Ok(())
    })?;

    if num.overrides.contains_key(&ilvl) {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("duplicate `w:lvlOverride` for `w:ilvl` {ilvl}: the first one wins"),
        )?;
        return Ok(());
    }
    num.overrides.insert(ilvl, over);
    Ok(())
}

// ---------------------------------------------------------------------------
// Свойства абзаца и знака внутри уровня
// ---------------------------------------------------------------------------

/// Разобрать `w:pPr` уровня.
///
/// Разбор локальный: общий с `document.rs`/`styles.rs` появится, когда слайсы
/// сойдутся, — а пока `numbering.rs` не тянет чужие файлы.
///
/// # Errors
/// Как у [`parse`].
fn parse_ppr(
    reader: &mut XmlReader<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<RawPPr> {
    let mut ppr = RawPPr::default();
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        match local_name(child.name().as_ref()) {
            b"pStyle" => {
                ppr.style = find(&attrs, "val").map(StyleId::new);
                skip_tail(reader, empty, part)?;
            }
            b"numPr" => ppr.num_pr = Some(parse_num_pr(reader, empty, ctx, part)?),
            b"spacing" => {
                ppr.spacing = Some(parse_spacing(&attrs, ctx, part)?);
                skip_tail(reader, empty, part)?;
            }
            b"ind" => {
                ppr.ind = Some(parse_ind(&attrs, ctx, part)?);
                skip_tail(reader, empty, part)?;
            }
            b"jc" => {
                ppr.jc = find(&attrs, "val").map(justification);
                skip_tail(reader, empty, part)?;
            }
            b"keepNext" => {
                ppr.keep_next = attr_toggle(&attrs, ctx, part, "w:keepNext")?;
                skip_tail(reader, empty, part)?;
            }
            b"keepLines" => {
                ppr.keep_lines = attr_toggle(&attrs, ctx, part, "w:keepLines")?;
                skip_tail(reader, empty, part)?;
            }
            b"pageBreakBefore" => {
                ppr.page_break_before = attr_toggle(&attrs, ctx, part, "w:pageBreakBefore")?;
                skip_tail(reader, empty, part)?;
            }
            b"widowControl" => {
                ppr.widow_control = attr_toggle(&attrs, ctx, part, "w:widowControl")?;
                skip_tail(reader, empty, part)?;
            }
            b"outlineLvl" => {
                ppr.outline_lvl = attr_outline_lvl(&attrs, ctx, part)?;
                skip_tail(reader, empty, part)?;
            }
            b"pBdr" => ppr.p_bdr = Some(parse_pbdr(reader, empty, ctx, part)?),
            b"shd" => {
                ppr.shd = Some(parse_shading(&attrs));
                skip_tail(reader, empty, part)?;
            }
            b"tabs" => ppr.tabs = parse_tabs(reader, empty, ctx, part)?,
            b"rPr" => ppr.r_pr = Some(parse_rpr(reader, empty, ctx, part)?),
            // `w:sectPr` внутри уровня бессмыслен (уровень — не абзац), но терять
            // его молча нельзя — как и прочее неподдержанное:
            // TODO (Спринт 9): разбирать `w:sectPr` общим с `document.rs` парсером.
            _ => capture_unknown(reader, child, empty, ctx, part, &mut ppr.unknown)?,
        }
        Ok(())
    })?;
    Ok(ppr)
}

/// Разобрать `w:rPr` уровня.
///
/// # Errors
/// Как у [`parse`].
fn parse_rpr(
    reader: &mut XmlReader<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<RawRPr> {
    let mut rpr = RawRPr::default();
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        match local_name(child.name().as_ref()) {
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
            b"color" => rpr.color = parse_color(&attrs, ctx, part)?,
            b"sz" => rpr.sz = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            b"szCs" => rpr.sz_cs = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            b"highlight" => rpr.highlight = find(&attrs, "val").map(highlight),
            b"u" => {
                rpr.u = Some(find(&attrs, "val").map_or(Underline::Single, underline));
            }
            b"vertAlign" => rpr.vert_align = parse_vert_align(&attrs, ctx, part)?,
            b"spacing" => {
                rpr.spacing = Some(CharacterSpacing {
                    value: attr_i32(&attrs, "val", ctx, part)?.map(Twips::new),
                });
            }
            b"position" => rpr.position = attr_i32(&attrs, "val", ctx, part)?.map(HalfPoint::new),
            _ => capture_unknown(reader, child, empty, ctx, part, &mut rpr.unknown)?,
        }
        // Листовые элементы свойств пусты; содержимое у них бывает только в
        // испорченном файле — тогда его пропускаем.
        skip_tail(reader, empty, part)
    })?;
    Ok(rpr)
}

/// Разобрать `w:numPr` — ссылку абзаца на список.
///
/// # Errors
/// Как у [`parse`].
fn parse_num_pr(
    reader: &mut XmlReader<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<NumPr> {
    let mut num_pr = NumPr::default();
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        match local_name(child.name().as_ref()) {
            b"ilvl" => {
                if let Some(raw) = attr_u32(&attrs, "val", ctx, part)? {
                    match u8::try_from(raw) {
                        Ok(value) if value <= MAX_ILVL => num_pr.ilvl = Some(value),
                        _ => ctx.warn(
                            WarningKind::InvalidAttribute,
                            part,
                            format!("`w:numPr/w:ilvl`: `w:val` {raw} is outside 0..={MAX_ILVL}, ignored"),
                        )?,
                    }
                }
            }
            b"numId" => num_pr.num_id = attr_u32(&attrs, "val", ctx, part)?.map(NumId::new),
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unknown element `{}` in `w:numPr`, skipped",
                        element_name(child)
                    ),
                )?;
            }
        }
        skip_tail(reader, empty, part)
    })?;
    Ok(num_pr)
}

/// Разобрать `w:spacing` свойств абзаца.
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_spacing(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<ParagraphSpacing> {
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

/// Разобрать `w:ind` — отступы уровня.
///
/// `w:start`/`w:end` — написание ISO Strict; `w:left`/`w:right` — Transitional,
/// его пишет Word. Оба означают одно и то же (ADR-0017: читаем и то, и то).
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_ind(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Ind> {
    Ok(Ind {
        left: attr_i32(attrs, "left", ctx, part)?
            .or(attr_i32(attrs, "start", ctx, part)?)
            .map(Twips::new),
        right: attr_i32(attrs, "right", ctx, part)?
            .or(attr_i32(attrs, "end", ctx, part)?)
            .map(Twips::new),
        first_line: attr_i32(attrs, "firstLine", ctx, part)?.map(Twips::new),
        hanging: attr_i32(attrs, "hanging", ctx, part)?.map(Twips::new),
    })
}

/// Разобрать `w:outlineLvl`: уровень структуры документа 0…9.
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn attr_outline_lvl(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<u8>> {
    let Some(raw) = attr_u32(attrs, "val", ctx, part)? else {
        return Ok(None);
    };
    match u8::try_from(raw) {
        Ok(value) if value <= 9 => Ok(Some(value)),
        _ => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:outlineLvl`: `w:val` {raw} is outside 0..=9, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Разобрать `w:pBdr` — границы абзаца уровня.
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_pbdr(
    reader: &mut XmlReader<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<ParagraphBorders> {
    let mut borders = ParagraphBorders::default();
    read_children(reader, empty, part, |reader, child, empty| {
        let attrs = attributes(child, part)?;
        let border = parse_border(&attrs, ctx, part)?;
        match local_name(child.name().as_ref()) {
            b"top" => borders.top = border,
            b"left" | b"start" => borders.left = border,
            b"bottom" => borders.bottom = border,
            b"right" | b"end" => borders.right = border,
            b"between" => borders.between = border,
            b"bar" => borders.bar = border,
            _ => {
                ctx.warn(
                    WarningKind::UnknownElement,
                    part,
                    format!(
                        "unknown element `{}` in `w:pBdr`, skipped",
                        element_name(child)
                    ),
                )?;
            }
        }
        skip_tail(reader, empty, part)
    })?;
    Ok(borders)
}

/// Разобрать одну границу (`w:top`, `w:left`, …).
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_border(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<Border>> {
    let Some(raw) = find(attrs, "val") else {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            "border without `w:val` is skipped",
        )?;
        return Ok(None);
    };
    Ok(Some(Border {
        val: border_style(raw),
        sz: attr_u32(attrs, "sz", ctx, part)?,
        space: attr_u32(attrs, "space", ctx, part)?,
        color: find(attrs, "color").and_then(color),
    }))
}

/// Разобрать `w:shd` — заливку абзаца уровня.
///
/// Без `w:val` заливка считается `clear`: так её понимает Word, и терять
/// `w:fill`/`w:color` из-за отсутствующего узора незачем.
fn parse_shading(attrs: &[Attr<'_>]) -> Shading {
    Shading {
        val: find(attrs, "val").map_or(ShadingPattern::Clear, shading_pattern),
        color: find(attrs, "color").and_then(color),
        fill: find(attrs, "fill").and_then(color),
    }
}

/// Разобрать `w:tabs` — позиции табуляции уровня.
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_tabs(
    reader: &mut XmlReader<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Vec<TabStop>> {
    let mut tabs = Vec::new();
    read_children(reader, empty, part, |reader, child, empty| {
        if local_name(child.name().as_ref()) != b"tab" {
            ctx.warn(
                WarningKind::UnknownElement,
                part,
                format!(
                    "unknown element `{}` in `w:tabs`, skipped",
                    element_name(child)
                ),
            )?;
            return skip_tail(reader, empty, part);
        }
        let attrs = attributes(child, part)?;
        let Some(pos) = attr_i32(&attrs, "pos", ctx, part)? else {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                "`w:tab` without a valid `w:pos` is skipped",
            )?;
            return skip_tail(reader, empty, part);
        };
        let Some(kind) = find(&attrs, "val").map(tab_stop_kind) else {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                "`w:tab` without `w:val` is skipped",
            )?;
            return skip_tail(reader, empty, part);
        };
        tabs.push(TabStop {
            val: Twips::new(pos),
            kind,
            leader: find(&attrs, "leader").map_or(TabLeader::None, tab_leader),
        });
        Ok(())
    })?;
    Ok(tabs)
}

/// Разобрать `w:rFonts`.
///
/// Шрифты темы (`w:asciiTheme`, …) модель не хранит: в ней есть только явные
/// имена, и выдумывать имена из темы на разборе нельзя — тема живёт в другой части.
fn parse_rfonts(attrs: &[Attr<'_>]) -> RFonts {
    RFonts {
        ascii: find(attrs, "ascii").map(str::to_owned),
        h_ansi: find(attrs, "hAnsi").map(str::to_owned),
        east_asia: find(attrs, "eastAsia").map(str::to_owned),
        cs: find(attrs, "cs").map(str::to_owned),
        hint: find(attrs, "hint").map(font_hint),
    }
}

/// Разобрать `w:color` (или `w:fill`, `w:color` границы).
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_color(attrs: &[Attr<'_>], ctx: &mut ParseCtx, part: &str) -> Result<Option<Color>> {
    let Some(raw) = find(attrs, "val") else {
        return Ok(None);
    };
    if let Some(parsed) = color(raw) {
        return Ok(Some(parsed));
    }
    ctx.warn(
        WarningKind::InvalidAttribute,
        part,
        format!("`w:color`: unrecognized `w:val` `{raw}`, ignored"),
    )?;
    Ok(None)
}

/// Цвет из строки: `auto`, `none` или `RRGGBB` без альфы (`ST_HexColor`).
fn color(raw: &str) -> Option<Color> {
    match raw.trim() {
        "auto" => Some(Color::Auto),
        "none" => Some(Color::None),
        hex if hex.len() == 6 => u32::from_str_radix(hex, 16).ok().map(Color::Rgb),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Значения перечислений
// ---------------------------------------------------------------------------

/// Вид многоуровневости `w:multiLevelType`; `None` — значение вне `ST_MultiLevelType`.
fn multi_level_type(raw: &str) -> Option<MultiLevelType> {
    match raw.trim() {
        "singleLevel" => Some(MultiLevelType::SingleLevel),
        "multilevel" => Some(MultiLevelType::Multilevel),
        "hybridMultilevel" => Some(MultiLevelType::HybridMultilevel),
        _ => None,
    }
}

/// Символ после номера `w:suff`; `None` — значение вне `ST_LevelSuffix`.
fn level_suffix(raw: &str) -> Option<LevelSuffix> {
    match raw.trim() {
        "tab" => Some(LevelSuffix::Tab),
        "space" => Some(LevelSuffix::Space),
        "nothing" => Some(LevelSuffix::Nothing),
        _ => None,
    }
}

/// Выравнивание `w:lvlJc`/`w:jc`; незнакомое значение хранится как есть.
fn justification(raw: &str) -> Justification {
    match raw.trim() {
        "left" => Justification::Left,
        "center" => Justification::Center,
        "right" => Justification::Right,
        "both" => Justification::Both,
        "distribute" => Justification::Distribute,
        "start" => Justification::Start,
        "end" => Justification::End,
        other => Justification::Other(other.to_owned()),
    }
}

/// Вид подчёркивания `w:u`; незнакомое значение хранится как есть.
fn underline(raw: &str) -> Underline {
    match raw.trim() {
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
        other => Underline::Other(other.to_owned()),
    }
}

/// Цвет выделения `w:highlight`; незнакомое значение хранится как есть.
fn highlight(raw: &str) -> Highlight {
    match raw.trim() {
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
    }
}

/// Вертикальное смещение `w:vertAlign`.
///
/// Варианта «прочее» у [`VertAlign`] нет, поэтому незнакомое значение
/// отбрасывается с предупреждением.
///
/// # Errors
/// [`Error::TooManyWarnings`] — как у [`parse`].
fn parse_vert_align(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Option<VertAlign>> {
    let Some(raw) = find(attrs, "val") else {
        return Ok(None);
    };
    match raw.trim() {
        "baseline" => Ok(Some(VertAlign::Baseline)),
        "superscript" => Ok(Some(VertAlign::Superscript)),
        "subscript" => Ok(Some(VertAlign::Subscript)),
        other => {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!("`w:vertAlign`: unrecognized `w:val` `{other}`, ignored"),
            )?;
            Ok(None)
        }
    }
}

/// Подсказка шрифта `w:hint`; незнакомое значение хранится как есть.
fn font_hint(raw: &str) -> FontHint {
    match raw.trim() {
        "default" => FontHint::Default,
        "eastAsia" => FontHint::EastAsia,
        "cs" => FontHint::Cs,
        other => FontHint::Other(other.to_owned()),
    }
}

/// Стиль границы `w:val`; незнакомое значение хранится как есть.
///
/// Перечень — `ST_Border` целиком: сокращать его нельзя, иначе граница потеряет
/// вид, а `Other` заведён для значений вне стандарта.
fn border_style(raw: &str) -> BorderStyle {
    match raw.trim() {
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

/// Узор заливки `w:val`; незнакомое значение хранится как есть.
fn shading_pattern(raw: &str) -> ShadingPattern {
    match raw.trim() {
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

/// Вид позиции табуляции `w:tab/@w:val`; незнакомое значение хранится как есть.
fn tab_stop_kind(raw: &str) -> TabStopKind {
    match raw.trim() {
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

/// Заполнитель табуляции `w:leader`; незнакомое значение хранится как есть.
fn tab_leader(raw: &str) -> TabLeader {
    match raw.trim() {
        "none" => TabLeader::None,
        "dot" => TabLeader::Dot,
        "hyphen" => TabLeader::Hyphen,
        "middleDot" => TabLeader::MiddleDot,
        "heavy" => TabLeader::Heavy,
        "underscore" => TabLeader::Underscore,
        other => TabLeader::Other(other.to_owned()),
    }
}

/// Правило высоты строки `w:lineRule`; незнакомое значение хранится как есть.
fn line_spacing_rule(raw: &str) -> LineSpacingRule {
    match raw.trim() {
        "auto" => LineSpacingRule::Auto,
        "exact" => LineSpacingRule::Exact,
        "atLeast" => LineSpacingRule::AtLeast,
        other => LineSpacingRule::Other(other.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Обход потока
// ---------------------------------------------------------------------------

/// Обойти дочерние элементы начатого элемента.
///
/// `child` обязан вычитать переданный элемент целиком — разобрать или пропустить:
/// иначе его закрывающий тег сдвинет разбор родителя.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше закрывающего тега.
fn read_children<'r, F>(
    reader: &mut XmlReader<'r>,
    empty: bool,
    part: &str,
    mut child: F,
) -> Result<()>
where
    F: FnMut(&mut XmlReader<'r>, &BytesStart<'static>, bool) -> Result<()>,
{
    if empty {
        return Ok(());
    }
    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(element) => child(reader, &element, false)?,
            Event::Empty(element) => child(reader, &element, true)?,
            Event::End(_) => return Ok(()),
            _ => {}
        }
    }
    Err(Error::malformed(part, "unexpected end of input"))
}

/// Вычитать текст начатого элемента до парного `End`, разворачивая сущности.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше `End`, текст или ссылка не
/// читаются.
fn read_text(reader: &mut XmlReader<'_>, part: &str) -> Result<String> {
    let mut text = String::new();
    let mut depth = 0u32;
    loop {
        let event = reader
            .next_significant()?
            .ok_or_else(|| Error::malformed(part, "unexpected end of input"))?;
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(text),
            Event::End(_) => depth -= 1,
            Event::Text(chunk) => text.push_str(
                &chunk
                    .xml10_content()
                    .map_err(|e| Error::malformed(part, format!("bad text: {e}")))?,
            ),
            Event::CData(chunk) => text.push_str(
                &chunk
                    .xml10_content()
                    .map_err(|e| Error::malformed(part, format!("bad CDATA: {e}")))?,
            ),
            // В `quick-xml` 0.41 ссылки приходят отдельным событием.
            Event::GeneralRef(reference) => text.push_str(&resolve_reference(&reference, part)?),
            _ => {}
        }
    }
}

/// Вычитать начатый элемент целиком, не заглядывая внутрь.
///
/// # Errors
/// [`Error::malformed`] — поток кончился раньше парного `End`.
fn skip_element(reader: &mut XmlReader<'_>, part: &str) -> Result<()> {
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

/// Дочитать элемент до парного `End`, если он не самозакрытый.
///
/// # Errors
/// Как у [`skip_element`].
fn skip_tail(reader: &mut XmlReader<'_>, empty: bool, part: &str) -> Result<()> {
    if empty {
        Ok(())
    } else {
        skip_element(reader, part)
    }
}

/// Сохранить нераспознанный дочерний элемент целиком (ADR-0014 §3) и предупредить.
///
/// Элемент не отбрасывается: раскладка и отладка должны увидеть неподдержанное
/// свойство, а не молча его потерять.
///
/// # Errors
/// Как у [`parse`].
fn capture_unknown(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'static>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
    unknown: &mut Vec<(String, String)>,
) -> Result<()> {
    let name = element_name(element);
    ctx.warn(
        WarningKind::UnknownElement,
        part,
        format!("unknown element `{name}`, kept as `Unknown`"),
    )?;
    let xml = if empty {
        verbatim_empty(element, part)?
    } else {
        capture_element(reader, element, ctx, part)?
    };
    unknown.push((name, xml));
    Ok(())
}

/// Локальное имя элемента строкой — для сообщений и `Unknown`.
fn element_name(element: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(local_name(element.name().as_ref())).into_owned()
}

/// XML самозакрытого элемента дословно: имя и атрибуты — те же байты, что в файле.
///
/// # Errors
/// [`Error::malformed`] — имя или атрибуты события не UTF-8.
fn verbatim_empty(element: &BytesStart<'_>, part: &str) -> Result<String> {
    let raw_name = element.name();
    let name = std::str::from_utf8(raw_name.as_ref())
        .map_err(|e| Error::malformed(part, format!("element name is not UTF-8: {e}")))?;
    let attrs = std::str::from_utf8(element.attributes_raw())
        .map_err(|e| Error::malformed(part, format!("element attributes are not UTF-8: {e}")))?;
    Ok(format!("<{name}{attrs}/>"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::raw::Toggle;
    use doc_converter_core::Archive;

    const PART: &str = "word/numbering.xml";

    /// Каталог фикстур DOCX — тот же, что у сквозных тестов крейта.
    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test-fixtures/docx");

    /// Часть с корнем `w:numbering` и объявленным пространством имён.
    fn part_xml(body: &str) -> String {
        format!(
            "<w:numbering \
             xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{body}\
             </w:numbering>"
        )
    }

    /// Разобрать часть из строки: таблица и виды предупреждений.
    fn parse_part(xml: &str) -> (NumberingTable, Vec<WarningKind>) {
        let mut ctx = ParseCtx::new();
        let table = parse(xml.as_bytes(), &mut ctx, PART).expect("часть разбирается");
        (table, ctx.warnings().iter().map(|w| w.kind).collect())
    }

    /// Уровень 0 единственной схемы части; предупреждений быть не должно.
    fn level_zero(body: &str) -> Lvl {
        let xml = part_xml(&format!(
            "<w:abstractNum w:abstractNumId=\"0\">{body}</w:abstractNum>"
        ));
        let (table, warnings) = parse_part(&xml);
        assert!(
            warnings.is_empty(),
            "неожиданные предупреждения: {warnings:?}"
        );
        table.abstract_nums[&AbstractNumId::new(0)].levels[&0].clone()
    }

    #[test]
    fn multi_level_type_is_recognized() {
        for (raw, expected) in [
            ("singleLevel", MultiLevelType::SingleLevel),
            ("multilevel", MultiLevelType::Multilevel),
            ("hybridMultilevel", MultiLevelType::HybridMultilevel),
        ] {
            let xml = part_xml(&format!(
                "<w:abstractNum w:abstractNumId=\"0\">\
                 <w:multiLevelType w:val=\"{raw}\"/></w:abstractNum>"
            ));
            let (table, warnings) = parse_part(&xml);
            assert!(warnings.is_empty(), "{raw}: {warnings:?}");
            assert_eq!(
                table.abstract_nums[&AbstractNumId::new(0)].multi_level_type,
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn missing_or_bogus_multi_level_type_falls_back_to_multilevel() {
        let (table, warnings) = parse_part(&part_xml("<w:abstractNum w:abstractNumId=\"0\"/>"));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].multi_level_type,
            MultiLevelType::Multilevel
        );

        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\">\
             <w:multiLevelType w:val=\"bogus\"/></w:abstractNum>",
        ));
        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].multi_level_type,
            MultiLevelType::Multilevel
        );
    }

    #[test]
    fn abstract_num_links_and_ids_are_parsed() {
        let xml = part_xml(
            "<w:abstractNum w:abstractNumId=\"4\"><w:nsid w:val=\"1A2B3C4D\"/>\
             <w:tmpl w:val=\"A1B2C3D4\"/><w:styleLink w:val=\"ListParagraph\"/>\
             <w:numStyleLink w:val=\"ListNumber\"/></w:abstractNum>",
        );
        let (table, warnings) = parse_part(&xml);
        assert!(warnings.is_empty(), "{warnings:?}");
        let scheme = &table.abstract_nums[&AbstractNumId::new(4)];
        assert_eq!(scheme.id, AbstractNumId::new(4));
        assert_eq!(scheme.nsid.as_deref(), Some("1A2B3C4D"));
        assert_eq!(scheme.tmpl.as_deref(), Some("A1B2C3D4"));
        assert_eq!(
            scheme.style_link.as_ref().map(StyleId::as_str),
            Some("ListParagraph")
        );
        assert_eq!(
            scheme.num_style_link.as_ref().map(StyleId::as_str),
            Some("ListNumber")
        );
    }

    #[test]
    fn level_attributes_are_parsed() {
        let lvl = level_zero(
            "<w:lvl w:ilvl=\"0\"><w:start w:val=\"5\"/><w:numFmt w:val=\"upperRoman\"/>\
             <w:lvlText w:val=\"%1)\"/><w:lvlJc w:val=\"right\"/><w:suff w:val=\"space\"/></w:lvl>",
        );
        assert_eq!(lvl.ilvl, 0);
        assert_eq!(lvl.start, 5);
        assert_eq!(lvl.num_fmt, NumFmt::UpperRoman);
        assert_eq!(lvl.lvl_text, "%1)");
        assert_eq!(lvl.lvl_jc, Some(Justification::Right));
        assert_eq!(lvl.suff, LevelSuffix::Space);
    }

    #[test]
    fn level_defaults_match_the_schema() {
        let xml =
            part_xml("<w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"3\"/></w:abstractNum>");
        let (table, warnings) = parse_part(&xml);
        assert!(warnings.is_empty(), "{warnings:?}");
        let lvl = &table.abstract_nums[&AbstractNumId::new(0)].levels[&3];
        assert_eq!(lvl.ilvl, 3);
        assert_eq!(lvl.start, 1);
        assert_eq!(lvl.num_fmt, NumFmt::Decimal);
        assert_eq!(lvl.lvl_text, "");
        assert_eq!(lvl.suff, LevelSuffix::Tab);
        assert_eq!(lvl.lvl_jc, None);
        assert_eq!(lvl.restart, None);
        assert_eq!(lvl.pstyle, None);
        assert_eq!(lvl.pic_bullet_id, None);
        assert!(!lvl.is_lgl);
        assert!(!lvl.tentative);
        assert_eq!(lvl.ppr, RawPPr::default());
        assert_eq!(lvl.rpr, RawRPr::default());
    }

    #[test]
    fn lvl_text_accepts_a_body_and_decodes_entities() {
        let lvl = level_zero("<w:lvl w:ilvl=\"0\"><w:lvlText>%1.</w:lvlText></w:lvl>");
        assert_eq!(lvl.lvl_text, "%1.");

        let lvl = level_zero("<w:lvl w:ilvl=\"0\"><w:lvlText>%1&amp;</w:lvlText></w:lvl>");
        assert_eq!(lvl.lvl_text, "%1&");

        let lvl = level_zero("<w:lvl w:ilvl=\"0\"><w:lvlText w:val=\"&#xF0B7;\"/></w:lvl>");
        assert_eq!(lvl.lvl_text, "\u{F0B7}");
    }

    #[test]
    fn unknown_num_fmt_is_kept_as_other() {
        let lvl = level_zero("<w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bogus\"/></w:lvl>");
        assert_eq!(lvl.num_fmt, NumFmt::Other("bogus".to_owned()));
    }

    /// Таблица [`NumFmt::from_ooxml`] обязана покрывать весь перечень модели:
    /// пропущенный вариант round-trip не переживёт.
    #[test]
    fn every_num_fmt_has_an_ooxml_spelling() {
        for variant in NumFmt::ALL {
            let debug = format!("{variant:?}");
            let ooxml = debug[..1].to_ascii_lowercase() + &debug[1..];
            assert_eq!(
                NumFmt::from_ooxml(&ooxml),
                variant,
                "`{debug}` не разбирается обратно"
            );
        }
    }

    #[test]
    fn level_suffix_is_parsed_and_bogus_value_warns() {
        for (raw, expected) in [
            ("tab", LevelSuffix::Tab),
            ("space", LevelSuffix::Space),
            ("nothing", LevelSuffix::Nothing),
        ] {
            let lvl = level_zero(&format!(
                "<w:lvl w:ilvl=\"0\"><w:suff w:val=\"{raw}\"/></w:lvl>"
            ));
            assert_eq!(lvl.suff, expected, "{raw}");
        }

        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\">\
             <w:lvl w:ilvl=\"0\"><w:suff w:val=\"bogus\"/></w:lvl></w:abstractNum>",
        ));
        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].levels[&0].suff,
            LevelSuffix::Tab
        );
    }

    #[test]
    fn level_restart_is_parsed_from_both_spellings() {
        let lvl = level_zero("<w:lvl w:ilvl=\"0\"><w:lvlRestart w:val=\"0\"/></w:lvl>");
        assert_eq!(lvl.restart, Some(0));

        let lvl = level_zero("<w:lvl w:ilvl=\"0\"><w:restart w:val=\"2\"/></w:lvl>");
        assert_eq!(lvl.restart, Some(2));
    }

    #[test]
    fn level_flags_and_bullet_reference_are_parsed() {
        let lvl = level_zero(
            "<w:lvl w:ilvl=\"0\"><w:pStyle w:val=\"ListParagraph\"/><w:isLgl/>\
             <w:tentative/><w:lvlPicBulletId w:val=\"3\"/></w:lvl>",
        );
        assert_eq!(
            lvl.pstyle.as_ref().map(StyleId::as_str),
            Some("ListParagraph")
        );
        assert!(lvl.is_lgl);
        assert!(lvl.tentative);
        assert_eq!(lvl.pic_bullet_id, Some(3));

        let lvl = level_zero(
            "<w:lvl w:ilvl=\"0\"><w:isLgl w:val=\"0\"/><w:tentative w:val=\"false\"/></w:lvl>",
        );
        assert!(!lvl.is_lgl);
        assert!(!lvl.tentative);
    }

    #[test]
    fn ppr_and_rpr_are_parsed() {
        let lvl = level_zero(
            "<w:lvl w:ilvl=\"0\">\
             <w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/><w:jc w:val=\"both\"/><w:keepNext/></w:pPr>\
             <w:rPr><w:rFonts w:ascii=\"Symbol\" w:hAnsi=\"Symbol\" w:hint=\"default\"/>\
             <w:b/><w:u w:val=\"wave\"/><w:sz w:val=\"24\"/><w:color w:val=\"FF0000\"/></w:rPr>\
             </w:lvl>",
        );
        let ind = lvl.ppr.ind.as_ref().expect("`w:ind` разобран");
        assert_eq!(ind.left, Some(Twips::new(720)));
        assert_eq!(ind.hanging, Some(Twips::new(360)));
        assert_eq!(lvl.ppr.jc, Some(Justification::Both));
        assert_eq!(lvl.ppr.keep_next, Some(Toggle::On));

        let fonts = lvl.rpr.r_fonts.as_ref().expect("`w:rFonts` разобран");
        assert_eq!(fonts.ascii.as_deref(), Some("Symbol"));
        assert_eq!(fonts.h_ansi.as_deref(), Some("Symbol"));
        assert_eq!(fonts.hint, Some(FontHint::Default));
        assert_eq!(lvl.rpr.b, Some(Toggle::On));
        assert_eq!(lvl.rpr.u, Some(Underline::Wave));
        assert_eq!(lvl.rpr.sz, Some(HalfPoint::new(24)));
        assert_eq!(lvl.rpr.color, Some(Color::Rgb(0x00FF_0000)));
    }

    #[test]
    fn unknown_level_child_is_skipped_with_a_warning() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\">\
             <w:foo w:bar=\"1\"><w:baz/></w:foo><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum>",
        ));
        assert_eq!(warnings, vec![WarningKind::UnknownElement]);
        // Разбор продолжился: элемент после неизвестного на месте.
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].levels[&0].num_fmt,
            NumFmt::Bullet
        );
    }

    #[test]
    fn unknown_ppr_child_is_kept_as_verbatim_xml() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\"><w:pPr>\
             <w:contextualSpacing/><w:suppressAutoHyphens w:val=\"0\"/></w:pPr></w:lvl></w:abstractNum>",
        ));
        assert_eq!(
            warnings,
            vec![WarningKind::UnknownElement, WarningKind::UnknownElement]
        );
        let unknown = &table.abstract_nums[&AbstractNumId::new(0)].levels[&0]
            .ppr
            .unknown;
        assert_eq!(
            unknown.as_slice(),
            [
                (
                    "contextualSpacing".to_owned(),
                    "<w:contextualSpacing/>".to_owned()
                ),
                (
                    "suppressAutoHyphens".to_owned(),
                    "<w:suppressAutoHyphens w:val=\"0\"/>".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn level_with_ilvl_outside_the_range_is_skipped() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\">\
             <w:lvl w:ilvl=\"9\"/><w:lvl w:ilvl=\"0\"/></w:abstractNum>",
        ));
        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        let levels = &table.abstract_nums[&AbstractNumId::new(0)].levels;
        assert_eq!(levels.len(), 1);
        assert!(levels.contains_key(&0));

        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"abc\"/></w:abstractNum>",
        ));
        assert_eq!(
            warnings,
            vec![WarningKind::InvalidAttribute, WarningKind::InvalidAttribute]
        );
        assert!(table.abstract_nums[&AbstractNumId::new(0)]
            .levels
            .is_empty());
    }

    #[test]
    fn duplicate_ids_keep_the_first_definition() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"singleLevel\"/></w:abstractNum>\
             <w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"multilevel\"/></w:abstractNum>\
             <w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/>\
             <w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"5\"/></w:lvlOverride></w:num>\
             <w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>",
        ));
        assert_eq!(
            warnings,
            vec![WarningKind::InvalidAttribute, WarningKind::InvalidAttribute]
        );
        assert_eq!(table.abstract_nums.len(), 1);
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].multi_level_type,
            MultiLevelType::SingleLevel
        );
        assert_eq!(table.nums.len(), 1);
        assert_eq!(
            table.nums[&NumId::new(1)].overrides[&0].start_override,
            Some(5)
        );
    }

    #[test]
    fn duplicate_level_and_override_keep_the_first() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\">\
             <w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/></w:lvl>\
             <w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum>\
             <w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/>\
             <w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"3\"/></w:lvlOverride>\
             <w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"9\"/></w:lvlOverride></w:num>",
        ));
        assert_eq!(
            warnings,
            vec![WarningKind::InvalidAttribute, WarningKind::InvalidAttribute]
        );
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].levels[&0].num_fmt,
            NumFmt::Decimal
        );
        assert_eq!(
            table.nums[&NumId::new(1)].overrides[&0].start_override,
            Some(3)
        );
    }

    #[test]
    fn level_override_replaces_or_extends_a_level() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:abstractNum w:abstractNumId=\"0\">\
             <w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/></w:lvl></w:abstractNum>\
             <w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/>\
             <w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"5\"/></w:lvlOverride>\
             <w:lvlOverride w:ilvl=\"1\"><w:lvl><w:numFmt w:val=\"bullet\"/>\
             <w:lvlText w:val=\"o\"/></w:lvl></w:lvlOverride></w:num>",
        ));
        assert!(warnings.is_empty(), "{warnings:?}");
        let num = &table.nums[&NumId::new(1)];
        assert_eq!(num.overrides[&0].start_override, Some(5));
        assert!(num.overrides[&0].lvl.is_none());
        let nested = num.overrides[&1].lvl.as_ref().expect("вложенный уровень");
        // Своего `w:ilvl` у вложенного уровня нет — берётся от `w:lvlOverride`.
        assert_eq!(nested.ilvl, 1);
        assert_eq!(nested.num_fmt, NumFmt::Bullet);
        assert_eq!(nested.lvl_text, "o");
    }

    #[test]
    fn num_without_a_scheme_is_reported_and_dropped() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:num w:numId=\"7\"><w:abstractNumId w:val=\"99\"/></w:num>",
        ));
        assert_eq!(warnings, vec![WarningKind::MissingAbstractNum]);
        assert!(table.nums.contains_key(&NumId::new(7)));

        let (table, warnings) = parse_part(&part_xml("<w:num w:numId=\"7\"/>"));
        assert_eq!(warnings, vec![WarningKind::InvalidAttribute]);
        assert!(table.nums.is_empty());
    }

    #[test]
    fn picture_bullet_definition_is_skipped_without_warnings() {
        let (table, warnings) = parse_part(&part_xml(
            "<w:numPicBullet w:numPicBulletId=\"0\"><w:pict>\
             <v:shape xmlns:v=\"urn:schemas-microsoft-com:vml\"/></w:pict></w:numPicBullet>\
             <w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\">\
             <w:lvlPicBulletId w:val=\"0\"/></w:lvl></w:abstractNum>",
        ));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            table.abstract_nums[&AbstractNumId::new(0)].levels[&0].pic_bullet_id,
            Some(0)
        );
    }

    #[test]
    fn unknown_root_element_is_skipped_with_a_warning() {
        let xml = part_xml("<w:foo/><w:abstractNum w:abstractNumId=\"0\"/>");
        let (table, warnings) = parse_part(&xml);
        assert_eq!(warnings, vec![WarningKind::UnknownElement]);
        assert!(table.abstract_nums.contains_key(&AbstractNumId::new(0)));
    }

    #[test]
    fn empty_numbering_is_an_empty_table() {
        let (table, warnings) = parse_part(&part_xml(""));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(table, NumberingTable::default());
    }

    #[test]
    fn broken_xml_is_an_error() {
        let mut ctx = ParseCtx::new();
        let err = parse(
            b"<w:numbering><w:abstractNum w:abstractNumId=\"0\">",
            &mut ctx,
            PART,
        )
        .expect_err("обрыв потока — ошибка");
        assert!(matches!(err, Error::Malformed { .. }), "{err}");

        let mut ctx = ParseCtx::new();
        let err = parse(b"<w:numbering></w:body>", &mut ctx, PART)
            .expect_err("несовпадающие теги — ошибка");
        assert!(matches!(err, Error::Core(_)), "{err}");
    }

    // -----------------------------------------------------------------------
    // Фикстуры
    // -----------------------------------------------------------------------

    #[derive(serde::Deserialize)]
    struct Sidecar {
        metadata: SidecarMetadata,
        content: SidecarContent,
    }

    #[derive(serde::Deserialize)]
    struct SidecarMetadata {
        #[serde(rename = "expectedWarnings")]
        expected_warnings: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    struct SidecarContent {
        numbering: Option<Vec<SidecarNum>>,
    }

    #[derive(serde::Deserialize)]
    struct SidecarNum {
        #[serde(rename = "numId")]
        num_id: u32,
        #[serde(rename = "abstractNumId")]
        abstract_num_id: u32,
        levels: Vec<SidecarLevel>,
    }

    #[derive(serde::Deserialize)]
    struct SidecarLevel {
        ilvl: u8,
        #[serde(rename = "numFmt")]
        num_fmt: String,
        #[serde(rename = "lvlText")]
        lvl_text: String,
        start: u32,
        font: Option<String>,
        #[serde(rename = "lvlRestart")]
        lvl_restart: Option<u8>,
        #[serde(rename = "startOverride")]
        start_override: Option<u32>,
    }

    /// Разобрать `word/numbering.xml` фикстуры (`numbering/decimal_basic`, …).
    fn parse_fixture(relative: &str) -> (NumberingTable, Vec<WarningKind>) {
        let path = format!("{FIXTURES}/{relative}.docx");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut archive = Archive::new(bytes).expect("архив открывается");
        let part = archive.read(PART).expect("`word/numbering.xml` есть");
        let mut ctx = ParseCtx::new();
        let table = parse(&part, &mut ctx, PART).expect("часть разбирается");
        (table, ctx.warnings().iter().map(|w| w.kind).collect())
    }

    /// Сайдкар фикстуры: ожидания по части и предупреждениям.
    fn read_sidecar(relative: &str) -> Sidecar {
        let path = format!("{FIXTURES}/{relative}.json");
        let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// Вид предупреждения по имени из сайдкара.
    ///
    /// Генератор фикстур пишет имя варианта (`MissingAbstractNum`); написание
    /// `snake_case` принимаем наравне — оно то же, что у [`WarningKind::as_str`].
    fn sidecar_warning_kind(raw: &str) -> WarningKind {
        WarningKind::ALL
            .iter()
            .copied()
            .find(|kind| format!("{kind:?}") == raw || kind.as_str() == raw)
            .unwrap_or_else(|| panic!("неизвестный вид предупреждения: `{raw}`"))
    }

    /// `numFmt` сайдкара — написание `ST_NumberFormat`; незнакомое значение в
    /// сайдкаре означает ошибку в нём, а не в парсере, поэтому паника.
    fn sidecar_num_fmt(raw: &str) -> NumFmt {
        match NumFmt::from_ooxml(raw) {
            NumFmt::Other(other) => panic!("неожиданный `numFmt` в сайдкаре: `{other}`"),
            known => known,
        }
    }

    /// Ни одно предупреждение части не выходит за список ожидаемых:
    /// часть отвечает только за свои предупреждения, чужие приходят из других парсеров.
    fn assert_warnings_are_expected(name: &str, warnings: &[WarningKind], sidecar: &Sidecar) {
        let expected: Vec<WarningKind> = sidecar
            .metadata
            .expected_warnings
            .iter()
            .map(|raw| sidecar_warning_kind(raw))
            .collect();
        for kind in warnings {
            assert!(
                expected.contains(kind),
                "{name}: предупреждение `{kind:?}` не ждали ({expected:?})"
            );
        }
    }

    #[test]
    fn fixtures_match_their_sidecars() {
        for name in [
            "numbering/decimal_basic",
            "numbering/multilevel",
            "numbering/nested_levels",
            "numbering/bullet_symbols",
            "numbering/custom_format",
            "numbering/restart_numbering",
            "numbering/start_override",
        ] {
            let sidecar = read_sidecar(name);
            let (table, warnings) = parse_fixture(name);
            assert_warnings_are_expected(name, &warnings, &sidecar);

            let expected = sidecar.content.numbering.unwrap_or_default();
            assert_eq!(table.nums.len(), expected.len(), "{name}: число списков");
            for num in &expected {
                let parsed = table
                    .nums
                    .get(&NumId::new(num.num_id))
                    .unwrap_or_else(|| panic!("{name}: нет списка `{}`", num.num_id));
                assert_eq!(
                    parsed.abstract_id,
                    AbstractNumId::new(num.abstract_num_id),
                    "{name}: список {}",
                    num.num_id
                );
                let scheme = &table.abstract_nums[&parsed.abstract_id];
                for level in &num.levels {
                    let lvl = scheme.levels.get(&level.ilvl).unwrap_or_else(|| {
                        panic!(
                            "{name}: нет уровня {} у схемы {}",
                            level.ilvl, num.abstract_num_id
                        )
                    });
                    assert_eq!(
                        lvl.num_fmt,
                        sidecar_num_fmt(&level.num_fmt),
                        "{name}: ilvl {}",
                        level.ilvl
                    );
                    assert_eq!(lvl.lvl_text, level.lvl_text, "{name}: ilvl {}", level.ilvl);
                    assert_eq!(lvl.start, level.start, "{name}: ilvl {}", level.ilvl);
                    assert_eq!(
                        lvl.restart, level.lvl_restart,
                        "{name}: ilvl {}",
                        level.ilvl
                    );
                    if let Some(font) = &level.font {
                        let ascii = lvl.rpr.r_fonts.as_ref().and_then(|f| f.ascii.clone());
                        assert_eq!(
                            ascii.as_deref(),
                            Some(font.as_str()),
                            "{name}: шрифт маркера ilvl {}",
                            level.ilvl
                        );
                    }
                    if let Some(start) = level.start_override {
                        let over = parsed.overrides.get(&level.ilvl).unwrap_or_else(|| {
                            panic!("{name}: нет переопределения ilvl {}", level.ilvl)
                        });
                        assert_eq!(
                            over.start_override,
                            Some(start),
                            "{name}: `w:startOverride` ilvl {}",
                            level.ilvl
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn broken_fixture_reports_a_missing_abstract_num() {
        let name = "broken/missing_style_and_abstract_num";
        let sidecar = read_sidecar(name);
        let (table, warnings) = parse_fixture(name);
        assert_warnings_are_expected(name, &warnings, &sidecar);

        assert_eq!(warnings, vec![WarningKind::MissingAbstractNum]);
        assert!(table.nums.contains_key(&NumId::new(7)));
        assert!(!table.abstract_nums.contains_key(&AbstractNumId::new(99)));
    }
}
