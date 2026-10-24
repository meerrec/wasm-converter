//! Раскладка слайда: фиксированный размер, без пагинации (ADR-0033 §4).
//!
//! Слайд — одна страница рендера; координаты EMU переводятся в пиксели при
//! 96 dpi ([`doc_converter_pptx::model::emu_to_px`]). Текст переносится по словам теми же
//! метриками, что у DOCX (`doc_converter_render::text_measure`): прогон знает
//! свой кегль, строка собирается жадным word wrap'ом, высота строки — максимум
//! кеглей прогонов × 1.2. Автоподбор кегля (`a:normAutofit`) не применяется.
//!
//! Раскладка не зависит от окна и зума: их знает рендер, поэтому считается
//! один раз при открытии презентации.

use std::collections::BTreeMap;

use doc_converter_render::chart::ChartData;
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE, DEFAULT_FONT_ID};
use doc_converter_render::geometry::Rect;
use doc_converter_render::text_measure::measure_text;

use doc_converter_pptx::error::{Error, Result};
use doc_converter_pptx::model::{
    emu_to_px, pt_to_px, Align, Anchor, Bullet, GeometryPreset, Paragraph, Presentation, Rgb,
    RunProps, Shape, ShapeKind, TextBody,
};

/// Высота строки: кегль × [`LINE_SPACING`].
const LINE_SPACING: f32 = 1.2;

/// Отступ уровня списка, пиксели раскладки (0.25 дюйма).
const LEVEL_INDENT_PX: f32 = 24.0;

/// Радиус скругления `roundRect`, пиксели раскладки.
const ROUND_RECT_RADIUS: f32 = 8.0;

/// Раскладка одного слайда.
#[derive(Debug, Clone)]
pub struct SlideLayout {
    /// Ширина слайда в пикселях раскладки.
    pub width: f32,
    /// Высота слайда в пикселях раскладки.
    pub height: f32,
    /// Фон слайда; `None` — белый.
    pub background: Option<Rgb>,
    /// Элементы в порядке рисования (наложения).
    pub items: Vec<Item>,
    /// Сериализованные блобы диаграмм в порядке [`Item::Chart`] в `items`.
    ///
    /// Заполняются при раскладке: сериализация выполняется один раз на
    /// диаграмму, кадры берут готовые байты. Слайд после раскладки неизменяем,
    /// поэтому кэш не инвалидируется.
    pub chart_blobs: Vec<Vec<u8>>,
}

/// Элемент раскладки.
#[derive(Debug, Clone)]
pub enum Item {
    /// Заливка/контур шейпа одним `Rect` кадра.
    Rect {
        rect: Rect,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        /// Толщина контура в пикселях раскладки.
        stroke_w: f32,
        /// Радиус скругления углов.
        radius: f32,
    },
    /// Картинка.
    Image { rect: Rect, image_id: u32 },
    /// Диаграмма (ADR-0011).
    Chart { rect: Rect, data: ChartData },
    /// Строки текстовой рамки.
    Text { lines: Vec<TextLine> },
}

/// Строка текста с прогонами.
#[derive(Debug, Clone)]
pub struct TextLine {
    /// Левый верхний угол строки.
    pub x: f32,
    pub y: f32,
    /// Прогоны с уже посчитанными смещениями по горизонтали.
    pub runs: Vec<PlacedRun>,
}

/// Прогон на строке.
#[derive(Debug, Clone)]
pub struct PlacedRun {
    /// Смещение от [`TextLine::x`].
    pub x: f32,
    pub text: String,
    /// Кегль в пикселях раскладки.
    pub size_px: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<Rgb>,
    pub font: Option<String>,
}

