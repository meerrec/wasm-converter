//! PDF-бэкенд диаграмм: примитивы раскладки (ADR-0011) в операции printpdf.
//!
//! Геометрию считает [`doc_converter_render::chart::layout`], бэкенд только
//! переводит её в точки и рисует — в том же порядке, что canvas-бэкенд
//! (`painter/chart.rs`): порядок операций задаёт наложение, и расходиться ему
//! нельзя.
//!
//! Единицы: `rect` и всё, что вернул `layout`, — пиксели раскладки листа, ось
//! `y` вниз. В точки их переводит [`PageGeometry`] (96 dpi против 72 pt плюс
//! масштаб печати) — как в `text.rs`: один множитель на всю диаграмму, поэтому
//! геометрия, кегль и толщины линий масштабируются вместе (ADR-0011 §2).

use doc_converter_render::chart::{layout, ChartData, ChartPrim, TextAlign};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_ID};
use doc_converter_render::geometry::Rect;
use printpdf::{
    FontId as PdfFontId, Line, LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, TextItem,
    WindingOrder,
};

use crate::layout::{PageGeometry, RectPx};
use crate::styles::to_pdf_color;

/// Отношение радиуса контрольной точки Безье к радиусу дуги для четверти
/// окружности: `4/3 * (sqrt(2) - 1)`. У PDF нет команд дуги, окружность
/// собирается из четырёх таких четвертей.
const KAPPA: f64 = 0.552_284_749_830_793_6;

/// Нарисовать диаграмму в прямоугольнике `rect` (пиксели раскладки листа).
///
/// Пустая раскладка — нулевой прямоугольник или ни одной серии — не рисует
/// ничего: ни клипа, ни обёртки `q`/`Q`.
pub fn draw(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    pdf_font: &PdfFontId,
    page: &PageGeometry,
    rect: RectPx,
    data: &ChartData,
) {
    let prims = layout(Rect::new(rect.x, rect.y, rect.w, rect.h), data);
    if prims.is_empty() {
        return;
    }

    // Клип по прямоугольнику диаграммы — как в canvas-бэкенде: подписи осей и
    // легенда, вылезшие за якорь, не должны попадать на страницу. Клип обязан
    // жить между `q` и `Q`: вне пары он срезал бы всё последующее содержимое
    // страницы.
    ops.push(Op::SaveGraphicsState);
    ops.push(Op::DrawPolygon {
        polygon: rect_polygon(page, rect, PaintMode::Clip),
    });

    for prim in &prims {
        draw_prim(ops, registry, pdf_font, page, prim);
    }

    ops.push(Op::RestoreGraphicsState);
}

/// Один примитив в операциях; порядок вызовов задаёт [`draw`].
fn draw_prim(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    pdf_font: &PdfFontId,
    page: &PageGeometry,
    prim: &ChartPrim,
) {
    match prim {
        ChartPrim::Rect { x, y, w, h, fill } => {
            ops.push(Op::SetFillColor {
                col: to_pdf_color(*fill),
            });
            ops.push(Op::DrawPolygon {
                polygon: rect_polygon(
                    page,
                    RectPx::new(to_f32(*x), to_f32(*y), to_f32(*w), to_f32(*h)),
                    PaintMode::Fill,
                ),
            });
        }
        ChartPrim::Polyline {
            points,
            stroke,
            width,
            closed,
        } => {
            if points.is_empty() {
                return;
            }
            ops.push(Op::SetOutlineThickness {
                pt: Pt(page.px_to_pt(to_f32(*width))),
            });
            ops.push(Op::SetOutlineColor {
                col: to_pdf_color(*stroke),
            });
            ops.push(Op::DrawLine {
                line: Line {
                    points: line_points(page, points),
                    is_closed: *closed,
                },
            });
        }
        ChartPrim::Polygon { points, fill } => {
            if points.is_empty() {
                return;
            }
            ops.push(Op::SetFillColor {
                col: to_pdf_color(*fill),
            });
            ops.push(Op::DrawPolygon {
                polygon: filled_polygon(line_points(page, points)),
            });
        }
        ChartPrim::Circle { cx, cy, r, fill } => {
            // Отрицательный радиус canvas отвергает и заливает пустой путь —
            // в PDF такой окружности тоже не должно быть.
            if *r <= 0.0 {
                return;
            }
            ops.push(Op::SetFillColor {
                col: to_pdf_color(*fill),
            });
            ops.push(Op::DrawPolygon {
                polygon: filled_polygon(circle_ring(page, *cx, *cy, *r)),
            });
        }
        // Секторы круговой диаграммы — слайс C4: примитив пропускается, а не
        // рисуется приблизительно.
        ChartPrim::Sector { .. } => {}
        ChartPrim::Text {
            x,
            y,
            text,
            size,
            align,
            fill,
        } => {
            if text.is_empty() {
                return;
            }
            let size_px = to_f32(*size);
            // Кегль примитива — в пикселях диаграммы, а метрики линейны по
            // переданному размеру: ширина строки приходит в тех же пикселях,
            // что и координаты, — выравнивание обходится без перевода в точки.
            let width_px = registry.measure(DEFAULT_FONT_ID, size_px, text).width;
            let anchor_x = match align {
                TextAlign::Left => *x,
                TextAlign::Center => *x - f64::from(width_px) / 2.0,
                TextAlign::Right => *x - f64::from(width_px),
            };
            ops.push(Op::SetFillColor {
                col: to_pdf_color(*fill),
            });
            // `y` примитива — уже базовая линия (canvas ставит тот же
            // baseline), поэтому курсор встаёт прямо на неё, без подъёма на
            // ascent, который нужен `text.rs` для верхней границы строки.
            ops.push(Op::StartTextSection);
            ops.push(Op::SetTextCursor {
                pos: to_pdf_point(page, anchor_x, *y),
            });
            ops.push(Op::SetFontSize {
                size: Pt(page.px_to_pt(size_px)),
                font: pdf_font.clone(),
            });
            ops.push(Op::WriteText {
                items: vec![TextItem::Text(text.clone())],
                font: pdf_font.clone(),
            });
            ops.push(Op::EndTextSection);
        }
    }
}

