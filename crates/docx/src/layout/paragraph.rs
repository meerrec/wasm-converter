//! Раскладка одного абзаца: строки, интервалы и флаги переноса.
//!
//! [`layout_paragraph`] отдаёт строки с готовыми координатами, а о разрывах
//! страниц только сообщает (`w:keepNext`, `w:keepLines`, `w:pageBreakBefore`,
//! `w:widowControl`): решения принимает движок, у которого есть страница целиком.
//!
//! Метрики приходят из каскада ([`super::cascade`]): кегль, цвет, интервалы и
//! отступы резолвятся из `docDefaults`, стиля абзаца и прямого форматирования.
//! Строка [`Line`] несёт один кегль и один цвет, поэтому текст абзаца режется на
//! отрезки по этим двум метрикам, и каждый отрезок переносится сам по себе:
//! смешанное форматирование внутри строки потребует спанов — их добавит
//! отдельный слайс.

use std::ops::Range;

use doc_converter_render::font::FontId;

use crate::model::raw::HalfPoint;
use crate::model::{Anchor, Document, Inline, Justification, Paragraph, Run, RunContent};

use super::cascade::{resolve_paragraph, resolve_run, ResolvedParagraph, StyleCache};
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

/// Строка до измерения: диапазон в тексте run'а, левый край её полосы и ширина полосы.
struct PendingLine {
    /// Байтовый диапазон строки в тексте run'а.
    range: Range<usize>,
    /// Позиция X полосы набора строки.
    x: f32,
    /// Ширина полосы набора строки.
    band: f32,
}

/// Отрезок абзаца с одними метриками: текст, кегль и цвет.
///
/// Строка [`Line`] несёт один кегль и один цвет, поэтому соседние run'ы с теми же
/// метриками раскладываются как один текст: абзац из сотни однородных run'ов
/// (обычное дело — так их оставляет Word) иначе рассыпался бы на сотню строк.
struct Segment {
    /// Текст отрезка.
    text: String,
    /// Кегль (`w:sz`).
    size: HalfPoint,
    /// Цвет текста (`w:color`).
    color: Option<u32>,
}

/// Собрать отрезки абзаца: текст run'ов, слитый по кеглю и цвету.
fn segments(
    document: &Document,
    paragraph: &Paragraph,
    resolved: &ResolvedParagraph,
    cache: &mut StyleCache,
) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for inline in &paragraph.runs {
        let Inline::Run(run) = inline else {
            // Прочие варианты `Inline` (гиперссылки, поля) раскладка пока не несёт.
            continue;
        };
        let text = run_text(run);
        if text.is_empty() {
            continue;
        }
        let rpr = resolve_run(document, resolved, run, cache);
        match segments.last_mut() {
            Some(last) if last.size == rpr.size && last.color == rpr.color => {
                last.text.push_str(&text);
            }
            _ => segments.push(Segment {
                text,
                size: rpr.size,
                color: rpr.color,
            }),
        }
    }
    segments
}

