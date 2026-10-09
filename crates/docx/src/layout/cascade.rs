//! Каскад стилей: сырые `RawPPr`/`RawRPr` → значения, готовые раскладке (ADR-0013).
//!
//! Порядок применения — от слабого к сильному:
//!
//! - `pPr`: `docDefaults` → цепочка `basedOn` абзацного стиля (от корня к листу) →
//!   прямое форматирование абзаца;
//! - `rPr`: `docDefaults` → `rPr` абзацного стиля (та же цепочка) → цепочка `basedOn`
//!   знакового стиля → прямое форматирование run'а.
//!
//! Каскад резолвится на раскладке, а не на разборе (ADR-0013 §1), поэтому и кэш живёт не в
//! модели: [`StyleCache`] заводит вызывающий и передаёт его в [`resolve_paragraph`]
//! и [`resolve_run`] на один вызов раскладки документа.
//!
//! # Вне объёма
//!
//! - свойства уровней нумерации: `w:numPr` переносится ссылкой, но `w:lvl` не применяется;
//! - стиль таблицы и её условные форматы (`w:tblStylePr`, `w:tblLook`): раскладка передаст
//!   контекст ячейки, когда он появится;
//! - `w:rPr` внутри `w:pPr` (mark run properties) разрешается в
//!   [`ResolvedParagraph::mark_rpr`], но к runs не применяется — в Word он принадлежит
//!   знаку абзаца (ADR-0013 §2);
//! - `w:link` и `w:next` в каскаде не участвуют (ADR-0013 §3).

use std::collections::{BTreeMap, BTreeSet};

use crate::layout::engine::twips_to_px;
use crate::layout::line_break::LineRule;
use crate::model::{
    CharacterStyle, Color, Document, HalfPoint, Highlight, Ind, Justification, LineSpacing,
    LineSpacingRule, NumPr, Paragraph, ParagraphBorders, ParagraphSpacing, RawPPr, RawRPr, Run,
    Shading, StyleId, Toggle, Twips, Underline, VertAlign,
};

/// Предел глубины цепочки `basedOn` — как в `model::validate` (ADR-0016 §2).
///
/// Файл с более длинной цепочкой уже получил предупреждение на разборе, но раскладка
/// обходить её не должна: цикл `a → b → a` предупреждениями не чинится.
const MAX_BASED_ON_DEPTH: usize = 32;

/// Кегль, когда `w:sz` не задан нигде, — 12 pt: столько же подставляет раскладка,
/// которой каскад ещё не передан.
const DEFAULT_SIZE_HALF_POINTS: i32 = 24;

/// Отступы абзаца в пикселях раскладки.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResolvedInd {
    /// Отступ слева.
    pub left_px: f32,
    /// Отступ справа.
    pub right_px: f32,
    /// Отступ первой строки: отрицательный — выступ.
    pub first_line_px: f32,
}

impl ResolvedInd {
    /// Накладывает слой `w:ind`.
    ///
    /// Поля внутри слоя независимы: незаданное поле оставляет значение предыдущего слоя.
    /// `w:hanging` — это отрицательный `w:firstLine`, и если в одном слое заданы оба,
    /// побеждает `hanging`: так же читает файл Word (ECMA-376 §17.3.1.12).
    fn apply(&mut self, ind: &Ind) {
        if let Some(left) = ind.left {
            self.left_px = twips_to_px(left);
        }
        if let Some(right) = ind.right {
            self.right_px = twips_to_px(right);
        }
        if let Some(first_line) = ind.first_line {
            self.first_line_px = twips_to_px(first_line);
        }
        if let Some(hanging) = ind.hanging {
            self.first_line_px = -twips_to_px(hanging);
        }
    }
}

/// Интервалы абзаца в пикселях и множителях.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResolvedSpacing {
    /// Интервал перед абзацем.
    pub before_px: f32,
    /// Интервал после абзаца.
    pub after_px: f32,
    /// Правило высоты строки.
    pub line: LineRule,
}

impl ResolvedSpacing {
    /// Накладывает слой `w:spacing`.
    ///
    /// `w:beforeLines`/`w:afterLines` и автоинтервалы не учитываются: раскладке нужны
    /// пиксели, а эти атрибуты описывают интервал в строках (восточноазиатская вёрстка)
    /// и подстановку самим приложением.
    fn apply(&mut self, spacing: &ParagraphSpacing) {
        if let Some(before) = spacing.before {
            self.before_px = twips_to_px(before);
        }
        if let Some(after) = spacing.after {
            self.after_px = twips_to_px(after);
        }
        if let Some(line) = spacing.line {
            self.line = resolve_line_rule(line, spacing.line_rule.as_ref());
        }
    }
}

