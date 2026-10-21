//! Разбор PresentationML-пакета в модель (ADR-0033 §3).
//!
//! Порядок: `_rels/.rels` → `ppt/presentation.xml` (размер `p:sldSz`, список
//! `p:sldIdLst`) → мастер `p:sldMaster` и тема `a:theme` → слайды
//! `ppt/slides/slideN.xml` (шейпы, плейсхолдеры, картинки, диаграммы).
//!
//! Политика ошибок — как у DOCX (ADR-0016): фатально только то, без чего
//! модель недостоверна (нет `presentation.xml`, нет слайдов); всё
//! восстановимое — битый шейп, ненайденная картинка, незнакомый атрибут —
//! копится в [`Presentation::warnings`] и не роняет пакет.
//!
//! Отмена — кооперативный флаг между слайдами (ADR-0032): разбор большой
//! презентации можно прервать, состояние после [`Error::Cancelled`] цело.

use std::cell::Cell;
use std::collections::BTreeMap;

use doc_converter_core::rels::RelMap;
use doc_converter_core::zip_limits::ZipLimits;
use doc_converter_core::{Archive, SegmentedBuffer};
use doc_converter_render::chart::{ChartData, ChartKind, ChartSeries};

use crate::error::{Error, Result};
use crate::model::{
    Align, Anchor, Bullet, Emu, Frame, GeometryPreset, ImagePart, Insets, Line, Paragraph,
    Presentation, Rgb, Run, RunProps, SchemeColor, Shape, ShapeKind, Slide, SlideSize, TextBody,
    TextBox, Theme,
};
use crate::xml::{parse_dom, Node};

/// Хвост типа relationship: `…/officeDocument`.
const REL_OFFICE_DOCUMENT: &str = "/officeDocument";
/// Хвост типа relationship: `…/slideLayout`.
const REL_SLIDE_LAYOUT: &str = "/slideLayout";
/// Хвост типа relationship: `…/theme`.
const REL_THEME: &str = "/theme";

/// Кегль по умолчанию, pt — умолчание `PowerPoint`.
const DEFAULT_SIZE_PT: f32 = 18.0;

/// Шрифт по умолчанию, когда тема не прочиталась.
const DEFAULT_FONT: &str = "Calibri";

/// Предел предупреждений: оборона от пакета-мусорки.
const MAX_WARNINGS: usize = 256;

/// Разобрать PPTX из среза байтов.
///
/// # Errors
/// [`Error::Core`] — не ZIP, нарушены лимиты ADR-0015, нет `presentation.xml`;
/// [`Error::Malformed`] — структура `PresentationML` не сошлась;
/// [`Error::NoSlides`] — в презентации нет слайдов.
pub fn parse(bytes: &[u8], limits: &ZipLimits) -> Result<Presentation> {
    parse_with_cancel(bytes, limits, &Cell::new(false))
}

/// Разобрать PPTX из байтов, взяв их во владение (без копии).
///
/// # Errors
/// То же, что у [`parse`].
pub fn parse_owned(bytes: Vec<u8>, limits: ZipLimits) -> Result<Presentation> {
    let archive = Archive::open_with_limits(bytes, limits)?;
    parse_archive(archive, &Cell::new(false))
}

/// Разобрать PPTX из потокового буфера чанков (ADR-0031).
///
/// # Errors
/// То же, что у [`parse`].
pub fn parse_segmented(buf: SegmentedBuffer, limits: ZipLimits) -> Result<Presentation> {
    let archive = Archive::from_segmented_with_limits(buf, limits)?;
    parse_archive(archive, &Cell::new(false))
}

/// Разобрать PPTX с кооперативной отменой: флаг проверяется между слайдами.
///
/// # Errors
/// То же, что у [`parse`], плюс [`Error::Cancelled`] — флаг взведён во время
/// разбора.
pub fn parse_with_cancel(
    bytes: &[u8],
    limits: &ZipLimits,
    cancelled: &Cell<bool>,
) -> Result<Presentation> {
    let archive = Archive::open_with_limits(bytes.to_vec(), limits.clone())?;
    parse_archive(archive, cancelled)
}