/// Пиксели диаграммы (`f64`, ось `y` вниз) в координаты printpdf: тот же
/// перевод, что в `text.rs` — начало координат в левом нижнем углу страницы.
fn to_pdf_point(page: &PageGeometry, x: f64, y: f64) -> Point {
    Point {
        x: Pt(page.origin_x_pt() + page.px_to_pt(to_f32(x))),
        y: Pt(page.height_pt() - (page.origin_y_pt() + page.y_px_to_pt(to_f32(y)))),
    }
}

/// Точки ломаной или контура в координатах printpdf; все — вершины, а не
/// управляющие Безье.
fn line_points(page: &PageGeometry, points: &[(f64, f64)]) -> Vec<LinePoint> {
    points
        .iter()
        .map(|&(x, y)| LinePoint {
            p: to_pdf_point(page, x, y),
            bezier: false,
        })
        .collect()
}

/// Замкнутый залитый контур: canvas заливает `Polygon` неявным замыканием,
/// printpdf замыкает путь сам (`h`) перед заливкой.
fn filled_polygon(points: Vec<LinePoint>) -> Polygon {
    Polygon {
        rings: vec![PolygonRing { points }],
        mode: PaintMode::Fill,
        winding_order: WindingOrder::NonZero,
    }
}

/// Полигон прямоугольника: якорь диаграммы для клипа и заливка `Rect`.
///
/// Вершины собираются вручную, а не через `Rect::to_polygon`: в вендоренном
/// printpdf `Rect::gen_points` отсчитывает высоту вниз от `y`
/// (`bottom = y - height`), то есть кладёт прямоугольник на высоту ниже якоря.
/// `image.rs` по той же причине строит клип по вершинам.
fn rect_polygon(page: &PageGeometry, rect: RectPx, mode: PaintMode) -> Polygon {
    let pdf = page.rect_to_pt(rect).to_pdf(page.height_pt());
    let (left, bottom) = (pdf.x.0, pdf.y.0);
    let (right, top) = (left + pdf.width.0, bottom + pdf.height.0);
    let points = [(left, bottom), (right, bottom), (right, top), (left, top)]
        .into_iter()
        .map(|(x, y)| LinePoint {
            p: Point { x: Pt(x), y: Pt(y) },
            bezier: false,
        })
        .collect();
    Polygon {
        rings: vec![PolygonRing { points }],
        mode,
        winding_order: WindingOrder::NonZero,
    }
}