/// Правило высоты строки из `w:line` и `w:lineRule`.
///
/// Множитель — единственное, что раскладка может применить, зная лишь метрику шрифта;
/// абсолютные правила уже переведены в пиксели.
// Незнакомое значение `w:lineRule` (`LineSpacingRule::Other`) читается как `auto`:
// документ с опечаткой в правиле лучше разложить по множителю, чем потерять интервал.
fn resolve_line_rule(line: LineSpacing, rule: Option<&LineSpacingRule>) -> LineRule {
    match rule {
        Some(LineSpacingRule::Exact) => LineRule::Exact(abs_twips_to_px(line)),
        Some(LineSpacingRule::AtLeast) => LineRule::AtLeast(abs_twips_to_px(line)),
        Some(LineSpacingRule::Auto | LineSpacingRule::Other(_)) | None => {
            LineRule::Auto(line_multiplier(line))
        }
    }
}

/// `w:line` при абсолютном правиле — twips.
fn abs_twips_to_px(line: LineSpacing) -> f32 {
    twips_to_px(Twips::new(line.value()))
}

/// `w:line` при `lineRule="auto"` — 240-е доли строки.
#[allow(clippy::cast_precision_loss)]
fn line_multiplier(line: LineSpacing) -> f32 {
    line.value() as f32 / 240.0
}

/// Свойства абзаца после каскада.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPPr {
    /// Выравнивание (`w:jc`); `None` — не задано нигде.
    pub jc: Option<Justification>,
    /// Отступы (`w:ind`).
    pub ind: ResolvedInd,
    /// Интервалы (`w:spacing`).
    pub spacing: ResolvedSpacing,
    /// Не отрывать абзац от следующего (`w:keepNext`).
    pub keep_next: bool,
    /// Не разрывать абзац между страницами (`w:keepLines`).
    pub keep_lines: bool,
    /// Начинать абзац с новой страницы (`w:pageBreakBefore`).
    pub page_break_before: bool,
    /// Не оставлять одну строку абзаца на странице (`w:widowControl`).
    pub widow_control: bool,
    /// Уровень структуры документа 0..=9 (`w:outlineLvl`).
    pub outline_lvl: Option<u8>,
    /// Ссылка на нумерацию (`w:numPr`) — только идентификаторы: свойства уровня
    /// нумерации в каскад не входят (см. «Вне объёма» в доккомментарии модуля).
    pub num_pr: Option<NumPr>,
    /// Заливка абзаца (`w:shd`).
    pub shd: Option<Shading>,
    /// Границы абзаца (`w:pBdr`).
    pub p_bdr: Option<ParagraphBorders>,
}

impl Default for ResolvedPPr {
    fn default() -> Self {
        Self {
            jc: None,
            ind: ResolvedInd::default(),
            spacing: ResolvedSpacing::default(),
            keep_next: false,
            keep_lines: false,
            page_break_before: false,
            // Единственный тумблер абзаца, включённый в Word по умолчанию: абзац без
            // единого `w:widowControl` висячие строки всё равно не оставляет.
            widow_control: true,
            outline_lvl: None,
            num_pr: None,
            shd: None,
            p_bdr: None,
        }
    }
}

impl ResolvedPPr {
    /// Накладывает слой `w:pPr`: заданное перекрывает предыдущее, незаданное не трогает.
    fn apply(&mut self, ppr: &RawPPr) {
        if let Some(jc) = &ppr.jc {
            self.jc = Some(jc.clone());
        }
        if let Some(ind) = &ppr.ind {
            self.ind.apply(ind);
        }
        if let Some(spacing) = &ppr.spacing {
            self.spacing.apply(spacing);
        }
        apply_toggle(&mut self.keep_next, ppr.keep_next);
        apply_toggle(&mut self.keep_lines, ppr.keep_lines);
        apply_toggle(&mut self.page_break_before, ppr.page_break_before);
        apply_toggle(&mut self.widow_control, ppr.widow_control);
        if let Some(level) = ppr.outline_lvl {
            self.outline_lvl = Some(level);
        }
        if let Some(num_pr) = &ppr.num_pr {
            // Поля `w:numPr` сливаются по отдельности: стиль задаёт `w:numId`, а прямое
            // форматирование — только `w:ilvl`.
            let slot = self.num_pr.get_or_insert_with(NumPr::default);
            slot.ilvl = num_pr.ilvl.or(slot.ilvl);
            slot.num_id = num_pr.num_id.or(slot.num_id);
        }
        if let Some(shd) = &ppr.shd {
            self.shd = Some(shd.clone());
        }
        if let Some(p_bdr) = &ppr.p_bdr {
            self.p_bdr = Some(p_bdr.clone());
        }
    }
}