/// Общий разбор открытого архива.
fn parse_archive(mut archive: Archive, cancelled: &Cell<bool>) -> Result<Presentation> {
    let office_part = office_document_part(&mut archive)?;
    let pres_xml = archive.read(&office_part)?;
    let pres_dom = parse_dom(&pres_xml, &office_part)?;
    let pres_dom = doc_element(&pres_dom, "presentation");
    let pres_rels = archive.rels_for(&office_part)?.unwrap_or_default();

    let size = slide_size(pres_dom);
    let mut theme = Theme::default();
    let master = read_master(
        pres_dom,
        &pres_rels,
        &office_part,
        &mut archive,
        &mut theme,
    );

    let mut ctx = Ctx {
        images: Vec::new(),
        image_by_part: BTreeMap::new(),
        warnings: Vec::new(),
        theme: &theme,
    };

    let mut slides = Vec::new();
    if let Some(list) = pres_dom.child("sldIdLst") {
        for (index, sld_id) in list.children_named("sldId").enumerate() {
            if cancelled.get() {
                return Err(Error::Cancelled);
            }
            let Some(rid) = sld_id.attr("r:id") else {
                ctx.warn(format!("slide {index}: no r:id, skipped"));
                continue;
            };
            let Some(rel) = pres_rels.get(rid) else {
                ctx.warn(format!(
                    "slide {index}: unresolved relationship {rid}, skipped"
                ));
                continue;
            };
            let Some(part) = rel.part(&office_part) else {
                ctx.warn(format!("slide {index}: external target, skipped"));
                continue;
            };
            let xml = match archive.read(&part) {
                Ok(xml) => xml,
                Err(error) => {
                    ctx.warn(format!("slide {index}: {error}"));
                    continue;
                }
            };
            let dom = match parse_dom(&xml, &part) {
                Ok(dom) => dom,
                Err(error) => {
                    ctx.warn(format!("slide {index}: {error}"));
                    continue;
                }
            };
            let slide_rels = archive.rels_for(&part)?.unwrap_or_default();
            let layout = read_layout(&slide_rels, &part, &mut archive, &mut ctx);
            slides.push(parse_slide(
                index,
                &dom,
                &slide_rels,
                &part,
                &mut archive,
                &layout,
                &master,
                &mut ctx,
            ));
        }
    }

    if slides.is_empty() {
        return Err(Error::NoSlides);
    }

    // Деструктуризация гасит заимствование `ctx.theme` перед переносом темы
    // в результат.
    let Ctx {
        images, warnings, ..
    } = ctx;
    Ok(Presentation {
        size,
        slides,
        images,
        theme,
        warnings,
    })
}

/// Контекст разбора: media-части, предупреждения, тема.
struct Ctx<'a> {
    images: Vec<ImagePart>,
    image_by_part: BTreeMap<String, u32>,
    warnings: Vec<String>,
    theme: &'a Theme,
}

impl Ctx<'_> {
    /// Записать восстановимое нарушение; сверх предела предупреждения
    /// отбрасываются (ADR-0015 §4).
    fn warn(&mut self, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message.into());
        }
    }
}

/// Найти часть `presentation.xml` через `_rels/.rels`; без связей —
/// стандартное имя части.
fn office_document_part(archive: &mut Archive) -> Result<String> {
    if let Ok(rels_bytes) = archive.read("_rels/.rels") {
        let rels = RelMap::parse(&rels_bytes)?;
        for rel in rels.items.values() {
            if rel.rel_type.ends_with(REL_OFFICE_DOCUMENT) {
                if let Some(part) = rel.part("") {
                    return Ok(part);
                }
            }
        }
    }
    // Пакет без `_rels/.rels`: пробуем стандартное имя.
    if archive.contains("ppt/presentation.xml") {
        return Ok("ppt/presentation.xml".to_owned());
    }
    Err(Error::Malformed("missing presentation part".to_owned()))
}

/// Корневой элемент части: DOM-парсер отдаёт синтетический корень, у
/// которого документный элемент — первый ребёнок (`p:presentation`, `a:theme`).
/// Нет такого ребёнка — сам узел (битая часть), разбор продолжит на пустоте.
fn doc_element<'a>(dom: &'a Node, name: &str) -> &'a Node {
    dom.child(name).unwrap_or(dom)
}

/// Размер слайда из `p:sldSz`; без него — умолчание 16:9.
fn slide_size(dom: &Node) -> SlideSize {
    let mut size = SlideSize::default();
    if let Some(node) = dom.child("sldSz") {
        if let Some(cx) = parse_emu(node.attr("cx")) {
            size.width = cx;
        }
        if let Some(cy) = parse_emu(node.attr("cy")) {
            size.height = cy;
        }
    }
    size
}

/// EMU из десятичной строки атрибута; битое значение — `None`.
fn parse_emu(value: Option<&str>) -> Option<Emu> {
    value.and_then(|v| v.parse::<Emu>().ok())
}

// ── Мастер и тема ───────────────────────────────────────────────────────────

/// Мастер-слайд: плейсхолдеры, стили уровней текста, фон.
#[derive(Debug, Default)]
struct MasterInfo {
    placeholders: Vec<Placeholder>,
    title_style: Vec<RunProps>,
    body_style: Vec<RunProps>,
    other_style: Vec<RunProps>,
    background: Option<Rgb>,
}

/// Layout-слайд: плейсхолдеры и фон.
#[derive(Debug, Default)]
struct LayoutInfo {
    placeholders: Vec<Placeholder>,
    background: Option<Rgb>,
}

/// Плейсхолдер мастера/layout: тип, индекс и геометрия (`None` — наследуется).
#[derive(Debug)]
struct Placeholder {
    idx: Option<u32>,
    ph_type: Option<String>,
    frame: Option<Frame>,
}

