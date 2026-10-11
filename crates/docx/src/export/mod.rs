//! Экспорт разобранного документа в текстовые форматы: Markdown и обычный текст.
//!
//! Экспорт — чистая функция `&Document → String`: ни раскладки, ни пагинации здесь нет
//! (ADR-0020 §6: PDF, PNG, Markdown, HTML и CSV — отдельные экспортёры, а не writer
//! в DOCX). Общее для экспортёров — извлечение содержимого абзаца с разрешёнными
//! каскадом (`layout::cascade`) начертаниями, уровень заголовка и маркеры списков —
//! живёт в этом модуле, чтобы [`markdown`], [`text`] и [`html`] его не дублировали.
//! Место ячеек в сетке таблицы (объединения по вертикали) считает приватный `grid`:
//! HTML и Markdown должны одинаково понимать, что такое одна ячейка.
//!
//! Три формата — [`markdown::to_markdown`], [`text::to_text`] и
//! [`html::to_html`]; разница между ними только в разметке, разбор абзаца
//! общий.

mod grid;

pub mod html;
pub mod markdown;
pub mod text;

use std::collections::BTreeMap;

use crate::layout::cascade::{self, ResolvedParagraph, StyleCache};
use crate::model::drawing::InlineOrAnchor;
use crate::model::inline::{BreakKind, Hyperlink, Inline, Paragraph, Run, RunContent};
use crate::model::numbering::{Lvl, NumFmt, NumId, NumberingTable};
use crate::model::raw::{StyleId, Underline};
use crate::Document;

/// Начертания, которые текстовый экспорт умеет выразить разметкой.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Marks {
    /// Полужирный (`w:b`).
    pub(crate) bold: bool,
    /// Курсив (`w:i`).
    pub(crate) italic: bool,
    /// Подчёркивание (`w:u`); явное «нет» (`w:u w:val="none"`) подчёркиванием не считается.
    pub(crate) underline: bool,
    /// Зачёркивание (`w:strike`).
    pub(crate) strike: bool,
}

impl Marks {
    /// Начертания run'а после каскада.
    fn of(rpr: &cascade::ResolvedRPr) -> Self {
        Self {
            bold: rpr.bold,
            italic: rpr.italic,
            underline: rpr
                .underline
                .as_ref()
                .is_some_and(|underline| !matches!(underline, Underline::None)),
            strike: rpr.strike,
        }
    }
}

/// Кусок inline-содержимого абзаца.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Piece {
    /// Текст.
    Text(String),
    /// Табуляция (`w:tab`).
    Tab,
    /// Разрыв строки, страницы или колонки (`w:br`).
    Break(BreakKind),
    /// Рисунок: часть пакета и подпись.
    Image {
        /// Путь части внутри пакета (`word/media/...`); файл экспортёр не распаковывает.
        part: String,
        /// Подпись (`w:docPr w:descr` или имя файла).
        alt: String,
    },
    /// Гиперссылка (`w:hyperlink`): цель и содержимое.
    Link {
        /// Адрес цели; для ссылки внутри документа — `#закладка`.
        target: String,
        /// Содержимое ссылки; начертания внутри свои у каждого куска.
        inner: Vec<Segment>,
    },
}

/// Кусок содержимого абзаца с разрешёнными каскадом начертаниями.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    /// Что это за кусок.
    pub(crate) piece: Piece,
    /// Начертания куска (у ссылки — пустые: марки свои у её содержимого).
    pub(crate) marks: Marks,
}

/// Свойства абзаца после каскада стилей.
///
/// Обёртка над [`cascade::resolve_paragraph`]: потребителю (экспортёру) каскад нужен
/// ещё и для того, чтобы отличить заголовок от текста, и врозь эти два вызова
/// разрешали бы стили дважды.
pub(crate) fn paragraph_layout(
    document: &Document,
    paragraph: &Paragraph,
    cache: &mut StyleCache,
) -> ResolvedParagraph {
    cascade::resolve_paragraph(document, paragraph, cache)
}