/// Контур окружности: четыре дуги по 90°, каждая — пара управляющих точек и
/// конечная.
///
/// Ловушка printpdf: кубическая Безье кодируется ровно двумя подряд идущими
/// `LinePoint { bezier: true }` плюс конечная точка
/// (`vendor/printpdf/src/serialize.rs`), одиночная управляющая молча
/// вырождается в прямую — поэтому дуги кладутся только парами.
fn circle_ring(page: &PageGeometry, cx: f64, cy: f64, r: f64) -> Vec<LinePoint> {
    let k = r * KAPPA;
    let mut ring = vec![LinePoint {
        p: to_pdf_point(page, cx + r, cy),
        bezier: false,
    }];
    push_quarter_arc(
        &mut ring,
        page,
        (cx + r, cy + k),
        (cx + k, cy + r),
        (cx, cy + r),
    );
    push_quarter_arc(
        &mut ring,
        page,
        (cx - k, cy + r),
        (cx - r, cy + k),
        (cx - r, cy),
    );
    push_quarter_arc(
        &mut ring,
        page,
        (cx - r, cy - k),
        (cx - k, cy - r),
        (cx, cy - r),
    );
    push_quarter_arc(
        &mut ring,
        page,
        (cx + k, cy - r),
        (cx + r, cy - k),
        (cx + r, cy),
    );
    ring
}

/// Добавить дугу Безье: две управляющие точки и конец.
fn push_quarter_arc(
    ring: &mut Vec<LinePoint>,
    page: &PageGeometry,
    c1: (f64, f64),
    c2: (f64, f64),
    end: (f64, f64),
) {
    for (point, bezier) in [(c1, true), (c2, true), (end, false)] {
        ring.push(LinePoint {
            p: to_pdf_point(page, point.0, point.1),
            bezier,
        });
    }
}