/// Прочитать мастер из `p:sldMasterIdLst` и тему из его связей.
fn read_master(
    pres_dom: &Node,
    pres_rels: &RelMap,
    office_part: &str,
    archive: &mut Archive,
    theme: &mut Theme,
) -> MasterInfo {
    let Some(master_id) = pres_dom
        .child("sldMasterIdLst")
        .and_then(|list| list.child("sldMasterId"))
        .and_then(|node| node.attr("r:id"))
    else {
        return MasterInfo::default();
    };
    let Some(rel) = pres_rels.get(master_id) else {
        return MasterInfo::default();
    };
    let Some(part) = rel.part(office_part) else {
        return MasterInfo::default();
    };
    let Ok(xml) = archive.read(&part) else {
        return MasterInfo::default();
    };
    let Ok(dom) = parse_dom(&xml, &part) else {
        return MasterInfo::default();
    };
    let dom = doc_element(&dom, "sldMaster");

    // Тема — из связей мастера.
    if let Ok(Some(master_rels)) = archive.rels_for(&part) {
        for rel in master_rels.items.values() {
            if rel.rel_type.ends_with(REL_THEME) {
                if let Some(theme_part) = rel.part(&part) {
                    if let Ok(theme_xml) = archive.read(&theme_part) {
                        if let Ok(theme_dom) = parse_dom(&theme_xml, &theme_part) {
                            *theme = parse_theme(doc_element(&theme_dom, "theme"));
                        }
                    }
                }
            }
        }
    }

    parse_master(dom, theme)
}

/// Разобрать мастер: плейсхолдеры, стили уровней, фон.
fn parse_master(dom: &Node, theme: &Theme) -> MasterInfo {
    // Фон и дерево шейпов мастера живут под `p:cSld`.
    let csld = dom.child("cSld");
    let mut master = MasterInfo {
        background: csld.and_then(background_of),
        ..MasterInfo::default()
    };
    if let Some(tree) = csld.and_then(|c| c.child("spTree")) {
        master.placeholders = placeholders_of(tree);
    }
    if let Some(styles) = dom.child("txStyles") {
        if let Some(title) = styles.child("titleStyle") {
            master.title_style = level_styles(title, theme);
        }
        if let Some(body) = styles.child("bodyStyle") {
            master.body_style = level_styles(body, theme);
        }
        if let Some(other) = styles.child("otherStyle") {
            master.other_style = level_styles(other, theme);
        }
    }
    master
}

/// Прочитать layout-слайд из связей слайда.
fn read_layout(
    slide_rels: &RelMap,
    slide_part: &str,
    archive: &mut Archive,
    ctx: &mut Ctx<'_>,
) -> LayoutInfo {
    let Some(rel) = slide_rels
        .items
        .values()
        .find(|rel| rel.rel_type.ends_with(REL_SLIDE_LAYOUT))
    else {
        return LayoutInfo::default();
    };
    let Some(part) = rel.part(slide_part) else {
        return LayoutInfo::default();
    };
    let xml = match archive.read(&part) {
        Ok(xml) => xml,
        Err(error) => {
            ctx.warn(format!("slide layout: {error}"));
            return LayoutInfo::default();
        }
    };
    let dom = match parse_dom(&xml, &part) {
        Ok(dom) => dom,
        Err(error) => {
            ctx.warn(format!("slide layout: {error}"));
            return LayoutInfo::default();
        }
    };
    let dom = doc_element(&dom, "sldLayout");
    let csld = dom.child("cSld");
    LayoutInfo {
        placeholders: csld
            .and_then(|c| c.child("spTree"))
            .map_or_else(Vec::new, placeholders_of),
        background: csld.and_then(background_of),
    }
}

/// Плейсхолдеры дерева шейпов: тип, индекс и своя геометрия.
fn placeholders_of(tree: &Node) -> Vec<Placeholder> {
    tree.children_named("sp")
        .filter_map(|sp| {
            let ph = sp
                .child("nvSpPr")
                .and_then(|nv| nv.child("nvPr"))
                .and_then(|nv| nv.child("ph"))?;
            let idx = ph.attr("idx").and_then(|v| v.parse::<u32>().ok());
            let ph_type = ph.attr("type").map(str::to_owned);
            let frame = sp.child("spPr").and_then(xfrm_of);
            Some(Placeholder {
                idx,
                ph_type,
                frame,
            })
        })
        .collect()
}

/// Стили уровней `p:titleStyle`/`p:bodyStyle`: девять уровней, каждый наследует
/// предыдущий — каскад OOXML.
fn level_styles(style: &Node, theme: &Theme) -> Vec<RunProps> {
    let mut levels: Vec<RunProps> = Vec::new();
    for level in &style.children {
        if !level.name.starts_with("lvl") {
            continue;
        }
        let own = level
            .child("defRPr")
            .map_or_else(RunProps::default, |node| parse_run_props(node, theme));
        let base = levels.last().cloned().unwrap_or_default();
        levels.push(base.merged(Some(&own)));
    }
    levels
}