/// Свойства знака после каскада.
///
/// Тумблеры `w:dstrike`, `w:vanish`, `w:outline`, `w:shadow`, `w:emboss`, `w:imprint`,
/// межбуквенный интервал `w:spacing` и `w:position` сюда не переносятся: их не несёт ни
/// `DisplayList`, ни раскладка.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRPr {
    /// Гарнитура (`w:rFonts w:ascii`, при его отсутствии — `w:hAnsi`).
    pub font_family: Option<String>,
    /// Кегль в полупунктах (`w:sz`).
    pub size: HalfPoint,
    /// Полужирный (`w:b`).
    pub bold: bool,
    /// Курсив (`w:i`).
    pub italic: bool,
    /// Все прописные (`w:caps`).
    pub all_caps: bool,
    /// Капитель (`w:smallCaps`).
    pub small_caps: bool,
    /// Зачёркнутый (`w:strike`).
    pub strike: bool,
    /// Подчёркивание (`w:u`); `Some(Underline::None)` — снято явно.
    pub underline: Option<Underline>,
    /// Цвет текста `RRGGBB` (`w:color`); `w:val="auto"` и `"none"` — `None`.
    pub color: Option<u32>,
    /// Вертикальное смещение (`w:vertAlign`).
    pub vert_align: Option<VertAlign>,
    /// Выделение цветом (`w:highlight`).
    pub highlight: Option<Highlight>,
}

impl Default for ResolvedRPr {
    fn default() -> Self {
        Self {
            font_family: None,
            size: HalfPoint::new(DEFAULT_SIZE_HALF_POINTS),
            bold: false,
            italic: false,
            all_caps: false,
            small_caps: false,
            strike: false,
            underline: None,
            color: None,
            vert_align: None,
            highlight: None,
        }
    }
}

impl ResolvedRPr {
    /// Накладывает слой `w:rPr`: заданное перекрывает предыдущее, незаданное не трогает.
    fn apply(&mut self, rpr: &RawRPr) {
        if let Some(fonts) = &rpr.r_fonts {
            // Внутри слоя побеждает `w:ascii`: это гарнитура латиницы, а `w:hAnsi` — запасная.
            if let Some(h_ansi) = &fonts.h_ansi {
                self.font_family = Some(h_ansi.clone());
            }
            if let Some(ascii) = &fonts.ascii {
                self.font_family = Some(ascii.clone());
            }
        }
        if let Some(size) = rpr.sz {
            self.size = size;
        }
        apply_toggle(&mut self.bold, rpr.b);
        apply_toggle(&mut self.italic, rpr.i);
        apply_toggle(&mut self.all_caps, rpr.caps);
        apply_toggle(&mut self.small_caps, rpr.small_caps);
        apply_toggle(&mut self.strike, rpr.strike);
        if let Some(underline) = &rpr.u {
            self.underline = Some(underline.clone());
        }
        if let Some(color) = rpr.color {
            // `auto` раскладке нечего показать — цвет выбирает контекст, поэтому слой
            // с `auto` снимает цвет предыдущего слоя, а не оставляет его.
            self.color = match color {
                Color::Rgb(value) => Some(value),
                Color::Auto | Color::None => None,
            };
        }
        if let Some(vert_align) = rpr.vert_align {
            self.vert_align = Some(vert_align);
        }
        if let Some(highlight) = &rpr.highlight {
            self.highlight = Some(highlight.clone());
        }
    }
}

/// Абзац после каскада.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedParagraph {
    /// Свойства абзаца.
    pub ppr: ResolvedPPr,
    /// Свойства знака, которые наследуют runs: `docDefaults` плюс `rPr` цепочки абзацного стиля.
    pub rpr: ResolvedRPr,
    /// Свойства знака абзаца: [`Self::rpr`] плюс `mark_rpr`. К runs не применяются (ADR-0013 §2).
    pub mark_rpr: ResolvedRPr,
}