/// Содержимое абзаца кусками: начертания разрешены каскадом (умолчания документа →
/// стиль абзаца → знаковый стиль → `w:rPr` run'а), соседние текстовые куски с
/// одинаковыми начертаниями и без ссылки слиты в один.
pub(crate) fn segments(
    document: &Document,
    resolved: &ResolvedParagraph,
    items: &[Inline],
    cache: &mut StyleCache,
) -> Vec<Segment> {
    let mut out = Vec::new();
    collect(document, resolved, items, cache, &mut out);
    merge(&mut out);
    out
}

/// Уровень заголовка 1..=9, если абзац — заголовок.
///
/// Стиль абзаца доезжает и до модели ([`Paragraph::style_ref`]), и до каскада:
/// `w:outlineLvl` цепочки `basedOn` даёт [`ResolvedParagraph::ppr`]`::outline_lvl`, и
/// им уровень структуры определяется независимо от того, как назван стиль
/// (`Heading 1`, `Заголовок 1` — имя стиля локализовано, а `w:outlineLvl` нет).
/// Значение 9 по ECMA-376 — «основной текст», а не девятый уровень.
/// Запасной признак — идентификатор стиля `Heading1`..`Heading9`: он нужен документу,
/// в котором стиля нет в `styles.xml` (тогда `w:outlineLvl` отсутствует).
pub(crate) fn heading_level(paragraph: &Paragraph, resolved: &ResolvedParagraph) -> Option<u8> {
    if let Some(outline) = resolved.ppr.outline_lvl {
        return (outline <= 8).then_some(outline + 1);
    }
    let style = paragraph
        .style_ref
        .as_ref()
        .or(paragraph.ppr.style.as_ref())?;
    style_heading_level(style)
}

/// Уровень заголовка по идентификатору стиля вида `Heading3`.
fn style_heading_level(style: &StyleId) -> Option<u8> {
    let digits = style.as_str().strip_prefix("Heading")?.trim();
    let level: u8 = digits.parse().ok()?;
    (1..=9).contains(&level).then_some(level)
}

/// Ссылка абзаца на нумерацию (`w:numId` и `w:ilvl`) после каскада.
///
/// `None` — абзац не пункт списка: нумерации нет или она снята (`w:numId` = 0).
pub(crate) fn list_ref(paragraph: &Paragraph, resolved: &ResolvedParagraph) -> Option<(NumId, u8)> {
    let num_pr = resolved.ppr.num_pr.as_ref();
    let num_id = num_pr
        .and_then(|num_pr| num_pr.num_id)
        .or(paragraph.numbering_ref)?;
    if num_id.value() == 0 {
        return None;
    }
    let ilvl = num_pr.and_then(|num_pr| num_pr.ilvl).unwrap_or(0);
    Some((num_id, ilvl.min(8)))
}

/// Счётчики пунктов списков на время обхода тела документа.
///
/// Состояние на документ, а не на абзац: номер пункта — свойство
/// последовательности, а `w:startOverride` сдвигает её начало.
#[derive(Debug, Default)]
pub(crate) struct ListNumbering {
    counters: BTreeMap<(NumId, u8), u32>,
}

impl ListNumbering {
    /// Маркер пункта списка: `- ` для `w:numFmt="bullet"`, `5. ` или `1.1. ` — по
    /// `w:lvlText` уровня и счётчику. Нераспознанная нумерация даёт `- `: структуру
    /// списка это сохраняет, а выдумывать формат номера не из чего.
    pub(crate) fn marker(&mut self, numbering: &NumberingTable, num_id: NumId, ilvl: u8) -> String {
        let (level, start) = level_and_start(numbering, num_id, ilvl);
        let Some(level) = level else {
            return "- ".to_owned();
        };
        if matches!(level.num_fmt, NumFmt::Bullet) {
            return "- ".to_owned();
        }
        // Пункт уровня `ilvl` начинает более глубокие уровни заново.
        self.counters
            .retain(|&(id, level), _| id != num_id || level <= ilvl);
        let counter = self.counters.entry((num_id, ilvl)).or_insert(start);
        let value = *counter;
        *counter = counter.saturating_add(1);
        let text = render_counter(self, numbering, num_id, ilvl, value, &level.lvl_text);
        format!("{text} ")
    }
}