/// Фон: `p:bg/p:bgPr` → заливка.
fn background_of(dom: &Node) -> Option<Rgb> {
    dom.child("bg")
        .and_then(|bg| bg.child("bgPr"))
        .and_then(fill_of)
}

/// Разобрать тему: цвета `a:clrScheme`, шрифты `a:fontScheme`.
fn parse_theme(dom: &Node) -> Theme {
    let mut theme = Theme::default();
    let Some(elements) = dom.child("themeElements") else {
        return theme;
    };
    if let Some(scheme) = elements.child("clrScheme") {
        for (name, slot) in [
            ("dk1", None::<SchemeColor>),
            ("lt1", None),
            ("dk2", None),
            ("lt2", None),
            ("accent1", Some(SchemeColor::Accent1)),
            ("accent2", Some(SchemeColor::Accent2)),
            ("accent3", Some(SchemeColor::Accent3)),
            ("accent4", Some(SchemeColor::Accent4)),
            ("accent5", Some(SchemeColor::Accent5)),
            ("accent6", Some(SchemeColor::Accent6)),
            ("hlink", Some(SchemeColor::Hyperlink)),
            ("folHlink", Some(SchemeColor::FollowedHyperlink)),
        ] {
            let Some(color) = scheme.child(name).and_then(theme_color) else {
                continue;
            };
            match slot {
                None => {
                    if name == "dk1" {
                        theme.colors.dark1 = color;
                    } else if name == "lt1" {
                        theme.colors.light1 = color;
                    } else if name == "dk2" {
                        theme.colors.dark2 = color;
                    } else if name == "lt2" {
                        theme.colors.light2 = color;
                    }
                }
                Some(scheme_color) => match scheme_color {
                    SchemeColor::Accent1 => theme.colors.accent[0] = color,
                    SchemeColor::Accent2 => theme.colors.accent[1] = color,
                    SchemeColor::Accent3 => theme.colors.accent[2] = color,
                    SchemeColor::Accent4 => theme.colors.accent[3] = color,
                    SchemeColor::Accent5 => theme.colors.accent[4] = color,
                    SchemeColor::Accent6 => theme.colors.accent[5] = color,
                    SchemeColor::Hyperlink => theme.colors.hyperlink = color,
                    SchemeColor::FollowedHyperlink => {
                        theme.colors.followed_hyperlink = color;
                    }
                    _ => {}
                },
            }
        }
    }
    if let Some(fonts) = elements.child("fontScheme") {
        if let Some(major) = fonts
            .child("majorFont")
            .and_then(|node| node.child("latin"))
        {
            if let Some(face) = major.attr("typeface") {
                face.clone_into(&mut theme.major_font);
            }
        }
        if let Some(minor) = fonts
            .child("minorFont")
            .and_then(|node| node.child("latin"))
        {
            if let Some(face) = minor.attr("typeface") {
                face.clone_into(&mut theme.minor_font);
            }
        }
    }
    if theme.major_font.is_empty() {
        DEFAULT_FONT.clone_into(&mut theme.major_font);
    }
    if theme.minor_font.is_empty() {
        DEFAULT_FONT.clone_into(&mut theme.minor_font);
    }
    theme
}

/// Цвет записи схемы: `a:srgbClr`/`a:sysClr`.
fn theme_color(entry: &Node) -> Option<Rgb> {
    if let Some(srgb) = entry.child("srgbClr") {
        if let Some(color) = srgb.attr("val").and_then(Rgb::from_hex) {
            return Some(color);
        }
    }
    entry
        .child("sysClr")
        .and_then(|sys| sys.attr("lastClr"))
        .and_then(Rgb::from_hex)
}

// ── Слайд ───────────────────────────────────────────────────────────────────

/// Разобрать слайд: фон, имя, шейпы.
#[allow(clippy::too_many_arguments)]
fn parse_slide(
    index: usize,
    dom: &Node,
    slide_rels: &RelMap,
    part: &str,
    archive: &mut Archive,
    layout: &LayoutInfo,
    master: &MasterInfo,
    ctx: &mut Ctx<'_>,
) -> Slide {
    let dom = doc_element(dom, "sld");
    let csld = dom.child("cSld");
    let name = csld
        .and_then(|node| node.attr("name"))
        .filter(|name| !name.is_empty())
        .map_or_else(|| format!("Slide {}", index + 1), str::to_owned);

    let mut background = csld.and_then(background_of);
    if background.is_none() {
        background = layout.background;
    }
    if background.is_none() {
        background = master.background;
    }

    let mut shapes = Vec::new();
    if let Some(tree) = csld.and_then(|node| node.child("spTree")) {
        let mut groups: Vec<GroupTransform> = Vec::new();
        parse_shape_tree(
            tree,
            &mut groups,
            slide_rels,
            part,
            archive,
            layout,
            master,
            ctx,
            &mut shapes,
        );
    }

    Slide {
        name,
        background,
        shapes,
    }
}