/// Разложить абзац в строки.
///
/// Отступы `w:ind` сужают полосу набора абзаца, а `w:firstLine`/`w:hanging`
/// сдвигает только её первую строку. `state` задаёт левую границу и ширину
/// полосы набора, но не позицию Y: строки ставит движок, который ведёт курсор
/// страницы.
#[must_use]
pub fn layout_paragraph(
    document: &Document,
    paragraph: &Paragraph,
    state: &LayoutState,
    options: &LayoutOptions,
    cache: &mut StyleCache,
    line_breaker: &mut LineBreaker,
) -> ParagraphLayout {
    let resolved = resolve_paragraph(document, paragraph, cache);
    let ind = resolved.ppr.ind;
    let jc = resolved.ppr.jc.as_ref();

    // Отступы слева и справа сужают полосу набора; первая строка получает ещё и
    // свой сдвиг — положительный отступ её сужает, выступ расширяет.
    let band = (options.effective_width(state.column_width) - ind.left_px - ind.right_px).max(0.0);
    let first_line_band = (band - ind.first_line_px).max(0.0);
    let origin = state.x + ind.left_px;

    let mut lines = Vec::new();
    for segment in &segments(document, paragraph, &resolved, cache) {
        let text = segment.text.as_str();
        let height = line_breaker.line_height_for(segment.size, resolved.ppr.spacing.line);

        // Первую строку абзаца отрезаем её полосой отдельно: `break_lines`
        // разбивает весь текст одной шириной, а у первой строки она другая.
        let mut pending = Vec::new();
        let mut laid_out = 0;
        if lines.is_empty() && ind.first_line_px != 0.0 {
            if let Some(range) = line_breaker
                .break_lines(text, FontId::default(), segment.size, first_line_band)
                .first()
                .cloned()
            {
                laid_out = range.end;
                pending.push(PendingLine {
                    range,
                    x: origin + ind.first_line_px,
                    band: first_line_band,
                });
            }
        }
        pending.extend(
            line_breaker
                .break_lines(&text[laid_out..], FontId::default(), segment.size, band)
                .into_iter()
                .map(|range| PendingLine {
                    range: range.start + laid_out..range.end + laid_out,
                    x: origin,
                    band,
                }),
        );

        for line in pending {
            let line_text = &text[line.range];
            let width = line_breaker.measure_text(line_text, FontId::default(), segment.size);
            lines.push(Line {
                text: line_text.to_owned(),
                x: line.x + align_shift(jc, line.band, width),
                width,
                height,
                color: segment.color,
            });
        }
    }

    // Абзац без текста всё равно занимает строку — как метка абзаца в Word.
    // Высота берётся из каскада: запасной вариант движка знает только 12 pt и
    // разошёлся бы с телом документа, набранным другим кеглем.
    if lines.is_empty() {
        lines.push(Line {
            text: String::new(),
            x: origin + ind.first_line_px + align_shift(jc, first_line_band, 0.0),
            width: 0.0,
            height: line_breaker.line_height_for(resolved.rpr.size, resolved.ppr.spacing.line),
            color: None,
        });
    }

    ParagraphLayout {
        lines,
        space_before: resolved.ppr.spacing.before_px,
        space_after: resolved.ppr.spacing.after_px,
        page_break_before: resolved.ppr.page_break_before,
        keep_next: resolved.ppr.keep_next,
        keep_lines: resolved.ppr.keep_lines,
        widow_control: resolved.ppr.widow_control,
        // Плавающие объекты разберёт отдельный слайс.
        anchors: Vec::new(),
    }
}