/// Уровень `w:ilvl` списка `num_id` и его начальный номер.
///
/// `w:lvlOverride` замещает уровень схемы, `w:startOverride` — начальный номер.
fn level_and_start(numbering: &NumberingTable, num_id: NumId, ilvl: u8) -> (Option<&Lvl>, u32) {
    let Some(num) = numbering.nums.get(&num_id) else {
        return (None, 1);
    };
    let override_lvl = num.overrides.get(&ilvl);
    let level = override_lvl.and_then(|over| over.lvl.as_ref()).or_else(|| {
        numbering
            .abstract_nums
            .get(&num.abstract_id)?
            .levels
            .get(&ilvl)
    });
    let start = override_lvl
        .and_then(|over| over.start_override)
        .or_else(|| level.map(|level| level.start))
        .unwrap_or(1);
    (level, start)
}

/// Текст счётчика по `w:lvlText`: `%1`..`%9` заменяются номерами соответствующих уровней.
fn render_counter(
    numbering_state: &ListNumbering,
    numbering: &NumberingTable,
    num_id: NumId,
    ilvl: u8,
    value: u32,
    lvl_text: &str,
) -> String {
    if lvl_text.is_empty() {
        return format!("{value}.");
    }
    if !lvl_text.contains('%') {
        return lvl_text.to_owned();
    }
    let mut out = String::with_capacity(lvl_text.len() + 4);
    let mut chars = lvl_text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(10)) else {
            out.push('%');
            continue;
        };
        chars.next();
        let Some(level) = digit
            .checked_sub(1)
            .and_then(|level| u8::try_from(level).ok())
        else {
            // `%0` — не плейсхолдер уровня: остаётся в тексте как есть.
            out.push('%');
            out.push('0');
            continue;
        };
        let number = if level == ilvl {
            value
        } else {
            numbering_state
                .counters
                .get(&(num_id, level))
                .copied()
                .unwrap_or_else(|| level_and_start(numbering, num_id, level).1)
        };
        let num_fmt = level_and_start(numbering, num_id, level)
            .0
            .map(|lvl| &lvl.num_fmt);
        out.push_str(&format_number(number, num_fmt));
    }
    out
}

/// Номер в формате `w:numFmt`; форматы, которые здесь не выражаются, — десятичные.
fn format_number(value: u32, num_fmt: Option<&NumFmt>) -> String {
    match num_fmt {
        Some(NumFmt::LowerLetter) => letters(value, false),
        Some(NumFmt::UpperLetter) => letters(value, true),
        Some(NumFmt::UpperRoman) => roman(value).unwrap_or_else(|| value.to_string()),
        Some(NumFmt::LowerRoman) => {
            roman(value).map_or_else(|| value.to_string(), |roman| roman.to_lowercase())
        }
        _ => value.to_string(),
    }
}

/// Римское число; `None` — вне представимого диапазона (1..=3999).
fn roman(value: u32) -> Option<String> {
    const PLACES: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    if value == 0 || value > 3999 {
        return None;
    }
    let mut rest = value;
    let mut out = String::new();
    for (place, symbol) in PLACES {
        while rest >= place {
            out.push_str(symbol);
            rest -= place;
        }
    }
    Some(out)
}