/// Трансформация группы (`p:grpSpPr/a:xfrm`): перенос и масштаб детей.
#[derive(Debug, Clone, Copy, Default)]
struct GroupTransform {
    off_x: Emu,
    off_y: Emu,
    ext_cx: Emu,
    ext_cy: Emu,
    ch_off_x: Emu,
    ch_off_y: Emu,
    ch_ext_cx: Emu,
    ch_ext_cy: Emu,
}

impl GroupTransform {
    /// Абсолютный прямоугольник ребёнка в координатах родителя.
    ///
    /// Координаты ребёнка отсчитываются от `chOff`, масштабируются в
    /// `ext`/`chExt` и сдвигаются на `off`. Без `chExt`/`ext` — единичный
    /// масштаб (только перенос).
    #[allow(clippy::cast_precision_loss)]
    fn apply(self, frame: Frame) -> Frame {
        let scale_x = if self.ch_ext_cx > 0 && self.ext_cx > 0 {
            self.ext_cx as f64 / self.ch_ext_cx as f64
        } else {
            1.0
        };
        let scale_y = if self.ch_ext_cy > 0 && self.ext_cy > 0 {
            self.ext_cy as f64 / self.ch_ext_cy as f64
        } else {
            1.0
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        {
            Frame {
                x: self.off_x + ((frame.x - self.ch_off_x) as f64 * scale_x) as Emu,
                y: self.off_y + ((frame.y - self.ch_off_y) as f64 * scale_y) as Emu,
                w: (frame.w as f64 * scale_x) as Emu,
                h: (frame.h as f64 * scale_y) as Emu,
            }
        }
    }
}

/// Применить цепочку трансформаций групп (от внутренней к внешней).
fn apply_groups(frame: Frame, groups: &[GroupTransform]) -> Frame {
    groups
        .iter()
        .rev()
        .fold(frame, |frame, group| group.apply(frame))
}

/// Рекурсивный разбор дерева шейпов, включая группы.
#[allow(clippy::too_many_arguments)]
fn parse_shape_tree(
    tree: &Node,
    groups: &mut Vec<GroupTransform>,
    slide_rels: &RelMap,
    part: &str,
    archive: &mut Archive,
    layout: &LayoutInfo,
    master: &MasterInfo,
    ctx: &mut Ctx<'_>,
    out: &mut Vec<Shape>,
) {
    for child in &tree.children {
        match child.name.as_str() {
            "sp" => {
                if let Some(shape) = parse_sp(child, groups, layout, master, ctx) {
                    out.push(shape);
                }
            }
            "pic" => {
                if let Some(shape) = parse_pic(child, groups, slide_rels, part, archive, ctx) {
                    out.push(shape);
                }
            }
            "graphicFrame" => {
                if let Some(shape) = parse_graphic_frame(child, groups, slide_rels, part, archive) {
                    out.push(shape);
                }
            }
            "grpSp" => {
                if let Some(transform) = group_transform(child) {
                    groups.push(transform);
                }
                parse_shape_tree(
                    child, groups, slide_rels, part, archive, layout, master, ctx, out,
                );
                if group_transform(child).is_some() {
                    groups.pop();
                }
            }
            _ => {}
        }
    }
}

/// Трансформация группы из `p:grpSpPr/a:xfrm`.
fn group_transform(group: &Node) -> Option<GroupTransform> {
    let xfrm = group.child("grpSpPr")?.child("xfrm")?;
    let off = xfrm.child("off")?;
    let ext = xfrm.child("ext")?;
    let ch_off = xfrm.child("chOff").unwrap_or(off);
    let ch_ext = xfrm.child("chExt").unwrap_or(ext);
    Some(GroupTransform {
        off_x: parse_emu(off.attr("x")).unwrap_or(0),
        off_y: parse_emu(off.attr("y")).unwrap_or(0),
        ext_cx: parse_emu(ext.attr("cx")).unwrap_or(0),
        ext_cy: parse_emu(ext.attr("cy")).unwrap_or(0),
        ch_off_x: parse_emu(ch_off.attr("x")).unwrap_or(0),
        ch_off_y: parse_emu(ch_off.attr("y")).unwrap_or(0),
        ch_ext_cx: parse_emu(ch_ext.attr("cx")).unwrap_or(0),
        ch_ext_cy: parse_emu(ch_ext.attr("cy")).unwrap_or(0),
    })
}

/// Шейп `p:sp`: имя, геометрия, заливка, контур, текст.
fn parse_sp(
    node: &Node,
    groups: &[GroupTransform],
    layout: &LayoutInfo,
    master: &MasterInfo,
    ctx: &Ctx<'_>,
) -> Option<Shape> {
    let nv = node.child("nvSpPr")?;
    let name = nv
        .child("cNvPr")
        .and_then(|n| n.attr("name"))
        .filter(|name| !name.is_empty())
        .unwrap_or("shape")
        .to_owned();

    let ph = nv.child("nvPr").and_then(|n| n.child("ph"));
    let ph_idx = ph
        .and_then(|p| p.attr("idx"))
        .and_then(|v| v.parse::<u32>().ok());
    let ph_type = ph.and_then(|p| p.attr("type")).map(str::to_owned);

    // Шейп может жить без `p:spPr`: вся геометрия — в плейсхолдере layout'а.
    let sp_pr = node.child("spPr");
    let local_frame = sp_pr.and_then(xfrm_of);
    let frame =
        local_frame.or_else(|| resolve_placeholder(ph_idx, ph_type.as_deref(), layout, master))?;
    let frame = apply_groups(frame, groups);

    let fill = sp_pr
        .and_then(|pr| pr.child("solidFill"))
        .and_then(|f| fill_with_theme(f, ctx.theme));
    let line = sp_pr
        .and_then(|pr| pr.child("ln"))
        .map(|ln| parse_line(ln, ctx.theme));

    // Стиль уровня: заголовки — по типу плейсхолдера.
    let level_styles = match ph_type.as_deref() {
        Some("title" | "ctrTitle") => &master.title_style,
        Some("obj" | "dt") => &master.other_style,
        _ => &master.body_style,
    };

    let kind = match node.child("txBody") {
        Some(body) => ShapeKind::Text(parse_text_body(body, ctx.theme, level_styles)),
        None => ShapeKind::Geometry(geometry_of(sp_pr)),
    };

    Some(Shape {
        name,
        frame,
        fill,
        line,
        kind,
    })
}

/// Геометрия шейпа из `p:nvSpPr`…`p:spPr`; плейсхолдер без своей геометрии
/// наследует её из layout-слайда, тот — из мастера.
fn resolve_placeholder(
    idx: Option<u32>,
    ph_type: Option<&str>,
    layout: &LayoutInfo,
    master: &MasterInfo,
) -> Option<Frame> {
    let find = |placeholders: &[Placeholder]| -> Option<Frame> {
        placeholders
            .iter()
            .find(|ph| idx.is_some_and(|i| ph.idx == Some(i)))
            .or_else(|| {
                placeholders
                    .iter()
                    .find(|ph| ph_type.is_some_and(|t| ph.ph_type.as_deref() == Some(t)))
            })
            .and_then(|ph| ph.frame)
    };
    find(&layout.placeholders).or_else(|| find(&master.placeholders))
}

/// Прямоугольник из `a:xfrm`: `a:off` + `a:ext`.
fn xfrm_of(sp_pr: &Node) -> Option<Frame> {
    let xfrm = sp_pr.child("xfrm")?;
    let off = xfrm.child("off")?;
    let ext = xfrm.child("ext")?;
    Some(Frame {
        x: parse_emu(off.attr("x")).unwrap_or(0),
        y: parse_emu(off.attr("y")).unwrap_or(0),
        w: parse_emu(ext.attr("cx")).unwrap_or(0),
        h: parse_emu(ext.attr("cy")).unwrap_or(0),
    })
}

/// Геометрия из `a:prstGeom prst`; без `p:spPr` — прямоугольник.
fn geometry_of(sp_pr: Option<&Node>) -> GeometryPreset {
    match sp_pr
        .and_then(|pr| pr.child("prstGeom"))
        .and_then(|g| g.attr("prst"))
    {
        Some("roundRect") => GeometryPreset::RoundRect,
        Some("ellipse") => GeometryPreset::Ellipse,
        _ => GeometryPreset::Rect,
    }
}

/// Контур из `a:ln`: цвет заливки и толщина `w`.
fn parse_line(ln: &Node, theme: &Theme) -> Line {
    Line {
        color: ln.child("solidFill").and_then(fill_with_theme_for(theme)),
        width: parse_emu(ln.attr("w")),
    }
}

/// `fill_with_theme`, прижатый к заимствованию темы в `parse_line`.
fn fill_with_theme_for(theme: &Theme) -> impl Fn(&Node) -> Option<Rgb> + '_ {
    |fill: &Node| fill_with_theme(fill, theme)
}