/// `f64`-геометрия `layout` в `f32`-единицы printpdf: сторона страницы
/// заведомо много меньше 2^24, где `f32` теряет целые пиксели.
#[allow(clippy::cast_possible_truncation)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PageConfig;
    use doc_converter_render::chart::{ChartKind, ChartSeries};
    use doc_converter_render::display_list::Color;

    /// Сверка с посчитанными вручную точками: `f32`-арифметика страницы.
    const EPS: f32 = 1e-3;

    /// Умолчания A4: поля 20 мм сверху и 15 мм слева, высота 297 мм.
    const ORIGIN_X_PT: f32 = 15.0 * 72.0 / 25.4;
    const ORIGIN_Y_PT: f32 = 20.0 * 72.0 / 25.4;
    const PAGE_H_PT: f32 = 297.0 * 72.0 / 25.4;

    fn page() -> PageGeometry {
        PageGeometry::new(&PageConfig::default())
    }

    fn font() -> PdfFontId {
        PdfFontId("F1".to_owned())
    }

    /// Пиксель раскладки в точки при масштабе 1 — независимо от `PageGeometry`.
    fn hand_pt(px: f32) -> f32 {
        px * 72.0 / 96.0
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < EPS,
            "expected {expected}, got {actual}"
        );
    }

    /// Диаграмма с одной серией: `layout` даёт непустой список примитивов.
    fn bar_data() -> ChartData {
        ChartData {
            kind: ChartKind::Bar,
            title: None,
            categories: vec!["A".to_owned()],
            series: vec![ChartSeries {
                name: String::new(),
                values: vec![3.0],
            }],
        }
    }

    /// Операции одного примитива: клип и обёртку добавляет только [`draw`].
    fn prim_ops(registry: &mut FontRegistry, prim: &ChartPrim) -> Vec<Op> {
        let mut ops = Vec::new();
        draw_prim(&mut ops, registry, &font(), &page(), prim);
        ops
    }

    #[test]
    fn empty_chart_emits_nothing() {
        let mut registry = FontRegistry::new(16);
        let mut ops = Vec::new();
        draw(
            &mut ops,
            &mut registry,
            &font(),
            &page(),
            RectPx::new(10.0, 10.0, 200.0, 100.0),
            &ChartData::default(),
        );
        assert!(ops.is_empty(), "без серий не должно быть ни одной операции");
    }

    #[test]
    fn zero_rect_emits_nothing() {
        let mut registry = FontRegistry::new(16);
        let mut ops = Vec::new();
        draw(
            &mut ops,
            &mut registry,
            &font(),
            &page(),
            RectPx::new(0.0, 0.0, 0.0, 100.0),
            &bar_data(),
        );
        assert!(
            ops.is_empty(),
            "нулевой якорь не рисует даже save/clip: layout вернул пусто"
        );
    }

    #[test]
    fn clip_is_wrapped_in_save_and_restore() {
        let mut registry = FontRegistry::new(16);
        let mut ops = Vec::new();
        draw(
            &mut ops,
            &mut registry,
            &font(),
            &page(),
            RectPx::new(10.0, 20.0, 400.0, 200.0),
            &bar_data(),
        );

        assert_eq!(ops.first(), Some(&Op::SaveGraphicsState));
        assert_eq!(ops.last(), Some(&Op::RestoreGraphicsState));
        assert!(ops.len() > 3, "кроме клипа должны быть и примитивы");

        let Some(Op::DrawPolygon { polygon }) = ops.get(1) else {
            panic!("после save должен идти клип");
        };
        assert_eq!(polygon.mode, PaintMode::Clip);
        assert_eq!(polygon.rings.len(), 1);
        let points = &polygon.rings[0].points;
        assert_eq!(points.len(), 4);
        // Левый нижний угол якоря (10, 20) px с высотой 200 px.
        close(points[0].p.x.0, ORIGIN_X_PT + hand_pt(10.0));
        close(
            points[0].p.y.0,
            PAGE_H_PT - ORIGIN_Y_PT - hand_pt(20.0) - hand_pt(200.0),
        );
        close(points[2].p.x.0, ORIGIN_X_PT + hand_pt(410.0));
        close(points[2].p.y.0, PAGE_H_PT - ORIGIN_Y_PT - hand_pt(20.0));
    }

    #[test]
    fn rect_fills_polygon_in_points() {
        let mut registry = FontRegistry::new(16);
        let color = Color::rgba(0x11, 0x22, 0x33, 0xFF);
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Rect {
                x: 96.0,
                y: 192.0,
                w: 48.0,
                h: 24.0,
                fill: color,
            },
        );

        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[0],
            Op::SetFillColor {
                col: to_pdf_color(color)
            }
        );
        let Some(Op::DrawPolygon { polygon }) = ops.get(1) else {
            panic!("ожидалась заливка полигоном");
        };
        assert_eq!(polygon.mode, PaintMode::Fill);
        let points = &polygon.rings[0].points;
        assert_eq!(points.len(), 4);
        // (96, 192) px, 48x24 px: нижний и верхний края прямоугольника.
        close(points[0].p.x.0, ORIGIN_X_PT + hand_pt(96.0));
        close(
            points[0].p.y.0,
            PAGE_H_PT - ORIGIN_Y_PT - hand_pt(192.0) - hand_pt(24.0),
        );
        close(points[2].p.y.0, PAGE_H_PT - ORIGIN_Y_PT - hand_pt(192.0));
    }

    #[test]
    fn polyline_strokes_with_width_and_color() {
        let mut registry = FontRegistry::new(16);
        let color = Color::rgba(0xAA, 0xBB, 0xCC, 0xFF);
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Polyline {
                points: vec![(0.0, 0.0), (96.0, 96.0)],
                stroke: color,
                width: 1.5,
                closed: true,
            },
        );

        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0], Op::SetOutlineThickness { pt: Pt(1.125) });
        assert_eq!(
            ops[1],
            Op::SetOutlineColor {
                col: to_pdf_color(color)
            }
        );
        let Some(Op::DrawLine { line }) = ops.get(2) else {
            panic!("ожидалась обводка линии");
        };
        assert!(line.is_closed);
        assert_eq!(line.points.len(), 2);
        close(line.points[0].p.x.0, ORIGIN_X_PT);
        close(line.points[0].p.y.0, PAGE_H_PT - ORIGIN_Y_PT);
        close(line.points[1].p.x.0, ORIGIN_X_PT + hand_pt(96.0));
        close(
            line.points[1].p.y.0,
            PAGE_H_PT - ORIGIN_Y_PT - hand_pt(96.0),
        );
    }

    #[test]
    fn polygon_fills_ring() {
        let mut registry = FontRegistry::new(16);
        let color = Color::BLACK;
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Polygon {
                points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
                fill: color,
            },
        );

        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[0],
            Op::SetFillColor {
                col: to_pdf_color(color)
            }
        );
        let Some(Op::DrawPolygon { polygon }) = ops.get(1) else {
            panic!("ожидалась заливка полигоном");
        };
        assert_eq!(polygon.mode, PaintMode::Fill);
        assert_eq!(polygon.rings[0].points.len(), 3);
    }

    #[test]
    fn circle_is_bezier_arcs_not_lines() {
        let mut registry = FontRegistry::new(16);
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Circle {
                cx: 50.0,
                cy: 50.0,
                r: 10.0,
                fill: Color::BLACK,
            },
        );

        assert_eq!(ops.len(), 2);
        let Some(Op::DrawPolygon { polygon }) = ops.get(1) else {
            panic!("ожидалась заливка полигоном");
        };
        let ring = &polygon.rings[0].points;
        assert_eq!(
            ring.len(),
            13,
            "4 дуги: начало + 4 x (2 управляющих + конец)"
        );
        // Ловушка printpdf: управляющие Безье обязаны идти парами, иначе дуга
        // молча вырождается в прямую.
        let flags: Vec<bool> = ring.iter().map(|point| point.bezier).collect();
        assert_eq!(
            flags,
            vec![false, true, true, false, true, true, false, true, true, false, true, true, false]
        );
        close(ring[0].p.x.0, ORIGIN_X_PT + hand_pt(60.0));
        close(ring[0].p.y.0, PAGE_H_PT - ORIGIN_Y_PT - hand_pt(50.0));
        // Контур замкнут: конечная точка последней дуги — начало первой.
        close(ring[12].p.x.0, ring[0].p.x.0);
        close(ring[12].p.y.0, ring[0].p.y.0);
    }

    #[test]
    fn sector_is_skipped_until_c4() {
        let mut registry = FontRegistry::new(16);
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Sector {
                cx: 50.0,
                cy: 50.0,
                r: 10.0,
                from: 0.0,
                to: std::f64::consts::FRAC_PI_2,
                fill: Color::BLACK,
            },
        );
        assert!(ops.is_empty());
    }

    #[test]
    fn text_writes_section_at_baseline() {
        let mut registry = FontRegistry::new(16);
        let color = Color::rgba(0x01, 0x02, 0x03, 0xFF);
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Text {
                x: 96.0,
                y: 100.0,
                text: "Hi".to_owned(),
                size: 12.0,
                align: TextAlign::Left,
                fill: color,
            },
        );

        assert_eq!(ops.len(), 6);
        assert_eq!(
            ops[0],
            Op::SetFillColor {
                col: to_pdf_color(color)
            }
        );
        assert_eq!(ops[1], Op::StartTextSection);
        let Some(Op::SetTextCursor { pos }) = ops.get(2) else {
            panic!("ожидался курсор");
        };
        // `y` — базовая линия: она же координата курсора, без подъёма на ascent.
        close(pos.x.0, ORIGIN_X_PT + hand_pt(96.0));
        close(pos.y.0, PAGE_H_PT - ORIGIN_Y_PT - hand_pt(100.0));
        assert_eq!(
            ops[3],
            Op::SetFontSize {
                size: Pt(9.0),
                font: font()
            }
        );
        let Some(Op::WriteText { items, .. }) = ops.get(4) else {
            panic!("ожидалась запись текста");
        };
        assert_eq!(items, &vec![TextItem::Text("Hi".to_owned())]);
        assert_eq!(ops[5], Op::EndTextSection);
    }

    #[test]
    fn empty_text_emits_nothing() {
        let mut registry = FontRegistry::new(16);
        let ops = prim_ops(
            &mut registry,
            &ChartPrim::Text {
                x: 0.0,
                y: 0.0,
                text: String::new(),
                size: 12.0,
                align: TextAlign::Center,
                fill: Color::BLACK,
            },
        );
        assert!(ops.is_empty());
    }

    /// Центрирование и правое выравнивание сдвигают курсор влево на половину
    /// и целую ширину строки; ширину даёт тот же `FontRegistry`, что у текста.
    #[test]
    fn center_and_right_shift_cursor_left() {
        let mut registry = FontRegistry::new(16);
        let text = "Hello";
        let size = 12.0_f64;
        let width_px = registry.measure(DEFAULT_FONT_ID, to_f32(size), text).width;

        let mut cursor = |align: TextAlign| {
            let ops = prim_ops(
                &mut registry,
                &ChartPrim::Text {
                    x: 96.0,
                    y: 50.0,
                    text: text.to_owned(),
                    size,
                    align,
                    fill: Color::BLACK,
                },
            );
            let Some(Op::SetTextCursor { pos }) = ops.get(2) else {
                panic!("ожидался курсор");
            };
            pos.x.0
        };

        let left = cursor(TextAlign::Left);
        let center = cursor(TextAlign::Center);
        let right = cursor(TextAlign::Right);
        assert!(center < left && right < center, "сдвиг обязан быть влево");
        close(center, ORIGIN_X_PT + hand_pt(96.0 - width_px / 2.0));
        close(right, ORIGIN_X_PT + hand_pt(96.0 - width_px));
    }

    /// Метрики линейны по кеглю: ширина при вдвое большем размере вдвое
    /// больше — на этом стоит перевод ширины в пиксели диаграммы.
    #[test]
    fn measure_is_linear_in_size() {
        let mut registry = FontRegistry::new(16);
        let small = registry.measure(DEFAULT_FONT_ID, 10.0, "Hello").width;
        let large = registry.measure(DEFAULT_FONT_ID, 20.0, "Hello").width;
        close(large, small * 2.0);
    }
}