/// Номер буквами: `1` → `a`/`A`, `27` → `aa`/`AA`.
fn letters(value: u32, upper: bool) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let base = if upper { b'A' } else { b'a' };
    let mut rest = value;
    let mut out = Vec::new();
    while rest > 0 {
        let offset = u8::try_from((rest - 1) % 26).unwrap_or(0);
        out.push(base + offset);
        rest = (rest - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

/// Обрезает переводы строк в конце — блоки разделяются ровно одной пустой строкой.
pub(crate) fn trim_end_newlines(text: &mut String) {
    while text.ends_with('\n') {
        text.pop();
    }
}

/// Содержимое абзаца: run'ы, ссылки, поля и закладки по порядку.
fn collect(
    document: &Document,
    resolved: &ResolvedParagraph,
    items: &[Inline],
    cache: &mut StyleCache,
    out: &mut Vec<Segment>,
) {
    for item in items {
        match item {
            Inline::Run(run) => collect_run(document, resolved, run, cache, out),
            Inline::Hyperlink(hyperlink) => {
                collect_hyperlink(document, resolved, hyperlink, cache, out);
            }
            Inline::Field(field) => collect(document, resolved, &field.result, cache, out),
            Inline::Break(kind) => out.push(segment(Piece::Break(kind.clone()))),
            Inline::Tab => out.push(segment(Piece::Tab)),
            Inline::Symbol { char, .. } => out.push(segment(Piece::Text(char.to_string()))),
            Inline::Drawing(drawing) => {
                if let Some(piece) = image_piece(drawing) {
                    out.push(segment(piece));
                }
            }
            Inline::Bookmark(_) | Inline::Unknown { .. } => {}
        }
    }
}

/// Содержимое run'а с начертаниями после каскада.
fn collect_run(
    document: &Document,
    resolved: &ResolvedParagraph,
    run: &Run,
    cache: &mut StyleCache,
    out: &mut Vec<Segment>,
) {
    let marks = Marks::of(&cascade::resolve_run(document, resolved, run, cache));
    for content in &run.content {
        let piece = match content {
            RunContent::Text(text) => Some(Piece::Text(text.clone())),
            RunContent::Tab => Some(Piece::Tab),
            RunContent::Break(kind) => Some(Piece::Break(kind.clone())),
            RunContent::Symbol { char, .. } => Some(Piece::Text(char.to_string())),
            RunContent::Drawing(drawing) => image_piece(drawing),
            RunContent::Unknown { .. } => None,
        };
        if let Some(piece) = piece {
            out.push(Segment { piece, marks });
        }
    }
}

/// Содержимое гиперссылки: цель — адрес (`w:hyperlink r:id`) или закладка
/// внутри документа (`w:anchor`), содержимое — её runs. Ссылка без цели
/// разворачивается в своё содержимое.
fn collect_hyperlink(
    document: &Document,
    resolved: &ResolvedParagraph,
    hyperlink: &Hyperlink,
    cache: &mut StyleCache,
    out: &mut Vec<Segment>,
) {
    let mut inner = Vec::new();
    collect(document, resolved, &hyperlink.runs, cache, &mut inner);
    merge(&mut inner);
    if inner.is_empty() {
        return;
    }
    let target = hyperlink
        .target
        .clone()
        .or_else(|| hyperlink.anchor.as_ref().map(|anchor| format!("#{anchor}")));
    match target {
        Some(target) => out.push(segment(Piece::Link { target, inner })),
        None => out.extend(inner),
    }
}

/// Рисунок как кусок содержимого; `None` — у рисунка нет разрешённой части пакета.
fn image_piece(drawing: &InlineOrAnchor) -> Option<Piece> {
    let image = drawing
        .inline
        .as_ref()
        .or_else(|| drawing.anchor.as_ref().map(|anchor| &anchor.image))?;
    let part = image.part.clone()?;
    let alt = image
        .description
        .clone()
        .or_else(|| image.name.clone())
        .unwrap_or_default();
    Some(Piece::Image { part, alt })
}

/// Кусок без начертаний.
fn segment(piece: Piece) -> Segment {
    Segment {
        piece,
        marks: Marks::default(),
    }
}

/// Сливает соседние текстовые куски с одинаковыми начертаниями: разбиение на run'ы
/// в модели произвольно, а `**по**` и `**ловина**` в разметке — это один `**половина**`.
fn merge(segments: &mut Vec<Segment>) {
    for segment in segments.iter_mut() {
        if let Piece::Link { inner, .. } = &mut segment.piece {
            merge(inner);
        }
    }
    let mut merged: Vec<Segment> = Vec::with_capacity(segments.len());
    for segment in segments.drain(..) {
        let appended = match (merged.last_mut(), &segment.piece) {
            (Some(previous), Piece::Text(text)) if previous.marks == segment.marks => {
                if let Piece::Text(existing) = &mut previous.piece {
                    existing.push_str(text);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !appended {
            merged.push(segment);
        }
    }
    *segments = merged;
}