/// Разложить слайд презентации.
///
/// # Errors
/// [`Error::NoSuchSlide`] — индекса нет в презентации.
pub fn layout_slide(pres: &Presentation, index: usize) -> Result<SlideLayout> {
    let slide = pres.slide(index).ok_or(Error::NoSuchSlide(index))?;
    let mut fonts = FontRegistry::new(DEFAULT_FONT_CACHE);
    let mut items = Vec::new();
    for shape in &slide.shapes {
        layout_shape(shape, &mut fonts, &mut items);
    }
    // Блобы — ровно в порядке `Item::Chart` в `items`: один пост-проход по
    // готовой раскладке, поэтому порядок не может разойтись с кадрами.
    let chart_blobs: Vec<Vec<u8>> = items
        .iter()
        .filter_map(|item| match item {
            Item::Chart { data, .. } => Some(data.to_blob()),
            _ => None,
        })
        .collect();
    Ok(SlideLayout {
        width: pres.size.width_px(),
        height: pres.size.height_px(),
        background: slide.background,
        items,
        chart_blobs,
    })
}

/// Разложить один шейп: сначала заливка/контур, поверх — содержимое.
fn layout_shape(shape: &Shape, fonts: &mut FontRegistry, items: &mut Vec<Item>) {
    let rect = shape.frame.px();

    // Радиус: у `ellipse` — половина меньшей стороны (прямоугольник со
    // скруглением по максимуму рисуется как эллипс), у `roundRect` — константа.
    let radius = match shape.kind {
        ShapeKind::Geometry(GeometryPreset::Ellipse) => rect.w.min(rect.h) / 2.0,
        ShapeKind::Geometry(GeometryPreset::RoundRect) => ROUND_RECT_RADIUS,
        _ => 0.0,
    };

    if shape.fill.is_some() || shape.line.is_some() {
        items.push(Item::Rect {
            rect,
            fill: shape.fill,
            stroke: shape.line.and_then(|line| line.color),
            stroke_w: shape
                .line
                .and_then(|line| line.width)
                .map_or(0.0, emu_to_px),
            radius,
        });
    }

    match &shape.kind {
        ShapeKind::Text(text_box) => {
            let lines = layout_text(&rect, &text_box.body, fonts);
            if !lines.is_empty() {
                items.push(Item::Text { lines });
            }
        }
        ShapeKind::Picture { image_id } => {
            if let Some(image_id) = *image_id {
                items.push(Item::Image { rect, image_id });
            }
        }
        ShapeKind::Chart(data) => {
            if !data.series.is_empty() {
                items.push(Item::Chart {
                    rect,
                    data: data.clone(),
                });
            }
        }
        ShapeKind::Geometry(_) => {}
    }
}

