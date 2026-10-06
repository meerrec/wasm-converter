//! Границы ячеек.
//!
//! Рисуется только собственная рамка ячейки: приоритет соседних сторон
//! (canvas-путь разрешает конфликт `xlsx::paint::resolve_side`), пунктиры и
//! двойные линии — следующие срезы.

use doc_converter_render::display_list::Color;
use doc_converter_xlsx::paint::resolve_color;
use doc_converter_xlsx::{Border, BorderSide, Theme};
use printpdf::{Line, LinePoint, Op, Point, Pt};

use crate::layout::RectPt;
use crate::styles::{border_style, to_pdf_color};

/// Есть ли у рамки хотя бы одна видимая сторона.
///
/// Диагонали (`Border::diagonal`) пока не рисуются — их нет ни в одном
/// сценарии спринта.
#[must_use]
pub fn is_visible(border: &Border) -> bool {
    [&border.left, &border.right, &border.top, &border.bottom]
        .iter()
        .any(|side| border_style(side.style).is_visible())
}

/// Нарисовать рамку ячейки: стороны с ненулевой толщиной.
pub fn draw_border(
    ops: &mut Vec<Op>,
    rect: RectPt,
    page_height_pt: f32,
    border: &Border,
    theme: &Theme,
) {
    let pdf = rect.to_pdf(page_height_pt);
    let (x0, y0) = (pdf.x.0, pdf.y.0);
    let (x1, y1) = (x0 + pdf.width.0, y0 + pdf.height.0);

    // Стороны в координатах printpdf: начало — левый нижний угол.
    for (side, from, to) in [
        (border.top, (x0, y1), (x1, y1)),
        (border.bottom, (x0, y0), (x1, y0)),
        (border.left, (x0, y0), (x0, y1)),
        (border.right, (x1, y0), (x1, y1)),
    ] {
        let width = border_style(side.style);
        if !width.is_visible() {
            continue;
        }
        ops.push(Op::SetOutlineThickness {
            pt: Pt(width.width_pt),
        });
        ops.push(Op::SetOutlineColor {
            col: to_pdf_color(side_color(&side, theme)),
        });
        ops.push(Op::DrawLine {
            line: segment(from, to),
        });
    }
}

/// Цвет стороны: разрешённый из палитры, иначе чёрный — как у canvas.
fn side_color(side: &BorderSide, theme: &Theme) -> Color {
    resolve_color(theme, side.color).unwrap_or(Color::BLACK)
}

/// Отрезок по двум точкам.
fn segment(from: (f32, f32), to: (f32, f32)) -> Line {
    Line {
        points: vec![
            LinePoint {
                p: Point {
                    x: Pt(from.0),
                    y: Pt(from.1),
                },
                bezier: false,
            },
            LinePoint {
                p: Point {
                    x: Pt(to.0),
                    y: Pt(to.1),
                },
                bezier: false,
            },
        ],
        is_closed: false,
    }
}