/// Текст run'а: содержимое `w:t`; `w:tab`, `w:br` и рисунки раскладка пока не несёт.
fn run_text(run: &Run) -> String {
    run.content
        .iter()
        .filter_map(|content| match content {
            RunContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Сдвиг строки внутри её полосы по `w:jc`.
///
/// `Both` и `Distribute` идут как левое: растянуть строку можно, лишь зная, какие
/// пробелы в ней растяжимы, — это отдельный слайс.
fn align_shift(jc: Option<&Justification>, band: f32, width: f32) -> f32 {
    match jc {
        Some(Justification::Center) => (band - width) / 2.0,
        Some(Justification::Right | Justification::End) => band - width,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::layout::engine::twips_to_px;
    use crate::model::raw::{
        Color, HalfPoint, Ind, LineSpacing, LineSpacingRule, ParagraphSpacing, RawPPr, RawRPr,
        Toggle,
    };
    use crate::model::{
        BlockItem, Body, DocDefaults, Metadata, NumberingTable, Relationships, Settings, StyleTable,
    };
    use crate::Twips;
    use doc_converter_core::NodeId;
    use doc_converter_render::font::FontRegistry;

    /// Сравнить пиксели с допуском: `f32` в `assert_eq!` не пройдёт `clippy::float_cmp`.
    #[track_caller]
    fn assert_px(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{what}: ожидалось {expected} px, получено {actual} px"
        );
    }

    /// Документ с пустым телом и заданными `docDefaults`.
    fn document_with_defaults(r_pr: RawRPr, p_pr: RawPPr) -> Document {
        Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::new(1),
                items: Vec::new(),
                sections: Vec::new(),
            },
            styles: StyleTable {
                doc_defaults: DocDefaults { r_pr, p_pr },
                ..StyleTable::default()
            },
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

    /// Документ без стилей: каскад отдаёт умолчание `docDefaults` — 12 pt без `w:line`.
    fn document() -> Document {
        document_with_defaults(RawRPr::default(), RawPPr::default())
    }

    /// Полоса набора шириной 600 px от левого края страницы.
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
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text(text.to_owned())],
                ..Run::default()
            })],
            ppr: Box::new(ppr),
            ..Paragraph::default()
        }
    }

    /// Разложить абзац в заданной полосе набора.
    fn layout(document: &Document, paragraph: &Paragraph, state: &LayoutState) -> ParagraphLayout {
        let mut fonts = FontRegistry::new(64);
        let mut line_breaker = LineBreaker::new(&mut fonts, FontId::default());
        layout_paragraph(
            document,
            paragraph,
            state,
            &LayoutOptions::default(),
            &mut StyleCache::new(),
            &mut line_breaker,
        )
    }

    /// Длинный текст: на 600 px он переносится, иначе проверить первую строку не на чем.
    const LONG_TEXT: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do \
                             eiusmod tempor incididunt ut labore et dolore magna aliqua ut enim";

    #[test]
    fn flags_and_spacing_come_from_the_cascade() {
        let ppr = RawPPr {
            keep_next: Some(Toggle::On),
            keep_lines: Some(Toggle::Off),
            page_break_before: Some(Toggle::On),
            widow_control: Some(Toggle::Inherit),
            spacing: Some(ParagraphSpacing {
                before: Some(Twips::new(240)),
                after: Some(Twips::new(120)),
                ..ParagraphSpacing::default()
            }),
            ..RawPPr::default()
        };

        let layout = layout(&document(), &paragraph("hello", ppr), &state());

        assert!(layout.keep_next, "keepNext включён");
        assert!(!layout.keep_lines, "keepLines выключен явно");
        assert!(layout.page_break_before, "pageBreakBefore включён");
        assert!(
            layout.widow_control,
            "widowControl включён в Word по умолчанию, а `inherit` его не снимает"
        );
        assert_px(layout.space_before, 16.0, "before 240 twips");
        assert_px(layout.space_after, 8.0, "after 120 twips");
        assert!(layout.anchors.is_empty(), "плавающих объектов пока нет");
        assert_eq!(layout.lines.len(), 1, "короткий текст — одна строка");
        assert_eq!(layout.lines[0].text, "hello");
        assert_px(layout.lines[0].x, 0.0, "без отступов строка у левого края");
        assert_px(layout.lines[0].height, 19.2, "12 pt без `w:line`");
        assert_eq!(layout.lines[0].color, None);
    }

    #[test]
    fn size_comes_from_the_document_defaults() {
        let document = document_with_defaults(
            RawRPr {
                sz: Some(HalfPoint::new(22)),
                ..RawRPr::default()
            },
            RawPPr::default(),
        );

        let layout = layout(&document, &paragraph("hello", RawPPr::default()), &state());

        // 11 pt = 14.667 px, межстрочный коэффициент 1.2 — 17.6 px.
        assert_px(layout.lines[0].height, 17.6, "кегль docDefaults");
    }

    #[test]
    fn line_rule_sets_the_height() {
        // 12 pt даёт базу 19.2 px: множитель 1.5 — 28.8, абсолютные правила — twips.
        let cases = [
            (LineSpacing::new(360), LineSpacingRule::Auto, 28.8),
            (LineSpacing::new(240), LineSpacingRule::Exact, 16.0),
            (LineSpacing::new(720), LineSpacingRule::AtLeast, 48.0),
        ];

        for (line, rule, expected) in cases {
            let ppr = RawPPr {
                spacing: Some(ParagraphSpacing {
                    line: Some(line),
                    line_rule: Some(rule),
                    ..ParagraphSpacing::default()
                }),
                ..RawPPr::default()
            };
            let layout = layout(&document(), &paragraph("hello", ppr), &state());
            assert_px(
                layout.lines[0].height,
                expected,
                "высота строки по `w:line`",
            );
        }
    }

    #[test]
    fn indents_narrow_the_band_and_shift_the_first_line() {
        let indented = RawPPr {
            ind: Some(Ind {
                left: Some(Twips::new(720)),
                right: Some(Twips::new(240)),
                ..Ind::default()
            }),
            ..RawPPr::default()
        };
        let layout = layout(&document(), &paragraph(LONG_TEXT, indented), &state());

        assert!(layout.lines.len() > 1, "длинный текст переносится");
        for line in &layout.lines {
            assert_px(line.x, 48.0, "отступ слева 720 twips");
            assert!(
                line.width <= 600.0 - 48.0 - 16.0 + 0.01,
                "строка шире полосы между отступами: {}",
                line.width
            );
        }
    }

    #[test]
    fn first_line_indent_moves_only_the_first_line() {
        let ppr = RawPPr {
            ind: Some(Ind {
                first_line: Some(Twips::new(720)),
                ..Ind::default()
            }),
            ..RawPPr::default()
        };
        let layout = layout(&document(), &paragraph(LONG_TEXT, ppr), &state());

        assert!(layout.lines.len() > 1, "длинный текст переносится");
        assert_px(layout.lines[0].x, 48.0, "первая строка со отступом");
        assert_px(layout.lines[1].x, 0.0, "последующие строки — без отступа");
    }

    #[test]
    fn hanging_indent_pulls_the_first_line_left() {
        let ppr = RawPPr {
            ind: Some(Ind {
                left: Some(Twips::new(1440)),
                hanging: Some(Twips::new(360)),
                ..Ind::default()
            }),
            ..RawPPr::default()
        };
        let layout = layout(&document(), &paragraph(LONG_TEXT, ppr), &state());

        assert!(layout.lines.len() > 1, "длинный текст переносится");
        assert_px(layout.lines[0].x, 96.0 - 24.0, "выступ первой строки");
        assert_px(
            layout.lines[1].x,
            96.0,
            "последующие строки — по отступу слева",
        );
    }

    #[test]
    fn alignment_shifts_the_line_inside_its_band() {
        let document = document();
        let jc = |value: Justification| RawPPr {
            jc: Some(value),
            ..RawPPr::default()
        };

        let left = layout(
            &document,
            &paragraph("short", jc(Justification::Left)),
            &state(),
        );
        let center = layout(
            &document,
            &paragraph("short", jc(Justification::Center)),
            &state(),
        );
        let right = layout(
            &document,
            &paragraph("short", jc(Justification::Right)),
            &state(),
        );
        let both = layout(
            &document,
            &paragraph("short", jc(Justification::Both)),
            &state(),
        );
        let width = left.lines[0].width;

        assert_px(left.lines[0].x, 0.0, "по левому краю");
        assert_px(center.lines[0].x, (600.0 - width) / 2.0, "по центру");
        assert_px(right.lines[0].x, 600.0 - width, "по правому краю");
        assert_px(both.lines[0].x, 0.0, "по ширине пока как по левому краю");
    }

    #[test]
    fn a_paragraph_without_text_takes_one_empty_line() {
        let defaults = document_with_defaults(
            RawRPr {
                sz: Some(HalfPoint::new(22)),
                ..RawRPr::default()
            },
            RawPPr::default(),
        );

        let layout = layout(&defaults, &paragraph("", RawPPr::default()), &state());

        assert_eq!(
            layout.lines.len(),
            1,
            "абзац без текста — одна пустая строка"
        );
        assert_eq!(layout.lines[0].text, "");
        assert_px(layout.lines[0].x, 0.0, "строка стоит у левого края полосы");
        assert_px(
            layout.lines[0].height,
            17.6,
            "высота пустой строки — из каскада, а не из умолчания движка",
        );
    }

    #[test]
    fn run_colour_reaches_the_line() {
        let paragraph = Paragraph {
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text("colored".to_owned())],
                rpr: Box::new(RawRPr {
                    color: Some(Color::Rgb(0x00FF_0000)),
                    ..RawRPr::default()
                }),
                ..Run::default()
            })],
            ..Paragraph::default()
        };

        let layout = layout(&document(), &paragraph, &state());

        assert_eq!(layout.lines[0].color, Some(0xFF_0000), "цвет run'а");
    }

    /// Путь к фикстуре от корня репозитория.
    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx")
            .join(name)
    }

    /// Разобрать фикстуру; её отсутствие роняет тест, а не пропускает его.
    fn fixture(name: &str) -> Document {
        let path = fixture_path(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|_| panic!("нет фикстуры {}", path.display()));
        crate::open(bytes).expect("фикстура разбирается")
    }

    /// Полоса набора из `w:sectPr` фикстуры: левое поле и ширина между полями.
    fn fixture_state(document: &Document) -> LayoutState {
        let section = document
            .body
            .sections
            .first()
            .expect("в фикстуре есть секция");
        let left = twips_to_px(section.margins.left);
        LayoutState {
            x: left,
            y: 0.0,
            column_index: 0,
            column_width: twips_to_px(section.page_size.width)
                - left
                - twips_to_px(section.margins.right),
            column_x: left,
            content_top: 0.0,
            content_bottom: 1000.0,
        }
    }

    /// Разложить абзац № `index` фикстуры её же полосой набора.
    fn layout_fixture(name: &str, index: usize) -> ParagraphLayout {
        let document = fixture(name);
        let paragraph = document
            .body
            .items
            .iter()
            .filter_map(|item| match item {
                BlockItem::Paragraph(paragraph) => Some(paragraph),
                _ => None,
            })
            .nth(index)
            .expect("абзац с таким номером есть в фикстуре");
        let state = fixture_state(&document);
        layout(&document, paragraph, &state)
    }

    /// `basic/indentation.docx`: отступы 720 twips = 48 px, 1440 = 96 px, выступ
    /// 360 twips = 24 px; левое поле 1134 twips = 75.6 px.
    #[test]
    fn fixture_indentation_places_the_lines() {
        let left = layout_fixture("basic/indentation.docx", 0);
        assert_px(left.lines[0].x, 123.6, "отступ слева 720 twips");

        let right = layout_fixture("basic/indentation.docx", 1);
        assert_px(
            right.lines[0].x,
            75.6,
            "отступ справа начало строки не сдвигает",
        );

        let first = layout_fixture("basic/indentation.docx", 2);
        assert_px(first.lines[0].x, 123.6, "отступ первой строки 720 twips");

        let hanging = layout_fixture("basic/indentation.docx", 3);
        assert_px(hanging.lines[0].x, 147.6, "отступ 1440 с выступом 360");
    }

    /// `basic/paragraph_spacing.docx`: `docDefaults` дают 11 pt (17.6 px) и
    /// `w:line="259"` (множитель 259/240); абсолютные правила — в twips.
    #[test]
    fn fixture_spacing_sets_heights_and_gaps() {
        let plain = layout_fixture("basic/paragraph_spacing.docx", 0);
        assert_px(plain.space_before, 16.0, "before 240 twips");
        assert_px(plain.space_after, 8.0, "after 120 twips");
        assert_px(
            plain.lines[0].height,
            17.6 * 259.0 / 240.0,
            "множитель docDefaults",
        );

        let auto = layout_fixture("basic/paragraph_spacing.docx", 1);
        assert_px(auto.lines[0].height, 17.6 * 1.5, "line 360, lineRule auto");

        let exact = layout_fixture("basic/paragraph_spacing.docx", 2);
        assert_px(exact.lines[0].height, 16.0, "line 240, lineRule exact");
        assert_px(exact.space_before, 0.0, "before 0");

        let at_least = layout_fixture("basic/paragraph_spacing.docx", 3);
        assert_px(at_least.lines[0].height, 48.0, "line 720, lineRule atLeast");
        assert_px(at_least.space_before, 24.0, "before 360 twips");
        assert_px(at_least.space_after, 24.0, "after 360 twips");
    }

    #[test]
    fn fixture_alignment_shifts_the_line() {
        let band = fixture_state(&fixture("basic/paragraph_alignment.docx")).column_width;
        let left = layout_fixture("basic/paragraph_alignment.docx", 0);
        let center = layout_fixture("basic/paragraph_alignment.docx", 1);
        let right = layout_fixture("basic/paragraph_alignment.docx", 2);
        let both = layout_fixture("basic/paragraph_alignment.docx", 3);

        assert_px(left.lines[0].x, 75.6, "по левому краю");
        assert_px(
            center.lines[0].x,
            75.6 + (band - center.lines[0].width) / 2.0,
            "по центру",
        );
        assert_px(
            right.lines[0].x,
            75.6 + band - right.lines[0].width,
            "по правому краю",
        );
        assert_px(both.lines[0].x, 75.6, "по ширине пока как по левому краю");
    }

    /// `formatting/heading_1.docx`: стиль `Heading1` даёт 16 pt (`w:sz w:val="32"`),
    /// а множитель интерлиньяжа 259/240 приходит из `docDefaults`.
    #[test]
    fn fixture_heading_uses_the_style_size() {
        let heading = layout_fixture("formatting/heading_1.docx", 0);
        let expected = 16.0 * 96.0 / 72.0 * 1.2 * 259.0 / 240.0;

        assert_px(heading.lines[0].height, expected, "кегль стиля Heading1");
        assert!(
            heading.lines[0].height > 19.2,
            "заголовок выше строки прежних 12 pt"
        );
        assert_eq!(
            heading.lines[0].color,
            Some(0x2F_5496),
            "цвет стиля Heading1"
        );
        assert!(heading.keep_next, "keepNext из стиля");
        assert!(heading.keep_lines, "keepLines из стиля");
        assert_px(heading.space_before, 16.0, "before 240 twips из стиля");
    }

    /// `basic/multiple_runs.docx`: у run'ов разные кегль и цвет, и строка несёт
    /// метрики того отрезка, из которого она собрана.
    #[test]
    fn fixture_run_metrics_reach_their_lines() {
        let layout = layout_fixture("basic/multiple_runs.docx", 0);
        let texts: Vec<&str> = layout.lines.iter().map(|line| line.text.as_str()).collect();

        // `w:b`, `w:i` и `w:u` метрик строки не меняют, поэтому четыре первых run'а
        // сливаются в один отрезок; дальше цвет, кегль и стиль знака режут по-своему.
        assert_eq!(
            texts,
            [
                "Plain bold italic underlined",
                "colored",
                "sized",
                "emphasized"
            ],
            "отрезки абзаца"
        );
        assert_eq!(
            layout.lines[0].color, None,
            "у run'ов без `w:color` цвета нет"
        );
        assert_px(
            layout.lines[0].height,
            17.6 * 259.0 / 240.0,
            "кегль docDefaults",
        );
        assert_eq!(layout.lines[1].color, Some(0xFF_0000), "цвет run'а");
        assert_px(
            layout.lines[2].height,
            16.0 * 96.0 / 72.0 * 1.2 * 259.0 / 240.0,
            "кегль run'а `w:sz 32`",
        );
    }
}