/// Кэш резолвинга на один вызов раскладки документа.
///
/// Глобального кэша нет намеренно: он дал бы блокировку и недетерминизм между документами,
/// а повторный резолвинг заведомо есть — у каждого абзаца свой стиль, у каждого run'а свой.
#[derive(Debug, Default)]
pub struct StyleCache {
    /// `docDefaults`, разложенные один раз.
    doc_defaults: Option<(ResolvedPPr, ResolvedRPr)>,
    /// Абзацный стиль, слитый со всей цепочкой `basedOn` и `docDefaults`.
    paragraph: BTreeMap<StyleId, (ResolvedPPr, ResolvedRPr)>,
    /// Порядок применения цепочки `basedOn` знакового стиля, от корня к листу.
    ///
    /// Цепочка, а не слитые значения: незаданное свойство знакового стиля обязано оставить
    /// в силе `rPr` абзацного стиля, а слитое значение эту границу уже потеряло.
    character: BTreeMap<StyleId, Vec<StyleId>>,
}

impl StyleCache {
    /// Пустой кэш.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `docDefaults` в виде готовых значений — низ каскада.
    fn defaults(&mut self, document: &Document) -> &(ResolvedPPr, ResolvedRPr) {
        self.doc_defaults.get_or_insert_with(|| {
            let mut ppr = ResolvedPPr::default();
            ppr.apply(&document.styles.doc_defaults.p_pr);
            let mut rpr = ResolvedRPr::default();
            rpr.apply(&document.styles.doc_defaults.r_pr);
            (ppr, rpr)
        })
    }
}

/// Резолвит свойства абзаца: `docDefaults` → цепочка `basedOn` абзацного стиля → `w:pPr`.
#[must_use]
pub fn resolve_paragraph(
    document: &Document,
    paragraph: &Paragraph,
    cache: &mut StyleCache,
) -> ResolvedParagraph {
    // `style_ref` — копия `w:pStyle` (модель дублирует её для раскладки); берём её, но
    // абзац, собранный не парсером, может заполнить только `ppr.style`.
    let style = paragraph
        .style_ref
        .as_ref()
        .or(paragraph.ppr.style.as_ref());
    let (mut ppr, rpr) = match style {
        Some(id) => paragraph_base(document, id, cache),
        None => cache.defaults(document).clone(),
    };
    ppr.apply(&paragraph.ppr);

    let mut mark_rpr = rpr.clone();
    mark_rpr.apply(&paragraph.mark_rpr);
    ResolvedParagraph { ppr, rpr, mark_rpr }
}

/// Резолвит свойства знака run'а: [`ResolvedParagraph::rpr`] → цепочка `basedOn` знакового
/// стиля → `w:rPr` run'а.
///
/// `ResolvedParagraph::mark_rpr` в каскад не входит: свойства знака абзаца к runs не
/// применяются (ADR-0013 §2).
#[must_use]
pub fn resolve_run(
    document: &Document,
    paragraph: &ResolvedParagraph,
    run: &Run,
    cache: &mut StyleCache,
) -> ResolvedRPr {
    let mut rpr = paragraph.rpr.clone();
    let style = run.style_ref.as_ref().or(run.rpr.style.as_ref());
    if let Some(id) = style {
        for style_id in character_chain(document, id, cache) {
            if let Some(style) = document.styles.character.get(style_id) {
                rpr.apply(&style.rpr);
            }
        }
    }
    rpr.apply(&run.rpr);
    rpr
}

/// `docDefaults` плюс цепочка `basedOn` абзацного стиля `id`; результат кэшируется.
fn paragraph_base(
    document: &Document,
    id: &StyleId,
    cache: &mut StyleCache,
) -> (ResolvedPPr, ResolvedRPr) {
    if let Some(base) = cache.paragraph.get(id) {
        return base.clone();
    }

    let (mut ppr, mut rpr) = cache.defaults(document).clone();
    for style_id in based_on_chain(id, |style_id| {
        document
            .styles
            .paragraph
            .get(style_id)
            .map(|style| style.based_on.clone())
    }) {
        if let Some(style) = document.styles.paragraph.get(&style_id) {
            ppr.apply(&style.ppr);
            rpr.apply(&style.rpr);
        }
    }

    cache
        .paragraph
        .insert(id.clone(), (ppr.clone(), rpr.clone()));
    (ppr, rpr)
}

/// Цепочка `basedOn` знакового стиля `id`, от корня к листу; результат кэшируется.
fn character_chain<'a>(
    document: &Document,
    id: &StyleId,
    cache: &'a mut StyleCache,
) -> &'a [StyleId] {
    cache.character.entry(id.clone()).or_insert_with(|| {
        based_on_chain(id, |style_id| {
            document
                .styles
                .character
                .get(style_id)
                .map(|style: &CharacterStyle| style.based_on.clone())
        })
    })
}

