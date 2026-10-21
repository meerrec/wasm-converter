//! Рендер слайда в `DisplayList` (ADR-0033 §5).
//!
//! Команды — те же `Rect`/`Line`/`Text`/`Image`/`Chart`, что у DOCX/XLSX;
//! формат `DisplayList` и `DL_VERSION` не меняются. Картинки в кадре — только
//! id (`Image.bitmap_id`): байты регистрирует потребитель (`pptx_image_bytes`
//! → `ImageBitmap` → `register_bitmap`), как у DOCX. Диаграмма сериализуется
//! в блоб [`doc_converter_render::chart::ChartData::to_blob`] и рисуется painter'ом.

use doc_converter_render::display_list::{
    Color, DisplayList, DrawCommand, TextAlign, TextBaseline,
};
use doc_converter_render::viewport::Viewport;

use crate::layout::{Item, PlacedRun, SlideLayout, TextLine};
use crate::model::Rgb;

/// Гарнитура по умолчанию кадра, когда прогон её не назвал.
const DEFAULT_FONT: &str = "Calibri";

/// Собрать слайд в `DisplayList`.
///
/// Координаты раскладки — пиксели от левого верхнего угла слайда; в кадр они
/// уходят физическими пикселями canvas: `to_screen` = (координата − прокрутка)
/// × масштаб. Кадр за клипом окна, как у DOCX (`doc_converter_docx::render`).
pub fn build(layout: &SlideLayout, viewport: &Viewport, out: &mut DisplayList) {
    out.clear();
    out.push(DrawCommand::Clear);

    out.push(DrawCommand::PushClip {
        x: 0.0,
        y: 0.0,
        w: viewport.w,
        h: viewport.h,
    });

    // Фон слайда: заданный цвет или белая страница.
    fill_rect(
        out,
        viewport,
        0.0,
        0.0,
        layout.width,
        layout.height,
        layout.background.map_or(Color::WHITE, rgb_color),
    );

    for item in &layout.items {
        draw_item(item, viewport, out);
    }

    out.push(DrawCommand::PopClip);
}

/// Один элемент раскладки.
fn draw_item(item: &Item, viewport: &Viewport, out: &mut DisplayList) {
    match item {
        Item::Rect {
            rect,
            fill,
            stroke,
            stroke_w,
            radius,
        } => {
            let (x, y) = to_screen(viewport, rect.x, rect.y);
            let w = rect.w * viewport.scale;
            let h = rect.h * viewport.scale;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            out.push(DrawCommand::Rect {
                x,
                y,
                w,
                h,
                fill: fill.map_or(Color::TRANSPARENT, rgb_color),
                stroke: stroke.map_or(Color::TRANSPARENT, rgb_color),
                stroke_w: *stroke_w * viewport.scale,
                radius: [*radius * viewport.scale; 4],
            });
        }
        Item::Image { rect, image_id } => {
            let (x, y) = to_screen(viewport, rect.x, rect.y);
            out.push(DrawCommand::Image {
                x,
                y,
                w: rect.w * viewport.scale,
                h: rect.h * viewport.scale,
                bitmap_id: *image_id,
            });
        }
        Item::Chart { rect, data } => {
            let (x, y) = to_screen(viewport, rect.x, rect.y);
            let blob = data.to_blob();
            let data = out.intern_bytes(&blob);
            out.push(DrawCommand::Chart {
                x,
                y,
                w: rect.w * viewport.scale,
                h: rect.h * viewport.scale,
                data,
            });
        }
        Item::Text { lines } => {
            for line in lines {
                for run in &line.runs {
                    draw_run(line, run, viewport, out);
                }
            }
        }
    }
}

/// Прогон строки: одна команда `Text`; выравнивание уже учтено в `run.x`.
fn draw_run(line: &TextLine, run: &PlacedRun, viewport: &Viewport, out: &mut DisplayList) {
    if run.text.is_empty() {
        return;
    }
    let (x, y) = to_screen(viewport, line.x + run.x, line.y);
    let text = out.intern(&run.text);
    let font = out.intern(run.font.as_deref().unwrap_or(DEFAULT_FONT));
    out.push(DrawCommand::Text {
        x,
        y,
        text,
        font,
        size: run.size_px * viewport.scale,
        color: run.color.map_or(Color::BLACK, rgb_color),
        align: TextAlign::Left,
        baseline: TextBaseline::Top,
        bold: run.bold,
        italic: run.italic,
        underline: run.underline,
    });
}