/// Разложить текст рамки в строки с прогонами.
fn layout_text(rect: &Rect, body: &TextBody, fonts: &mut FontRegistry) -> Vec<TextLine> {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return Vec::new();
    }
    let content = Rect::new(
        rect.x + emu_to_px(body.insets.left),
        rect.y + emu_to_px(body.insets.top),
        (rect.w - emu_to_px(body.insets.left) - emu_to_px(body.insets.right)).max(0.0),
        (rect.h - emu_to_px(body.insets.top) - emu_to_px(body.insets.bottom)).max(0.0),
    );
    // Без переноса (`wrap="none"`) строка не ограничена шириной.
    let max_width = if body.wrap { content.w } else { f32::INFINITY };

    let mut lines: Vec<TextLine> = Vec::new();
    let mut y = content.y;
    let mut auto_num: BTreeMap<u8, u32> = BTreeMap::new();

    for paragraph in &body.paragraphs {
        let default_size = paragraph
            .runs
            .first()
            .map_or(DEFAULT_PT, |run| run.props.size.unwrap_or(DEFAULT_PT));
        let indent = f32::from(paragraph.level) * LEVEL_INDENT_PX;

        let mut words = words_of(paragraph, &mut auto_num);
        if words.is_empty() {
            // Пустой абзац занимает строку.
            y += pt_to_px(default_size) * LINE_SPACING;
            continue;
        }

        let mut laid: Vec<TextLine> = Vec::new();
        let mut current: Vec<(PlacedRun, f32, bool)> = Vec::new(); // (run, width, space_after)
        let mut width = 0.0;

        for word in words.drain(..) {
            let size_px = pt_to_px(word.props.size.unwrap_or(default_size));
            let word_w = measure_text(fonts, DEFAULT_FONT_ID, size_px, &word.text);
            let space_w = measure_text(fonts, DEFAULT_FONT_ID, size_px, " ");

            let needs_break = !current.is_empty() && width + space_w + word_w > max_width;
            if needs_break {
                flush_line(&mut current, width, max_width, paragraph.align, &mut laid);
                width = 0.0;
            }
            let gap = if current.is_empty() { 0.0 } else { space_w };
            let run = PlacedRun {
                x: width + gap,
                text: word.text,
                size_px,
                bold: word.props.bold,
                italic: word.props.italic,
                underline: word.props.underline,
                color: word.props.color,
                font: word.props.font,
            };
            width = run.x + word_w;
            current.push((run, word_w, word.space_after));
            if word.line_break_after {
                flush_line(&mut current, width, max_width, paragraph.align, &mut laid);
                width = 0.0;
            }
        }
        flush_line(&mut current, width, max_width, paragraph.align, &mut laid);

        for mut line in laid {
            line.y = y;
            line.x = content.x + indent;
            y += line_height(&line);
            lines.push(line);
        }
        // Интервал после абзаца — половина строки.
        y += pt_to_px(default_size) * LINE_SPACING / 2.0;
    }

    if let Some(extra) = anchor_shift(body.anchor, content, y - content.y) {
        for line in &mut lines {
            line.y += extra;
        }
    }
    lines
}

/// Кегль по умолчанию для пустых абзацев, pt.
const DEFAULT_PT: f32 = 18.0;

/// Слово абзаца со свойствами и флагами пробела/разрыва.
struct Word {
    text: String,
    props: RunProps,
    space_after: bool,
    line_break_after: bool,
}

/// Разбить прогоны абзаца на слова; `\n` форсирует разрыв строки.
fn words_of(paragraph: &Paragraph, auto_num: &mut BTreeMap<u8, u32>) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();

    // Маркер списка — первое слово со свойствами первого прогона.
    let bullet = bullet_text(paragraph.bullet, paragraph.level, auto_num);
    if let Some(text) = bullet {
        let props = paragraph
            .runs
            .first()
            .map_or_else(RunProps::default, |run| run.props.clone());
        words.push(Word {
            text,
            props,
            space_after: true,
            line_break_after: false,
        });
    }

    for run in &paragraph.runs {
        let mut parts: Vec<&str> = run.text.split('\n').collect();
        let part_count = parts.len();
        for (index, part) in parts.drain(..).enumerate() {
            let mut word = String::new();
            for ch in part.chars() {
                if ch.is_whitespace() {
                    if !word.is_empty() {
                        words.push(Word {
                            text: word.clone(),
                            props: run.props.clone(),
                            space_after: true,
                            line_break_after: false,
                        });
                        word.clear();
                    } else if let Some(last) = words.last_mut() {
                        last.space_after = true;
                    }
                } else {
                    word.push(ch);
                }
            }
            if !word.is_empty() {
                words.push(Word {
                    text: word,
                    props: run.props.clone(),
                    space_after: false,
                    line_break_after: index + 1 < part_count,
                });
            }
            if index + 1 < part_count {
                if let Some(last) = words.last_mut() {
                    last.line_break_after = true;
                }
            }
        }
    }
    words
}

/// Текст маркера списка: `•`/`-`, автонумерация — счётчик уровня.
fn bullet_text(
    bullet: Option<Bullet>,
    level: u8,
    auto_num: &mut BTreeMap<u8, u32>,
) -> Option<String> {
    match bullet? {
        Bullet::Char(ch) => Some(format!("{ch} ")),
        Bullet::AutoNum { start } => {
            let counter = auto_num.entry(level).or_insert(start);
            let text = format!("{counter}. ");
            *counter = counter.saturating_add(1);
            Some(text)
        }
    }
}

