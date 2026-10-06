//! Текст ячейки в операциях printpdf.
//!
//! Перенос — [`break_lines`] из `doc-converter-render`, тот же вызов, что и в
//! canvas-пути (`xlsx::paint::wrap_layout`), с тем же шрифтом и кеглем в
//! пикселях. Точки разрыва совпадают по построению, а не по договорённости
//! (ADR-0007).
//!
//! Начертание (`pdf_font`) влияет только на то, каким встроенным шрифтом
//! записан текст: измеряет и переносит его regular, как canvas, — иначе
//! точки разрыва разъехались бы между экраном и PDF.

use std::ops::Range;

use doc_converter_render::display_list::TextAlign;
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_ID};
use doc_converter_render::text_measure::{break_lines, measure_text};
use doc_converter_xlsx::layout::PX_PER_POINT;
use doc_converter_xlsx::model::VerticalAlign;
use printpdf::{FontId as PdfFontId, Op, Point, Pt, TextItem};

use crate::layout::{PageGeometry, RectPx};
use crate::styles::{to_pdf_color, CellStyle};

/// Поля текста внутри ячейки, пиксели раскладки.
///
/// То же число, что `TEXT_PADDING` canvas-пути (`xlsx::paint`): текст в ячейке
/// на экране и в PDF должен стоять одинаково.
const TEXT_PADDING_PX: f32 = 3.0;

/// Ширина, в которую должен уместиться текст ячейки: без полей по краям.
#[must_use]
pub fn inner_width_px(page: &PageGeometry, rect: RectPx) -> f32 {
    rect.w - TEXT_PADDING_PX * page.scale() * 2.0
}

/// Заменить число, не помещающееся в ячейку, решётками — как Excel.
///
/// Canvas показывает то же самое (`xlsx::paint::HASHES`), иначе на экране
/// было бы видно число, а в PDF — нет.
#[must_use]
pub fn clip_number(
    text: String,
    size_px: f32,
    inner_w_px: f32,
    fonts: &mut FontRegistry,
) -> String {
    if measure_text(fonts, DEFAULT_FONT_ID, size_px, &text) > inner_w_px {
        return String::from("#####");
    }
    text
}

/// Нарисовать текст в прямоугольнике ячейки.
///
/// `rect` — прямоугольник ячейки в пикселях раскладки. Кегль и перенос берутся
/// из стиля, масштаб печати — из геометрии страницы. Пустой текст, нулевая
/// ячейка и текст без единой строки после переноса не рисуют ничего.
pub fn draw_cell_text(
    ops: &mut Vec<Op>,
    fonts: &mut FontRegistry,
    pdf_font: &PdfFontId,
    page: &PageGeometry,
    text: &str,
    style: &CellStyle,
    rect: RectPx,
) {
    let padding_px = TEXT_PADDING_PX * page.scale();
    let inner_w_px = rect.w - padding_px * 2.0;
    if text.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 || inner_w_px <= 0.0 {
        return;
    }

    let size_px = style.font_size_pt * PX_PER_POINT * page.scale();
    let lines: Vec<Range<usize>> = if style.wrap {
        break_lines(fonts, DEFAULT_FONT_ID, size_px, text, inner_w_px)
    } else {
        std::iter::once(0..text.len()).collect()
    };
    if lines.is_empty() {
        return;
    }

    let metrics = fonts.measure(DEFAULT_FONT_ID, size_px, "");
    let line_height_px = metrics.line_height;
    // Число строк много меньше 2^24 — точность f32 достаточна (как в canvas-пути).
    #[allow(clippy::cast_precision_loss)]
    let block_h_px = line_height_px * lines.len() as f32;
    let first_top_px = match style.vertical {
        VerticalAlign::Top => rect.y + padding_px,
        VerticalAlign::Center => rect.y + (rect.h - block_h_px) / 2.0,
        VerticalAlign::Bottom | VerticalAlign::Justify | VerticalAlign::Distributed => {
            rect.y + rect.h - padding_px - block_h_px
        }
    };

    ops.push(Op::SetFillColor {
        col: to_pdf_color(style.text_color),
    });

    for (index, range) in lines.iter().enumerate() {
        let line = &text[range.clone()];
        let width_px = measure_text(fonts, DEFAULT_FONT_ID, size_px, line);
        let line_x_px = match style.align {
            TextAlign::Center => rect.x + (rect.w - width_px) / 2.0,
            TextAlign::Right => rect.x + rect.w - padding_px - width_px,
            TextAlign::Left | TextAlign::Justify => rect.x + padding_px,
        };
        #[allow(clippy::cast_precision_loss)]
        let line_top_px = first_top_px + line_height_px * index as f32;
        // PDF ставит текст по базовой линии, а раскладка знает верх строки:
        // подъём берётся из метрик того же шрифта.
        let baseline_px = line_top_px + metrics.ascent;
        let cursor_x_pt = page.px_to_pt(line_x_px) + page.origin_x_pt();
        let baseline_y_pt = page.y_px_to_pt(baseline_px) + page.origin_y_pt();

        // Каждый фрагмент — своё `BT`/`ET`: `Td` внутри текстовой секции
        // смещает начало относительно предыдущего, а координаты у нас
        // абсолютные.
        ops.push(Op::StartTextSection);
        ops.push(Op::SetTextCursor {
            pos: Point {
                x: Pt(cursor_x_pt),
                y: Pt(page.height_pt() - baseline_y_pt),
            },
        });
        ops.push(Op::SetFontSize {
            size: Pt(style.font_size_pt * page.scale()),
            font: pdf_font.clone(),
        });
        ops.push(Op::WriteText {
            items: vec![TextItem::Text(line.to_owned())],
            font: pdf_font.clone(),
        });
        ops.push(Op::EndTextSection);
    }
}