/// Заливка из `a:solidFill`: `a:srgbClr` или `a:schemeClr`.
fn fill_of(fill: &Node) -> Option<Rgb> {
    if let Some(srgb) = fill.child("srgbClr") {
        if let Some(color) = srgb.attr("val").and_then(Rgb::from_hex) {
            return Some(color);
        }
    }
    None
}

/// Заливка с резолвом цвета схемы через тему.
fn fill_with_theme(fill: &Node, theme: &Theme) -> Option<Rgb> {
    if let Some(color) = fill_of(fill) {
        return Some(color);
    }
    fill.child("schemeClr")
        .and_then(|scheme| scheme.attr("val"))
        .and_then(SchemeColor::from_val)
        .map(|color| theme.colors.resolve(color))
}

/// Шейп `p:pic`: картинка по `a:blip r:embed`.
fn parse_pic(
    node: &Node,
    groups: &[GroupTransform],
    slide_rels: &RelMap,
    part: &str,
    archive: &mut Archive,
    ctx: &mut Ctx<'_>,
) -> Option<Shape> {
    let name = node
        .child("nvPicPr")
        .and_then(|nv| nv.child("cNvPr"))
        .and_then(|nv| nv.attr("name"))
        .filter(|name| !name.is_empty())
        .unwrap_or("picture")
        .to_owned();
    let frame = node.child("spPr").and_then(xfrm_of)?;
    let frame = apply_groups(frame, groups);

    let rid = node.child("blipFill")?.child("blip")?.attr("r:embed")?;
    let image_id = resolve_image(rid, slide_rels, part, archive, ctx);

    Some(Shape {
        name,
        frame,
        fill: None,
        line: None,
        kind: ShapeKind::Picture { image_id },
    })
}