/// Сложить собранную строку в раскладку с выравниванием.
fn flush_line(
    current: &mut Vec<(PlacedRun, f32, bool)>,
    width: f32,
    max_width: f32,
    align: Align,
    out: &mut Vec<TextLine>,
) {
    if current.is_empty() {
        return;
    }
    let shift = match align {
        Align::Left => 0.0,
        Align::Center => (max_width - width) / 2.0,
        Align::Right => max_width - width,
    }
    .max(0.0);
    let runs = current
        .drain(..)
        .map(|(mut run, _, _)| {
            run.x += shift;
            run
        })
        .collect();
    out.push(TextLine {
        x: 0.0,
        y: 0.0,
        runs,
    });
}

/// Высота строки: максимальный кегль прогонов × [`LINE_SPACING`].
fn line_height(line: &TextLine) -> f32 {
    line.runs.iter().map(|run| run.size_px).fold(0.0, f32::max) * LINE_SPACING
}

/// Сдвиг по вертикальному якорю: `Center`/`Bottom`.
fn anchor_shift(anchor: Anchor, content: Rect, total: f32) -> Option<f32> {
    let free = content.h - total;
    match anchor {
        Anchor::Top => None,
        Anchor::Center => Some((free / 2.0).max(0.0)),
        Anchor::Bottom => Some(free.max(0.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_converter_pptx::model::{Frame, Insets, Run, Slide, SlideSize, TextBox};

    fn run(text: &str, size: f32) -> Run {
        Run {
            text: text.to_owned(),
            props: RunProps {
                size: Some(size),
                ..RunProps::default()
            },
        }
    }

    fn body(paragraphs: Vec<Paragraph>) -> TextBody {
        TextBody {
            insets: Insets::default(),
            anchor: Anchor::Top,
            wrap: true,
            paragraphs,
        }
    }

    fn slide_with(text: TextBody, frame: Frame) -> (Presentation, usize) {
        let pres = Presentation {
            size: SlideSize::default(),
            slides: vec![Slide {
                name: "S".into(),
                background: None,
                shapes: vec![Shape {
                    name: "t".into(),
                    frame,
                    fill: None,
                    line: None,
                    kind: ShapeKind::Text(TextBox { body: text }),
                }],
            }],
            ..Presentation::default()
        };
        (pres, 0)
    }

    #[test]
    fn short_text_is_one_line() {
        let (pres, index) = slide_with(
            body(vec![Paragraph {
                runs: vec![run("Hello world", 18.0)],
                ..Paragraph::default()
            }]),
            Frame {
                x: 0,
                y: 0,
                w: 2_000_000,
                h: 1_000_000,
            },
        );
        let layout = layout_slide(&pres, index).unwrap();
        let text = layout
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { lines } => Some(lines),
                _ => None,
            })
            .unwrap();
        assert_eq!(text.len(), 1);
        assert_eq!(text[0].runs.len(), 2); // «Hello» и «world»
        assert_eq!(text[0].runs[0].text, "Hello");
        assert!(text[0].y >= 0.0);
    }

    #[test]
    fn narrow_frame_wraps_words() {
        let (pres, index) = slide_with(
            body(vec![Paragraph {
                runs: vec![run("one two three", 18.0)],
                ..Paragraph::default()
            }]),
            Frame {
                x: 0,
                y: 0,
                w: 600_000, // ~63 px — влезает одно короткое слово
                h: 2_000_000,
            },
        );
        let layout = layout_slide(&pres, index).unwrap();
        let lines = layout
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { lines } => Some(lines),
                _ => None,
            })
            .unwrap();
        assert!(
            lines.len() > 1,
            "три слова должны разложиться в несколько строк"
        );
    }

    #[test]
    fn missing_slide_is_an_error() {
        let pres = Presentation::default();
        assert!(matches!(layout_slide(&pres, 0), Err(Error::NoSuchSlide(0))));
    }
}
