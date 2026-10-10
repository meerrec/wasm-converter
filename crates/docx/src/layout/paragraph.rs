//! Раскладка одного абзаца: строки, интервалы и флаги переноса.
//!
//! [`layout_paragraph`] отдаёт строки с готовыми координатами, а о разрывах
//! страниц только сообщает (`w:keepNext`, `w:keepLines`, `w:pageBreakBefore`,
//! `w:widowControl`): решения принимает движок, у которого есть страница целиком.

use doc_converter_render::font::FontId;

use crate::model::raw::{HalfPoint, Toggle};
use crate::model::{Anchor, Document, Inline, Paragraph, RunContent};

use super::cascade::StyleCache;
use super::engine::{LayoutOptions, LayoutState};
use super::line_break::LineBreaker;

/// Строка абзаца после раскладки.
#[derive(Debug, Clone)]
pub struct Line {
    /// Текст строки.
    pub text: String,
    /// Позиция X от левого края страницы.
    pub x: f32,
    /// Ширина строки.
    pub width: f32,
    /// Высота строки.
    pub height: f32,
    /// Цвет текста; `None` — цвет по умолчанию.
    pub color: Option<u32>,
}

/// Результат раскладки абзаца.
#[derive(Debug, Clone)]
pub struct ParagraphLayout {
    /// Строки сверху вниз.
    pub lines: Vec<Line>,
    /// Интервал перед абзацем.
    pub space_before: f32,
    /// Интервал после абзаца.
    pub space_after: f32,
    /// Абзац начинает новую страницу (`w:pageBreakBefore`).
    pub page_break_before: bool,
    /// Не отрывать абзац от следующего (`w:keepNext`).
    pub keep_next: bool,
    /// Не разрывать абзац между страницами (`w:keepLines`).
    pub keep_lines: bool,
    /// Не оставлять одну строку в начале или конце страницы (`w:widowControl`).
    pub widow_control: bool,
    /// Плавающие объекты абзаца.
    pub anchors: Vec<Anchor>,
}

/// Разложить абзац в строки.
///
/// Размер шрифта пока один на весь документ (12 pt): свойства run'ов учтёт
/// каскад, поэтому `document` и `cache` уже в подписи, но ещё не в работе.
/// `state` задаёт левую границу и ширину полосы набора, но не позицию Y:
/// строки ставит движок, который ведёт курсор страницы.
#[must_use]
pub fn layout_paragraph(
    _document: &Document,
    paragraph: &Paragraph,
    state: &LayoutState,
    options: &LayoutOptions,
    _cache: &mut StyleCache,
    line_breaker: &mut LineBreaker,
) -> ParagraphLayout {
    // Collect text from all runs
    let text: String = paragraph
        .runs
        .iter()
        .filter_map(|inline| {
            if let Inline::Run(run) = inline {
                Some(
                    run.content
                        .iter()
                        .filter_map(|content| {
                            if let RunContent::Text(text) = content {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .concat(),
                )
            } else {
                None
            }
        })
        .collect::<String>();

    // TODO: extract actual font size from run properties
    let font_size_half_points = HalfPoint::new(24); // 12pt default
    let default_font_id = FontId::default();
    let line_height = line_breaker.line_height_with_spacing(font_size_half_points, None);

    // Calculate available width
    let available_width = options.effective_width(state.column_width);

    // Break text into lines
    let line_ranges = line_breaker.break_lines(
        &text,
        default_font_id,
        font_size_half_points,
        available_width,
    );

    let lines = line_ranges
        .into_iter()
        .map(|line_range| {
            let line_text = &text[line_range];
            Line {
                text: line_text.to_string(),
                x: state.x,
                width: line_breaker.measure_text(line_text, default_font_id, font_size_half_points),
                height: line_height,
                color: None,
            }
        })
        .collect();

    ParagraphLayout {
        lines,
        // Интервалы останутся нулевыми до каскада: `w:spacing` приходит из стиля,
        // а без него раскладка сдвинула бы текст относительно сегодняшнего.
        space_before: 0.0,
        space_after: 0.0,
        page_break_before: is_on(paragraph.ppr.page_break_before),
        keep_next: is_on(paragraph.ppr.keep_next),
        keep_lines: is_on(paragraph.ppr.keep_lines),
        widow_control: is_on(paragraph.ppr.widow_control),
        // Плавающие объекты разберёт отдельный слайс.
        anchors: Vec::new(),
    }
}

/// Включён ли тумблер.
///
/// `Inherit` (`w:val="inherit"`) до каскада неотличим от `Off`: значение пришло
/// бы из стиля, а стилей раскладка ещё не видит.
fn is_on(value: Option<Toggle>) -> bool {
    matches!(value, Some(Toggle::On))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::raw::RawPPr;
    use crate::model::{Body, Metadata, NumberingTable, Relationships, Run, Settings, StyleTable};
    use doc_converter_core::NodeId;
    use doc_converter_render::font::FontRegistry;

    /// Документ с пустым телом: абзацу он пока нужен только как владелец стилей.
    fn document() -> Document {
        Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::new(1),
                items: Vec::new(),
                sections: Vec::new(),
            },
            styles: StyleTable::default(),
            numbering: NumberingTable::default(),
            settings: Settings::default(),
            metadata: Metadata::default(),
            rels: Relationships::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            headers: std::collections::BTreeMap::new(),
            footers: std::collections::BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Полоса набора шириной 600 px: раскладке абзаца важны только X и ширина.
    fn state() -> LayoutState {
        LayoutState {
            x: 0.0,
            y: 0.0,
            column_index: 0,
            column_width: 600.0,
            column_x: 0.0,
            content_top: 0.0,
            content_bottom: 1000.0,
        }
    }

    /// Абзац из одного run'а с текстом и заданным `w:pPr`.
    fn paragraph(text: &str, ppr: RawPPr) -> Paragraph {
        Paragraph {
            id: NodeId::new(1),
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text(text.to_owned())],
                ..Run::default()
            })],
            ppr: Box::new(ppr),
            ..Paragraph::default()
        }
    }

    // Интервалы пока ровно нули — сравнение точное, допуск тут ничего не проверял бы.
    #[allow(clippy::float_cmp)]
    #[test]
    fn flags_come_from_the_paragraph_properties() {
        let ppr = RawPPr {
            keep_next: Some(Toggle::On),
            keep_lines: Some(Toggle::Off),
            page_break_before: Some(Toggle::On),
            widow_control: Some(Toggle::Inherit),
            ..RawPPr::default()
        };
        let mut fonts = FontRegistry::new(64);
        let mut line_breaker = LineBreaker::new(&mut fonts, FontId::default());
        let mut cache = StyleCache::new();

        let layout = layout_paragraph(
            &document(),
            &paragraph("hello", ppr),
            &state(),
            &LayoutOptions::default(),
            &mut cache,
            &mut line_breaker,
        );

        assert!(layout.keep_next, "keepNext включён");
        assert!(!layout.keep_lines, "keepLines выключен явно");
        assert!(layout.page_break_before, "pageBreakBefore включён");
        assert!(!layout.widow_control, "inherit до каскада — не «включено»");
        assert!(layout.anchors.is_empty(), "плавающих объектов пока нет");
        assert_eq!(layout.space_before, 0.0);
        assert_eq!(layout.space_after, 0.0);
        assert_eq!(layout.lines.len(), 1, "короткий текст — одна строка");
        assert_eq!(layout.lines[0].text, "hello");
        assert!(layout.lines[0].width > 0.0);
        assert!(layout.lines[0].height > 0.0);
        assert_eq!(layout.lines[0].color, None);
    }
}