/// `p:graphicFrame`: диаграмма `c:chart` по `r:id`.
fn parse_graphic_frame(
    node: &Node,
    groups: &[GroupTransform],
    slide_rels: &RelMap,
    part: &str,
    archive: &mut Archive,
) -> Option<Shape> {
    let name = node
        .child("nvGraphicFramePr")
        .and_then(|nv| nv.child("cNvPr"))
        .and_then(|nv| nv.attr("name"))
        .filter(|name| !name.is_empty())
        .unwrap_or("chart")
        .to_owned();
    let frame = xfrm_of(node)?;
    let frame = apply_groups(frame, groups);

    let rid = node
        .child("graphic")?
        .child("graphicData")?
        .child("chart")?
        .attr("r:id")?;
    let rel = slide_rels.get(rid)?;
    let chart_part = rel.part(part)?;
    let xml = archive.read(&chart_part).ok()?;
    let dom = parse_dom(&xml, &chart_part).ok()?;
    let data = parse_chart(&dom)?;

    Some(Shape {
        name,
        frame,
        fill: None,
        line: None,
        kind: ShapeKind::Chart(data),
    })
}

/// Зарегистрировать media-часть и вернуть её id; уже известная часть — тот же id.
fn resolve_image(
    rid: &str,
    slide_rels: &RelMap,
    part: &str,
    archive: &mut Archive,
    ctx: &mut Ctx<'_>,
) -> Option<u32> {
    let rel = slide_rels.get(rid)?;
    let target = rel.part(part)?;
    if let Some(&id) = ctx.image_by_part.get(&target) {
        return Some(id);
    }
    let bytes = match archive.read(&target) {
        Ok(bytes) => bytes,
        Err(error) => {
            ctx.warn(format!("image {target}: {error}"));
            return None;
        }
    };
    let id = u32::try_from(ctx.images.len()).ok()?;
    let mime = mime_for(&target);
    ctx.images.push(ImagePart {
        id,
        name: target.clone(),
        mime,
        bytes,
    });
    ctx.image_by_part.insert(target, id);
    Some(id)
}

/// MIME-тип по расширению части; неизвестное — octet-stream.
fn mime_for(part: &str) -> String {
    let extension = part.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    };
    mime.to_owned()
}

// ── Текст ───────────────────────────────────────────────────────────────────

/// Текстовая рамка: отступы, якорь, перенос, абзацы.
fn parse_text_body(body: &Node, theme: &Theme, level_styles: &[RunProps]) -> TextBox {
    let body_pr = body.child("bodyPr");
    let insets = body_pr.map_or_else(Insets::default, |pr| Insets {
        left: parse_emu(pr.attr("lIns")).unwrap_or(Insets::default().left),
        top: parse_emu(pr.attr("tIns")).unwrap_or(Insets::default().top),
        right: parse_emu(pr.attr("rIns")).unwrap_or(Insets::default().right),
        bottom: parse_emu(pr.attr("bIns")).unwrap_or(Insets::default().bottom),
    });
    let anchor = match body_pr.and_then(|pr| pr.attr("anchor")) {
        Some("ctr") => Anchor::Center,
        Some("b") => Anchor::Bottom,
        _ => Anchor::Top,
    };
    let wrap = body_pr
        .and_then(|pr| pr.attr("wrap"))
        .is_none_or(|value| value != "none");

    let paragraphs = body
        .children_named("p")
        .map(|p| parse_paragraph(p, theme, level_styles))
        .collect();

    TextBox {
        body: TextBody {
            insets,
            anchor,
            wrap,
            paragraphs,
        },
    }
}

