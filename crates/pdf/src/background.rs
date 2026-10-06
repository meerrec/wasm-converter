//! Заливки ячеек.
//!
//! Пока умеет одно: сплошной прямоугольник. Узоры Excel (растр), цветовые
//! шкалы и полосы данных условного форматирования — следующие срезы.

use doc_converter_render::display_list::Color;
use printpdf::Op;

use crate::layout::RectPt;
use crate::styles::to_pdf_color;

/// Залить прямоугольник сплошным цветом.
///
/// `page_height_pt` — высота страницы: у printpdf начало координат в левом
/// нижнем углу, а прямоугольник приходит сверху вниз.
pub fn fill_rect(ops: &mut Vec<Op>, rect: RectPt, page_height_pt: f32, color: Color) {
    ops.push(Op::SetFillColor {
        col: to_pdf_color(color),
    });
    ops.push(Op::DrawPolygon {
        polygon: rect.to_pdf(page_height_pt).to_polygon(),
    });
}
