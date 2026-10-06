//! Заливки ячеек.
//!
//! Пока умеет одно: сплошной прямоугольник. Узоры Excel (растр), цветовые
//! шкалы и полосы данных условного форматирования — следующие срезы.

use doc_converter_render::display_list::Color;
use printpdf::{LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, WindingOrder};

use crate::layout::RectPt;
use crate::styles::to_pdf_color;

/// Залить прямоугольник сплошным цветом.
///
/// `page_height_pt` — высота страницы: у printpdf начало координат в левом
/// нижнем углу, а прямоугольник приходит сверху вниз.
///
/// Вершины собираются вручную, а не через `Rect::to_polygon`: в вендоренном
/// printpdf `Rect::gen_points` трактует `y` как верхнюю кромку
/// (`bottom = y - height`), а `RectPt::to_pdf` кладёт в `y` нижнюю. Через
/// `to_polygon` полигон уезжал бы вниз на собственную высоту; тем же приёмом
/// вершины строят `image.rs` и `chart.rs`.
pub fn fill_rect(ops: &mut Vec<Op>, rect: RectPt, page_height_pt: f32, color: Color) {
    let (left, right) = (rect.x, rect.x + rect.w);
    let (bottom, top) = (page_height_pt - rect.y - rect.h, page_height_pt - rect.y);
    let points = [(left, bottom), (right, bottom), (right, top), (left, top)]
        .into_iter()
        .map(|(x, y)| LinePoint {
            p: Point { x: Pt(x), y: Pt(y) },
            bezier: false,
        })
        .collect();

    ops.push(Op::SetFillColor {
        col: to_pdf_color(color),
    });
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings: vec![PolygonRing { points }],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сверка координат с посчитанными вручную: `f32`-арифметика страницы.
    const EPS: f32 = 1e-3;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < EPS,
            "expected {expected}, got {actual}"
        );
    }

    /// Вершины полигона — то, что уйдёт в PDF, в порядке обхода.
    fn polygon_points(rect: RectPt, page_height_pt: f32) -> Vec<(f32, f32)> {
        let mut ops = Vec::new();
        fill_rect(
            &mut ops,
            rect,
            page_height_pt,
            Color::rgba(0x11, 0x22, 0x33, 0xFF),
        );
        let Some(Op::DrawPolygon { polygon }) = ops.get(1) else {
            panic!("ожидалась заливка полигоном");
        };
        assert_eq!(polygon.mode, PaintMode::Fill);
        polygon.rings[0]
            .points
            .iter()
            .map(|point| (point.p.x.0, point.p.y.0))
            .collect()
    }

    #[test]
    fn fill_lands_on_the_rect_band() {
        let points = polygon_points(
            RectPt {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 5.0,
            },
            100.0,
        );
        assert_eq!(points.len(), 4);

        // Полоса целиком: x от 10 до 40, y от 75 до 80.
        for &(x, y) in &points {
            assert!((10.0..=40.0).contains(&x), "x вне полосы: {x}");
            assert!((75.0..=80.0).contains(&y), "y вне полосы: {y}");
        }
        let min = |values: &[f32]| values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = |values: &[f32]| values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let xs: Vec<f32> = points.iter().map(|&(x, _)| x).collect();
        let ys: Vec<f32> = points.iter().map(|&(_, y)| y).collect();
        close(min(&xs), 10.0);
        close(max(&xs), 40.0);
        close(min(&ys), 75.0);
        close(max(&ys), 80.0);
    }

    #[test]
    fn fill_is_not_shifted_below_the_rect() {
        let points = polygon_points(
            RectPt {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 5.0,
            },
            100.0,
        );
        let top = points
            .iter()
            .map(|&(_, y)| y)
            .fold(f32::NEG_INFINITY, f32::max);
        // `to_polygon()` упирался в 75.0 — верхнюю кромку старой полосы 70..75.
        close(top, 80.0);
    }
}