/// Абзац: свойства, маркер, прогоны с разрешённым каскадом.
fn parse_paragraph(p: &Node, theme: &Theme, level_styles: &[RunProps]) -> Paragraph {
    let p_pr = p.child("pPr");
    let align = match p_pr.and_then(|pr| pr.attr("algn")) {
        Some("ctr") => Align::Center,
        Some("r") => Align::Right,
        _ => Align::Left,
    };
    let level = p_pr
        .and_then(|pr| pr.attr("lvl"))
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(0)
        .min(8);

    let bullet = p_pr.and_then(|pr| {
        if pr.child("buNone").is_some() {
            return None;
        }
        if let Some(bu_char) = pr.child("buChar") {
            return bu_char
                .attr("char")
                .and_then(|c| c.chars().next())
                .map(Bullet::Char);
        }
        if let Some(auto) = pr.child("buAutoNum") {
            let start = auto
                .attr("startAt")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(1);
            return Some(Bullet::AutoNum { start });
        }
        None
    });

    // Каскад: стиль уровня → defRPr абзаца.
    let level_default = level_styles
        .get(level as usize)
        .cloned()
        .unwrap_or_default();
    let default = level_default.merged(
        p_pr.and_then(|pr| pr.child("defRPr"))
            .map(|node| parse_run_props(node, theme))
            .as_ref(),
    );

    let mut runs = Vec::new();
    for run in p.children_named("r") {
        let props = default
            .merged(
                run.child("rPr")
                    .map(|node| parse_run_props(node, theme))
                    .as_ref(),
            )
            .with_default_size(DEFAULT_SIZE_PT);
        let text = run_text(run);
        if text.is_empty() && !props.bold && !props.italic && !props.underline {
            continue;
        }
        runs.push(Run { text, props });
    }
    Paragraph {
        level,
        align,
        bullet,
        runs,
    }
}

/// Свойства знака из `a:rPr`/`a:defRPr`.
fn parse_run_props(node: &Node, theme: &Theme) -> RunProps {
    RunProps {
        // `sz` — сотые пункта.
        size: node
            .attr("sz")
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| v / 100.0),
        bold: node.attr("b").is_some_and(|v| v == "1" || v == "true"),
        italic: node.attr("i").is_some_and(|v| v == "1" || v == "true"),
        underline: node
            .attr("u")
            .is_some_and(|v| v == "sng" || v == "1" || v == "true"),
        color: node
            .child("solidFill")
            .and_then(|f| fill_with_theme(f, theme)),
        font: node
            .child("latin")
            .and_then(|latin| latin.attr("typeface"))
            .map(str::to_owned),
    }
}

/// Текст прогона: `a:t` подряд, `a:br` — перевод строки, `a:fld` — поле.
fn run_text(run: &Node) -> String {
    let mut out = String::new();
    for child in &run.children {
        match child.name.as_str() {
            "t" => out.push_str(&child.text),
            "br" => out.push('\n'),
            "fld" => out.push_str(&child.text_all()),
            _ => {}
        }
    }
    out
}

// ── Диаграммы ───────────────────────────────────────────────────────────────

/// Разобрать `ppt/charts/chartN.xml` в общую модель ADR-0011.
fn parse_chart(dom: &Node) -> Option<ChartData> {
    let chart = doc_element(dom, "chartSpace").child("chart")?;
    let plot = chart.child("plotArea")?;

    let (kind, type_node) = [
        ("barChart", ChartKind::Bar),
        ("lineChart", ChartKind::Line),
        ("pieChart", ChartKind::Pie),
        ("scatterChart", ChartKind::Scatter),
        ("areaChart", ChartKind::Area),
    ]
    .into_iter()
    .find_map(|(name, kind)| plot.child(name).map(|node| (kind, node)))?;

    let title = chart.child("title").map(texts_of).and_then(|texts| {
        texts
            .into_iter()
            .map(|text| text.trim().to_owned())
            .find(|text| !text.is_empty())
    });

    let mut series = Vec::new();
    for (index, ser) in type_node.children_named("ser").enumerate() {
        let name = ser
            .child("tx")
            .map(texts_of)
            .and_then(|texts| {
                texts
                    .into_iter()
                    .map(|text| text.trim().to_owned())
                    .find(|text| !text.is_empty())
            })
            .unwrap_or_else(|| format!("Series {}", index + 1));
        let values = ser.child("val").map(values_of).unwrap_or_default();
        if values.is_empty() {
            continue;
        }
        series.push(ChartSeries { name, values });
    }
    if series.is_empty() {
        return None;
    }

    // Категории — из первой серии; без них — индексы.
    let categories = type_node
        .children_named("ser")
        .next()
        .and_then(|ser| ser.child("cat"))
        .map(category_values)
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| {
            let count = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
            (1..=count).map(|i| i.to_string()).collect()
        });

    Some(ChartData {
        kind,
        title,
        categories,
        series,
    })
}

/// Все тексты `a:t`/`c:v` поддерева — для заголовков и имён серий.
fn texts_of(node: &Node) -> Vec<String> {
    let mut out = Vec::new();
    collect_texts(node, &mut out);
    out
}

fn collect_texts(node: &Node, out: &mut Vec<String>) {
    if matches!(node.name.as_str(), "t" | "v") && !node.text.trim().is_empty() {
        out.push(node.text.clone());
    }
    for child in &node.children {
        collect_texts(child, out);
    }
}

/// Числа серии: все `c:v` поддерева, битые значения пропускаются.
fn values_of(node: &Node) -> Vec<f32> {
    texts_of(node)
        .into_iter()
        .filter_map(|text| text.trim().parse::<f32>().ok())
        .collect()
}

/// Подписи категорий: `c:v` поддерева; числа остаются строками как есть.
fn category_values(node: &Node) -> Vec<String> {
    texts_of(node)
        .into_iter()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect()
}