/// Идентификаторы цепочки `basedOn` от корня к листу; последний — `id`.
///
/// `parent` отдаёт `basedOn` стиля: `None` — стиля в таблице нет, `Some(None)` — цепочка
/// кончилась. И то и другое не ошибка: парсер уже предупредил о пропавшем стиле
/// (ADR-0013 §3), раскладке остаётся обойти то, что есть.
///
/// Множество посещённых и предел глубины защищают от цикла: `a → b → a` на разборе лишь
/// предупреждение (ADR-0016 §2), и без них раскладка зациклилась бы на целом документе.
fn based_on_chain(
    id: &StyleId,
    parent: impl Fn(&StyleId) -> Option<Option<StyleId>>,
) -> Vec<StyleId> {
    let mut chain = Vec::new();
    let mut visited = BTreeSet::new();
    let mut current = Some(id.clone());
    while let Some(style_id) = current {
        if chain.len() >= MAX_BASED_ON_DEPTH || !visited.insert(style_id.clone()) {
            break;
        }
        let Some(base) = parent(&style_id) else {
            break;
        };
        chain.push(style_id);
        current = base;
    }
    chain.reverse();
    chain
}

/// Тумблер свойства: `On` включает, `Off` выключает, `Inherit` и отсутствие элемента
/// оставляют значение предыдущего слоя.
///
/// Различать `Off` и «не задано» обязательно: стиль включает `w:b`, а прямое
/// форматирование гасит его через `w:b w:val="0"` — при единственном `bool` жирный
/// остался бы включённым (ADR-0013 §4).
fn apply_toggle(slot: &mut bool, value: Option<Toggle>) {
    match value {
        Some(Toggle::On) => *slot = true,
        Some(Toggle::Off) => *slot = false,
        Some(Toggle::Inherit) | None => {}
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use doc_converter_core::NodeId;

    use super::*;
    use crate::model::{
        BlockItem, Body, Metadata, NumberingTable, ParagraphStyle, Relationships, Settings,
        StyleTable,
    };

    /// Документ с пустой таблицей стилей: тесты переопределяют нужное сами.
    fn document(styles: StyleTable) -> Document {
        Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::ROOT,
                items: Vec::new(),
                sections: Vec::new(),
            },
            styles,
            numbering: NumberingTable::default(),
            settings: Settings::default(),
            metadata: Metadata::default(),
            rels: Relationships::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            headers: BTreeMap::new(),
            footers: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Таблица стилей из абзацных стилей.
    fn paragraph_styles(styles: Vec<ParagraphStyle>) -> StyleTable {
        let mut table = StyleTable::default();
        for style in styles {
            table.paragraph.insert(style.id.clone(), style);
        }
        table
    }

    /// Абзацный стиль; не заданное здесь — пусто.
    fn paragraph_style(
        id: &str,
        based_on: Option<&str>,
        ppr: RawPPr,
        rpr: RawRPr,
    ) -> ParagraphStyle {
        ParagraphStyle {
            id: StyleId::new(id),
            name: None,
            based_on: based_on.map(StyleId::new),
            next: None,
            link: None,
            is_default: false,
            hidden: false,
            custom: false,
            aliases: Vec::new(),
            ppr,
            rpr,
        }
    }

    /// Знаковый стиль; не заданное здесь — пусто.
    fn character_style(id: &str, based_on: Option<&str>, rpr: RawRPr) -> CharacterStyle {
        CharacterStyle {
            id: StyleId::new(id),
            name: None,
            based_on: based_on.map(StyleId::new),
            link: None,
            is_default: false,
            hidden: false,
            custom: false,
            aliases: Vec::new(),
            rpr,
        }
    }

    /// Абзац со ссылкой на стиль.
    fn paragraph(style: Option<&str>) -> Paragraph {
        Paragraph {
            id: NodeId::ROOT,
            ppr: Box::new(RawPPr {
                style: style.map(StyleId::new),
                ..RawPPr::default()
            }),
            mark_rpr: Box::new(RawRPr::default()),
            runs: Vec::new(),
            style_ref: style.map(StyleId::new),
            numbering_ref: None,
            section_break: None,
        }
    }

    /// Run со ссылкой на знаковый стиль и прямым форматированием.
    fn run(style: Option<&str>, rpr: RawRPr) -> Run {
        Run {
            id: NodeId::ROOT,
            rpr: Box::new(rpr),
            style_ref: style.map(StyleId::new),
            content: Vec::new(),
        }
    }

    /// Разобранная фикстура `test-fixtures/docx/styles`; нет файла — паника, а не пропуск теста.
    fn fixture_document(name: &str) -> Document {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx/styles")
            .join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        crate::open(bytes).expect("the fixture parses")
    }

    /// Сравнение пикселей с допуском: `f32` в `assert_eq!` не пройдёт `clippy::float_cmp`.
    #[track_caller]
    fn assert_px(actual: f32, expected: f32, message: &str) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{message}: {actual} != {expected}"
        );
    }

    /// Первый абзац тела фикстуры.
    fn first_paragraph(document: &Document) -> &Paragraph {
        document
            .body
            .items
            .iter()
            .find_map(|item| match item {
                BlockItem::Paragraph(paragraph) => Some(paragraph),
                BlockItem::Table(_) | BlockItem::SectPr(_) | BlockItem::Unknown { .. } => None,
            })
            .expect("the fixture has a paragraph")
    }

    #[test]
    fn apply_toggle_keeps_the_previous_value_on_inherit_and_absence() {
        let mut slot = true;
        apply_toggle(&mut slot, Some(Toggle::Off));
        assert!(!slot);
        apply_toggle(&mut slot, Some(Toggle::Inherit));
        assert!(!slot);
        apply_toggle(&mut slot, None);
        assert!(!slot);

        apply_toggle(&mut slot, Some(Toggle::On));
        assert!(slot);
        apply_toggle(&mut slot, Some(Toggle::Inherit));
        assert!(slot);
        apply_toggle(&mut slot, None);
        assert!(slot);
    }

    #[test]
    fn the_based_on_chain_fixture_inherits_every_link() {
        // Фикстура: Heading3 → Heading2 → Heading1 → Normal, свойства разложены по
        // цепочке так, что побеждает ближайший к листу стиль.
        let document = fixture_document("based_on_chain.docx");
        let resolved = resolve_paragraph(
            &document,
            first_paragraph(&document),
            &mut StyleCache::new(),
        );

        assert!(resolved.ppr.keep_next, "keepNext из Heading1");
        assert_eq!(resolved.ppr.outline_lvl, Some(2), "outlineLvl из Heading3");
        assert_px(
            resolved.ppr.spacing.before_px,
            twips_to_px(Twips::new(240)),
            "w:before из Heading1",
        );
        assert_px(
            resolved.ppr.spacing.after_px,
            twips_to_px(Twips::new(60)),
            "w:after из Heading1",
        );
        assert_eq!(resolved.ppr.spacing.line, LineRule::Auto(259.0 / 240.0));

        assert_eq!(resolved.rpr.font_family.as_deref(), Some("Calibri"));
        assert_eq!(resolved.rpr.size, HalfPoint::new(24), "sz из Heading3");
        assert!(resolved.rpr.bold, "w:b из Heading1");
        assert!(resolved.rpr.italic, "w:i из Heading3");
        assert_eq!(resolved.rpr.color, Some(0x1F_3763), "цвет из Heading3");
    }

    #[test]
    fn the_doc_defaults_fixture_reaches_a_paragraph_without_direct_formatting() {
        let document = fixture_document("doc_defaults.docx");
        let resolved = resolve_paragraph(
            &document,
            first_paragraph(&document),
            &mut StyleCache::new(),
        );

        assert_eq!(resolved.rpr.font_family.as_deref(), Some("Georgia"));
        assert_eq!(resolved.rpr.size, HalfPoint::new(26));
        assert_eq!(resolved.rpr.color, Some(0x1A_1A_1A));
        assert_eq!(resolved.ppr.jc, Some(Justification::Both));
        assert_px(
            resolved.ppr.spacing.before_px,
            0.0,
            "w:before из docDefaults",
        );
        assert_px(
            resolved.ppr.spacing.after_px,
            twips_to_px(Twips::new(240)),
            "w:after из docDefaults",
        );
        assert_eq!(resolved.ppr.spacing.line, LineRule::Auto(1.5));
    }

    #[test]
    fn a_run_style_that_names_no_character_style_ends_the_chain() {
        // В фикстуре `w:rStyle` указывает на абзацный стиль: знакового с таким
        // идентификатором нет, и это просто конец цепочки, а не ошибка.
        let document = fixture_document("based_on_chain.docx");
        let mut cache = StyleCache::new();
        let paragraph = resolve_paragraph(&document, first_paragraph(&document), &mut cache);
        let run = match &first_paragraph(&document).runs[0] {
            crate::model::Inline::Run(run) => run,
            other => panic!("the fixture run is a w:r: {other:?}"),
        };

        let resolved = resolve_run(&document, &paragraph, run, &mut cache);
        assert_eq!(resolved.size, HalfPoint::new(24));
        assert!(resolved.bold);
    }

    #[test]
    fn direct_toggle_off_beats_the_style_toggle() {
        let styles = paragraph_styles(vec![paragraph_style(
            "Normal",
            None,
            RawPPr::default(),
            RawRPr {
                b: Some(Toggle::On),
                sz: Some(HalfPoint::new(32)),
                ..RawRPr::default()
            },
        )]);
        let document = document(styles);
        let paragraph = paragraph(Some("Normal"));
        let mut cache = StyleCache::new();
        let resolved = resolve_paragraph(&document, &paragraph, &mut cache);

        let without_override = resolve_run(
            &document,
            &resolved,
            &run(None, RawRPr::default()),
            &mut cache,
        );
        assert!(without_override.bold, "стиль включает жирный");

        let overridden = resolve_run(
            &document,
            &resolved,
            &run(
                None,
                RawRPr {
                    b: Some(Toggle::Off),
                    sz: Some(HalfPoint::new(36)),
                    ..RawRPr::default()
                },
            ),
            &mut cache,
        );
        assert!(!overridden.bold, "w:b w:val=\"0\" гасит жирный стиля");
        assert_eq!(overridden.size, HalfPoint::new(36), "прямой w:sz побеждает");
    }

    #[test]
    fn inherit_does_not_clear_the_style_toggle() {
        let styles = paragraph_styles(vec![paragraph_style(
            "Normal",
            None,
            RawPPr::default(),
            RawRPr {
                b: Some(Toggle::On),
                i: Some(Toggle::On),
                ..RawRPr::default()
            },
        )]);
        let document = document(styles);
        let resolved = resolve_paragraph(
            &document,
            &paragraph(Some("Normal")),
            &mut StyleCache::new(),
        );
        let resolved = resolve_run(
            &document,
            &resolved,
            &run(
                None,
                RawRPr {
                    b: Some(Toggle::Inherit),
                    i: Some(Toggle::Inherit),
                    ..RawRPr::default()
                },
            ),
            &mut StyleCache::new(),
        );

        assert!(resolved.bold);
        assert!(resolved.italic);
    }

    #[test]
    fn a_cyclic_chain_ends_instead_of_looping() {
        // Цикл `a → b → a` парсер только предупреждает (ADR-0016 §2); раскладка обязана
        // завершиться — тест падал бы по таймауту, а не по assert.
        let styles = paragraph_styles(vec![
            paragraph_style(
                "a",
                Some("b"),
                RawPPr::default(),
                RawRPr {
                    b: Some(Toggle::On),
                    ..RawRPr::default()
                },
            ),
            paragraph_style(
                "b",
                Some("a"),
                RawPPr::default(),
                RawRPr {
                    i: Some(Toggle::On),
                    ..RawRPr::default()
                },
            ),
        ]);
        let document = document(styles);
        let resolved = resolve_paragraph(&document, &paragraph(Some("a")), &mut StyleCache::new());

        assert!(resolved.rpr.bold, "свойства стиля `a` применены");
        assert!(resolved.rpr.italic, "свойства стиля `b` применены");
    }

    #[test]
    fn a_missing_style_is_the_end_of_the_chain() {
        let document = document(paragraph_styles(vec![paragraph_style(
            "Orphan",
            Some("Vanished"),
            RawPPr::default(),
            RawRPr {
                b: Some(Toggle::On),
                ..RawRPr::default()
            },
        )]));
        let resolved = resolve_paragraph(
            &document,
            &paragraph(Some("Orphan")),
            &mut StyleCache::new(),
        );

        assert!(resolved.rpr.bold);

        let unresolvable = resolve_paragraph(
            &document,
            &paragraph(Some("Nowhere")),
            &mut StyleCache::new(),
        );
        assert_eq!(unresolvable.rpr, ResolvedRPr::default());
    }

    #[test]
    fn the_second_call_on_the_same_cache_gives_the_same_result() {
        let document = fixture_document("based_on_chain.docx");
        let paragraph = first_paragraph(&document);
        let mut cache = StyleCache::new();

        let first = resolve_paragraph(&document, paragraph, &mut cache);
        let second = resolve_paragraph(&document, paragraph, &mut cache);
        let fresh = resolve_paragraph(&document, paragraph, &mut StyleCache::new());

        assert_eq!(first, second);
        assert_eq!(first, fresh);
        assert!(!cache.paragraph.is_empty(), "второй вызов обошёлся кэшем");
    }

    #[test]
    fn character_styles_apply_after_the_paragraph_style() {
        let mut styles = paragraph_styles(vec![paragraph_style(
            "Quote",
            None,
            RawPPr::default(),
            RawRPr {
                i: Some(Toggle::On),
                color: Some(Color::Rgb(0x59_59_59)),
                ..RawRPr::default()
            },
        )]);
        styles.character.insert(
            StyleId::new("Emphasis"),
            character_style(
                "Emphasis",
                Some("Base"),
                RawRPr {
                    b: Some(Toggle::On),
                    ..RawRPr::default()
                },
            ),
        );
        styles.character.insert(
            StyleId::new("Base"),
            character_style(
                "Base",
                None,
                RawRPr {
                    u: Some(Underline::Single),
                    ..RawRPr::default()
                },
            ),
        );
        let document = document(styles);
        let resolved =
            resolve_paragraph(&document, &paragraph(Some("Quote")), &mut StyleCache::new());
        let resolved = resolve_run(
            &document,
            &resolved,
            &run(Some("Emphasis"), RawRPr::default()),
            &mut StyleCache::new(),
        );

        assert!(resolved.italic, "курсив из абзацного стиля");
        assert!(resolved.bold, "жирный из знакового стиля");
        assert_eq!(resolved.underline, Some(Underline::Single), "из Base");
        assert_eq!(resolved.color, Some(0x59_59_59));
    }

    #[test]
    fn mark_rpr_is_resolved_but_not_applied_to_runs() {
        let document = document(StyleTable::default());
        let mut paragraph = paragraph(None);
        *paragraph.mark_rpr = RawRPr {
            b: Some(Toggle::On),
            ..RawRPr::default()
        };

        let resolved = resolve_paragraph(&document, &paragraph, &mut StyleCache::new());
        assert!(resolved.mark_rpr.bold, "знак абзаца жирный");

        let resolved_run = resolve_run(
            &document,
            &resolved,
            &run(None, RawRPr::default()),
            &mut StyleCache::new(),
        );
        assert!(!resolved_run.bold, "к run'ам свойства знака абзаца не идут");
    }

    #[test]
    fn indent_fields_merge_across_layers_and_hanging_wins_inside_a_layer() {
        let styles = paragraph_styles(vec![paragraph_style(
            "Normal",
            None,
            RawPPr {
                ind: Some(Ind {
                    left: Some(Twips::new(720)),
                    right: Some(Twips::new(180)),
                    first_line: Some(Twips::new(180)),
                    hanging: None,
                }),
                ..RawPPr::default()
            },
            RawRPr::default(),
        )]);
        let document = document(styles);
        let mut paragraph = paragraph(Some("Normal"));
        paragraph.ppr.ind = Some(Ind {
            left: None,
            right: None,
            first_line: Some(Twips::new(180)),
            hanging: Some(Twips::new(360)),
        });

        let resolved = resolve_paragraph(&document, &paragraph, &mut StyleCache::new());

        assert_px(
            resolved.ppr.ind.left_px,
            twips_to_px(Twips::new(720)),
            "отступ слева из стиля",
        );
        assert_px(
            resolved.ppr.ind.right_px,
            twips_to_px(Twips::new(180)),
            "отступ справа из стиля",
        );
        assert_px(
            resolved.ppr.ind.first_line_px,
            -twips_to_px(Twips::new(360)),
            "w:hanging побеждает w:firstLine того же слоя",
        );
    }

    #[test]
    fn line_rules_translate_from_the_raw_spacing() {
        let cases = [
            (
                Some(LineSpacingRule::Exact),
                LineRule::Exact(twips_to_px(Twips::new(360))),
            ),
            (
                Some(LineSpacingRule::AtLeast),
                LineRule::AtLeast(twips_to_px(Twips::new(360))),
            ),
            (Some(LineSpacingRule::Auto), LineRule::Auto(1.5)),
            (None, LineRule::Auto(1.5)),
        ];

        for (rule, expected) in cases {
            let document = document(StyleTable::default());
            let mut paragraph = paragraph(None);
            paragraph.ppr.spacing = Some(ParagraphSpacing {
                line: Some(LineSpacing::new(360)),
                line_rule: rule.clone(),
                ..ParagraphSpacing::default()
            });

            let resolved = resolve_paragraph(&document, &paragraph, &mut StyleCache::new());
            assert_eq!(resolved.ppr.spacing.line, expected, "правило {rule:?}");
        }

        let document = document(StyleTable::default());
        let resolved = resolve_paragraph(&document, &paragraph(None), &mut StyleCache::new());
        assert_eq!(resolved.ppr.spacing.line, LineRule::Single, "w:line нет");
        assert!(resolved.ppr.widow_control, "в Word включён по умолчанию");
    }
}