/// Прямоугольник одной командой `Rect` без рамки и скруглений.
fn fill_rect(
    out: &mut DisplayList,
    viewport: &Viewport,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fill: Color,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let (x, y) = to_screen(viewport, x, y);
    out.push(DrawCommand::Rect {
        x,
        y,
        w: w * viewport.scale,
        h: h * viewport.scale,
        fill,
        stroke: Color::TRANSPARENT,
        stroke_w: 0.0,
        radius: [0.0; 4],
    });
}

/// Точка раскладки → физические пиксели окна.
fn to_screen(viewport: &Viewport, x: f32, y: f32) -> (f32, f32) {
    (
        (x - viewport.x) * viewport.scale,
        (y - viewport.y) * viewport.scale,
    )
}

/// Цвет модели (`RRGGBB`) → `RRGGBBAA` `DisplayList`.
fn rgb_color(color: Rgb) -> Color {
    Color((color.to_u32() << 8) | 0xFF)
}

#[cfg(test)]
mod tests {
    use doc_converter_render::display_list::DisplayList;

    use super::*;
    use crate::layout::layout_slide;
    use crate::model::{
        Frame, Paragraph, Presentation, Run, RunProps, Shape, ShapeKind, Slide, SlideSize,
        TextBody, TextBox,
    };

    #[test]
    fn empty_slide_is_background_only() {
        let pres = Presentation {
            size: SlideSize::default(),
            slides: vec![Slide::default()],
            ..Presentation::default()
        };
        let layout = layout_slide(&pres, 0).unwrap();
        let mut out = DisplayList::new();
        build(&layout, &Viewport::default(), &mut out);
        let bytes = out.to_bytes();
        let reader = DisplayList::from_bytes(&bytes).unwrap();
        let commands: Vec<_> = reader.iter().collect::<Result<_, _>>().unwrap();
        assert!(matches!(commands[0], DrawCommand::Clear));
        assert!(commands
            .iter()
            .any(|cmd| { matches!(cmd, DrawCommand::Rect { fill, .. } if *fill == Color::WHITE) }));
    }

    #[test]
    fn text_run_lands_in_the_frame() {
        let pres = Presentation {
            size: SlideSize::default(),
            slides: vec![Slide {
                shapes: vec![Shape {
                    name: "t".into(),
                    frame: Frame {
                        x: 100_000,
                        y: 100_000,
                        w: 2_000_000,
                        h: 500_000,
                    },
                    fill: None,
                    line: None,
                    kind: ShapeKind::Text(TextBox {
                        body: TextBody {
                            paragraphs: vec![Paragraph {
                                runs: vec![Run {
                                    text: "Hi".into(),
                                    props: RunProps {
                                        size: Some(18.0),
                                        ..RunProps::default()
                                    },
                                }],
                                ..Paragraph::default()
                            }],
                            ..TextBody::default()
                        },
                    }),
                }],
                ..Slide::default()
            }],
            ..Presentation::default()
        };
        let layout = layout_slide(&pres, 0).unwrap();
        let mut out = DisplayList::new();
        build(&layout, &Viewport::default(), &mut out);
        let bytes = out.to_bytes();
        let reader = DisplayList::from_bytes(&bytes).unwrap();
        let commands: Vec<_> = reader.iter().collect::<Result<_, _>>().unwrap();
        let hi = commands
            .iter()
            .find(
                |cmd| matches!(cmd, DrawCommand::Text { text, .. } if reader.string(*text) == "Hi"),
            )
            .expect("текст должен попасть в кадр");
        match hi {
            DrawCommand::Text { x, y, size, .. } => {
                assert!(*x > 0.0 && *y > 0.0, "текст внутри слайда");
                assert!(*size > 0.0);
            }
            _ => unreachable!(),
        }
    }
}
