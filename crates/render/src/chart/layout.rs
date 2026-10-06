//! Раскладка диаграммы в примитивы (ADR-0011).
//!
//! Вход — данные [`ChartData`] и прямоугольник в единицах бэкенда, выход —
//! примитивы в порядке рисования. Причуды canvas-пути переносятся дословно
//! (ADR-0011, п. 4): отрицательные столбцы уходят вниз от нижней границы
//! построения, легенда круговой — по категориям, доли ≤ 0 пропускаются, цвета
//! циклически из палитры в восемь записей. Паритет с экраном важнее починки:
//! PDF обязан повторить то же самое.

use crate::chart::prim::{ChartPrim, TextAlign};
use crate::chart::{ChartData, ChartKind};
use crate::display_list::Color;
use crate::geometry::Rect;

/// Палитра серий — как в Excel.
const PALETTE: [Color; 8] = [
    Color::rgba(0x4E, 0x79, 0xA7, 0xFF),
    Color::rgba(0xF2, 0x8E, 0x2B, 0xFF),
    Color::rgba(0xE1, 0x57, 0x59, 0xFF),
    Color::rgba(0x76, 0xB7, 0xB2, 0xFF),
    Color::rgba(0x59, 0xA1, 0x4F, 0xFF),
    Color::rgba(0xED, 0xC9, 0x48, 0xFF),
    Color::rgba(0xB0, 0x7A, 0xA1, 0xFF),
    Color::rgba(0x9C, 0x75, 0x5F, 0xFF),
];

/// Высота полосы заголовка и легенды, отступы под подписи осей.
const TITLE_H: f64 = 18.0;
const LEGEND_H: f64 = 18.0;
const VALUE_MARGIN: f64 = 30.0;
const CATEGORY_MARGIN: f64 = 20.0;

/// Кегль заголовка и подписей. Шрифты не унифицированы с PDF (ADR-0011, п. 3):
/// canvas рисует `Npx sans-serif`, PDF — общий `FontRegistry`.
const TITLE_SIZE: f64 = 12.0;
const LABEL_SIZE: f64 = 10.0;

const TITLE_COLOR: Color = Color::rgba(0x00, 0x00, 0x00, 0xFF);
const LABEL_COLOR: Color = Color::rgba(0x44, 0x44, 0x44, 0xFF);
/// Цвет сетки и рамки области построения.
const GRID_COLOR: Color = Color::rgba(0xCC, 0xCC, 0xCC, 0xFF);

/// Толщина линий сетки и серий, радиус маркера точечной диаграммы.
const GRID_W: f64 = 1.0;
const LINE_W: f64 = 1.5;
const MARKER_R: f64 = 2.5;

/// Прямоугольник построения: `(x, y, w, h)`.
type Plot = (f64, f64, f64, f64);

/// Раскладывает диаграмму в примитивы; числа — в единицах `rect`.
///
/// Пустой список — рисовать нечего: нулевой прямоугольник или ни одной серии.
#[must_use]
pub fn layout(rect: Rect, data: &ChartData) -> Vec<ChartPrim> {
    if rect.w <= 0.0 || rect.h <= 0.0 || data.series.is_empty() {
        return Vec::new();
    }
    let (x, y, w, h) = (
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.w),
        f64::from(rect.h),
    );

    let title_h = if data.title.is_some() { TITLE_H } else { 0.0 };
    let legend_h = if has_legend(data) { LEGEND_H } else { 0.0 };

    let mut prims = Vec::new();
    if let Some(title) = &data.title {
        prims.push(ChartPrim::Text {
            x: x + w / 2.0,
            y: y + 14.0,
            text: title.clone(),
            size: TITLE_SIZE,
            align: TextAlign::Center,
            fill: TITLE_COLOR,
        });
    }

    let plot = (
        x + VALUE_MARGIN,
        y + title_h,
        (w - VALUE_MARGIN - 8.0).max(0.0),
        (h - title_h - legend_h - CATEGORY_MARGIN).max(0.0),
    );

    match data.kind {
        ChartKind::Pie => layout_pie(&mut prims, plot, data),
        ChartKind::Bar => {
            let range = layout_axes(&mut prims, (x, y, w, h), plot, data, true);
            layout_bars(&mut prims, plot, data, range);
        }
        ChartKind::Line => {
            let range = layout_axes(&mut prims, (x, y, w, h), plot, data, false);
            layout_lines(&mut prims, plot, data, range);
        }
        ChartKind::Scatter => {
            let range = layout_axes(&mut prims, (x, y, w, h), plot, data, false);
            layout_scatter(&mut prims, plot, data, range);
        }
        ChartKind::Area => {
            let range = layout_axes(&mut prims, (x, y, w, h), plot, data, true);
            layout_areas(&mut prims, plot, data, range);
        }
    }
    layout_legend(&mut prims, x, y + h - legend_h, w, legend_h, data);
    prims
}

fn has_legend(data: &ChartData) -> bool {
    match data.kind {
        ChartKind::Pie => !data.categories.is_empty(),
        _ => data.series.iter().any(|series| !series.name.is_empty()),
    }
}

/// Кладёт рамку, сетку и подписи осей; возвращает размах значений `max - min`.
fn layout_axes(
    prims: &mut Vec<ChartPrim>,
    chart: Plot,
    plot: Plot,
    data: &ChartData,
    include_zero: bool,
) -> f64 {
    let (px, py, pw, ph) = plot;
    let (vmin, vmax) = value_range(data, include_zero);
    let categories = max_category_count(data);

    // Canvas-путь один раз штрихует текущий путь после `clip()` — а это и есть
    // прямоугольник диаграммы. Повторяем явной замкнутой ломаной: без неё
    // рамка исчезнет, и экран разойдётся с прежним видом.
    let (cx, cy, cw, ch) = chart;
    prims.push(ChartPrim::Polyline {
        points: vec![(cx, cy), (cx + cw, cy), (cx + cw, cy + ch), (cx, cy + ch)],
        stroke: GRID_COLOR,
        width: GRID_W,
        closed: true,
    });

    for tick in 0..=4 {
        let fraction = tick as f64 / 4.0;
        let value = vmin + (vmax - vmin) * fraction;
        let line_y = py + ph - ph * fraction;
        prims.push(ChartPrim::Polyline {
            points: vec![(px, line_y), (px + pw, line_y)],
            stroke: GRID_COLOR,
            width: GRID_W,
            closed: false,
        });
        prims.push(label(
            px - 4.0,
            line_y + 3.0,
            fmt_tick(value),
            TextAlign::Right,
        ));
    }

    if categories > 1 {
        let step = (categories as f64 / 6.0).ceil().max(1.0) as usize;
        for index in (0..categories).step_by(step) {
            let label_x = px + (index as f64 + 0.5) * pw / categories as f64;
            let text = data
                .categories
                .get(index)
                .map_or_else(|| index.to_string(), String::clone);
            prims.push(label(label_x, py + ph + 14.0, text, TextAlign::Center));
        }
    }

    if (vmax - vmin).abs() < f64::EPSILON {
        1.0
    } else {
        vmax - vmin
    }
}

/// Диапазон значений: столбцы и области всегда включают ноль.
fn value_range(data: &ChartData, include_zero: bool) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for series in &data.series {
        for value in &series.values {
            let value = f64::from(*value);
            min = min.min(value);
            max = max.max(value);
        }
    }
    if include_zero {
        min = min.min(0.0);
        max = max.max(0.0);
    }
    if !min.is_finite() || !max.is_finite() {
        return (0.0, 1.0);
    }
    if (max - min).abs() < f64::EPSILON {
        return (min - 0.5, max + 0.5);
    }
    (min, max)
}

fn layout_bars(prims: &mut Vec<ChartPrim>, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let categories = max_category_count(data);
    if categories == 0 {
        return;
    }
    let slot = pw / categories as f64;
    let bar_w = slot * 0.7 / data.series.len().max(1) as f64;
    let baseline = py + ph;
    for (series_index, series) in data.series.iter().enumerate() {
        let color = PALETTE[series_index % PALETTE.len()];
        for (index, value) in series.values.iter().enumerate() {
            let value = f64::from(*value);
            let height = value / range * ph;
            let x = px + index as f64 * slot + slot * 0.15 + series_index as f64 * bar_w;
            let y = if value >= 0.0 {
                baseline - height
            } else {
                baseline
            };
            // Нулевая высота не рисуется (canvas-путь её пропускал), зато
            // отрицательная уходит вниз от базовой линии — известный дефект,
            // закреплённый ADR-0011 как поведение обоих бэкендов.
            let bar_h = height.abs();
            if bar_w > 0.0 && bar_h > 0.0 {
                prims.push(ChartPrim::Rect {
                    x,
                    y,
                    w: bar_w,
                    h: bar_h,
                    fill: color,
                });
            }
        }
    }
}

fn layout_lines(prims: &mut Vec<ChartPrim>, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let categories = max_category_count(data).max(1);
    for (series_index, series) in data.series.iter().enumerate() {
        let color = PALETTE[series_index % PALETTE.len()];
        let points: Vec<(f64, f64)> = series
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                (
                    px + (index as f64 + 0.5) * pw / categories as f64,
                    py + ph - f64::from(*value) / range * ph,
                )
            })
            .collect();
        // Ломаная из одной точки — пустой путь: canvas-обводка ничего не
        // рисовала, и примитив рисовать нечего.
        if points.len() < 2 {
            continue;
        }
        prims.push(ChartPrim::Polyline {
            points,
            stroke: color,
            width: LINE_W,
            closed: false,
        });
    }
}

fn layout_scatter(prims: &mut Vec<ChartPrim>, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let categories = max_category_count(data).max(1);
    for (series_index, series) in data.series.iter().enumerate() {
        let color = PALETTE[series_index % PALETTE.len()];
        for (index, value) in series.values.iter().enumerate() {
            let cx = px + (index as f64 + 0.5) * pw / categories as f64;
            let cy = py + ph - f64::from(*value) / range * ph;
            prims.push(ChartPrim::Circle {
                cx,
                cy,
                r: MARKER_R,
                fill: color,
            });
        }
    }
}

fn layout_areas(prims: &mut Vec<ChartPrim>, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let baseline = py + ph;
    let categories = max_category_count(data).max(1);
    for (series_index, series) in data.series.iter().enumerate() {
        let color = PALETTE[series_index % PALETTE.len()];
        let mut points = vec![(px, baseline)];
        for (index, value) in series.values.iter().enumerate() {
            points.push((
                px + (index as f64 + 0.5) * pw / categories as f64,
                py + ph - f64::from(*value) / range * ph,
            ));
        }
        points.push((px + pw, baseline));
        prims.push(ChartPrim::Polygon {
            points,
            fill: color,
        });
    }
}

fn layout_pie(prims: &mut Vec<ChartPrim>, plot: Plot, data: &ChartData) {
    let Some(series) = data.series.first() else {
        return;
    };
    let total: f64 = series
        .values
        .iter()
        .map(|value| f64::from(*value))
        .filter(|value| *value > 0.0)
        .sum();
    if total <= 0.0 {
        return;
    }
    let (px, py, pw, ph) = plot;
    let radius = pw.min(ph) / 2.0 - 2.0;
    let cx = px + pw / 2.0;
    let cy = py + ph / 2.0;
    let mut angle = -std::f64::consts::FRAC_PI_2;
    for (index, value) in series.values.iter().enumerate() {
        let value = f64::from(*value);
        // Доли ≤ 0 пропускаются, но номер цвета не сдвигают: палитра — по
        // индексу исходного значения (canvas-путь, ADR-0011 п. 4).
        if value <= 0.0 {
            continue;
        }
        let end = angle + value / total * std::f64::consts::TAU;
        prims.push(ChartPrim::Sector {
            cx,
            cy,
            r: radius,
            from: angle,
            to: end,
            fill: PALETTE[index % PALETTE.len()],
        });
        angle = end;
    }
}

fn layout_legend(prims: &mut Vec<ChartPrim>, x: f64, y: f64, w: f64, h: f64, data: &ChartData) {
    if !has_legend(data) {
        return;
    }
    let entries: Vec<&str> = match data.kind {
        ChartKind::Pie => data.categories.iter().map(String::as_str).collect(),
        _ => data
            .series
            .iter()
            .map(|series| series.name.as_str())
            .collect(),
    };
    let slot = w / entries.len().max(1) as f64;
    for (index, name) in entries.iter().enumerate() {
        let color = PALETTE[index % PALETTE.len()];
        prims.push(ChartPrim::Rect {
            x: x + 8.0 + index as f64 * slot,
            y: y + (h - 8.0) / 2.0,
            w: 8.0,
            h: 8.0,
            fill: color,
        });
        prims.push(label(
            x + 20.0 + index as f64 * slot,
            y + h - 5.0,
            *name,
            TextAlign::Left,
        ));
    }
}

fn max_category_count(data: &ChartData) -> usize {
    data.series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(0)
}

fn label(x: f64, y: f64, text: impl Into<String>, align: TextAlign) -> ChartPrim {
    ChartPrim::Text {
        x,
        y,
        text: text.into(),
        size: LABEL_SIZE,
        align,
        fill: LABEL_COLOR,
    }
}

/// Подпись деления: не больше двух знаков после запятой, без хвостовых нулей.
fn fmt_tick(value: f64) -> String {
    if value.abs() < 1e-9 {
        return "0".to_owned();
    }
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::ChartSeries;

    /// Прямоугольник 400×300 в начале координат: числа тестов считаются от него.
    const RECT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 400.0,
        h: 300.0,
    };

    fn chart(kind: ChartKind, series: &[(&str, &[f32])]) -> ChartData {
        ChartData {
            kind,
            title: None,
            categories: Vec::new(),
            series: series
                .iter()
                .map(|(name, values)| ChartSeries {
                    name: (*name).to_owned(),
                    values: values.to_vec(),
                })
                .collect(),
        }
    }

    fn with_categories(mut data: ChartData, categories: &[&str]) -> ChartData {
        data.categories = categories.iter().map(|c| (*c).to_owned()).collect();
        data
    }

    fn count(prims: &[ChartPrim], pred: impl Fn(&ChartPrim) -> bool) -> usize {
        prims.iter().filter(|p| pred(p)).count()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "ожидалось {expected}, получено {actual}"
        );
    }

    #[test]
    fn no_series_lays_out_nothing() {
        let mut data = chart(ChartKind::Bar, &[]);
        data.title = Some("Пусто".to_owned());
        assert!(layout(RECT, &data).is_empty());
    }

    #[test]
    fn zero_rect_lays_out_nothing() {
        let data = chart(ChartKind::Bar, &[("", &[1.0])]);
        let flat = Rect { w: 0.0, ..RECT };
        assert!(layout(flat, &data).is_empty());
        let flat = Rect { h: 0.0, ..RECT };
        assert!(layout(flat, &data).is_empty());
    }

    #[test]
    fn title_is_the_first_prim_at_the_top_center() {
        let mut data = chart(ChartKind::Line, &[("", &[1.0, 2.0])]);
        data.title = Some("Продажи".to_owned());
        let prims = layout(RECT, &data);
        match &prims[0] {
            ChartPrim::Text {
                x,
                y,
                text,
                size,
                align,
                fill,
            } => {
                assert_close(*x, 200.0);
                assert_close(*y, 14.0);
                assert_eq!(text, "Продажи");
                assert_close(*size, 12.0);
                assert_eq!(*align, TextAlign::Center);
                assert_eq!(*fill, TITLE_COLOR);
            }
            other => panic!("ожидался заголовок, получено {other:?}"),
        }
    }

    #[test]
    fn bar_lays_out_border_grid_labels_and_columns() {
        let data = with_categories(chart(ChartKind::Bar, &[("", &[1.0, 2.0])]), &["A", "B"]);
        let prims = layout(RECT, &data);
        // рамка + 5 линий сетки + 5 подписей делений + 2 подписи категорий
        // + 2 столбца
        assert_eq!(prims.len(), 15);

        match &prims[0] {
            ChartPrim::Polyline {
                points,
                stroke,
                width,
                closed,
            } => {
                assert_eq!(
                    *points,
                    vec![(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)]
                );
                assert_eq!(*stroke, GRID_COLOR);
                assert_close(*width, 1.0);
                assert!(*closed);
            }
            other => panic!("ожидалась рамка, получено {other:?}"),
        }

        // Область построения: x = 30, w = 400 - 30 - 8 = 362, h = 300 - 20 = 280,
        // размах 0..2, столбец на категорию шириной 181 * 0.7.
        let bars: Vec<&ChartPrim> = prims
            .iter()
            .filter(|p| matches!(p, ChartPrim::Rect { .. }))
            .collect();
        assert_eq!(bars.len(), 2);
        match bars[0] {
            ChartPrim::Rect { x, y, w, h, fill } => {
                assert_close(*x, 30.0 + 181.0 * 0.15);
                assert_close(*y, 140.0);
                assert_close(*w, 181.0 * 0.7);
                assert_close(*h, 140.0);
                assert_eq!(*fill, PALETTE[0]);
            }
            other => panic!("ожидался столбец, получено {other:?}"),
        }
    }

    #[test]
    fn negative_bar_falls_below_the_plot_baseline() {
        let data = chart(ChartKind::Bar, &[("", &[-2.0, 2.0])]);
        let prims = layout(RECT, &data);
        // Размах -2..2, столбцы по |height| = 140; отрицательный — вниз от
        // нижней границы построения (y = 280), это закреплённая причуда.
        let bars: Vec<&ChartPrim> = prims
            .iter()
            .filter(|p| matches!(p, ChartPrim::Rect { .. }))
            .collect();
        assert_eq!(bars.len(), 2);
        match bars[0] {
            ChartPrim::Rect { y, h, .. } => {
                assert_close(*y, 280.0);
                assert_close(*h, 140.0);
            }
            other => panic!("ожидался столбец, получено {other:?}"),
        }
        match bars[1] {
            ChartPrim::Rect { y, h, .. } => {
                assert_close(*y, 140.0);
                assert_close(*h, 140.0);
            }
            other => panic!("ожидался столбец, получено {other:?}"),
        }
    }

    #[test]
    fn zero_range_expands_and_zero_columns_vanish() {
        let data = chart(ChartKind::Bar, &[("", &[0.0, 0.0])]);
        let prims = layout(RECT, &data);
        // Вырожденный размах расширяется до [-0.5, 0.5] — деления считаются,
        // а столбцы нулевой высоты не рисуются.
        assert_eq!(count(&prims, |p| matches!(p, ChartPrim::Rect { .. })), 0);
        let tick = prims.iter().find_map(|p| match p {
            ChartPrim::Text {
                text,
                align: TextAlign::Right,
                ..
            } => Some(text.as_str()),
            _ => None,
        });
        assert_eq!(tick, Some("-0.5"));
    }

    #[test]
    fn line_lays_out_a_polyline_through_slot_centers() {
        let data = chart(ChartKind::Line, &[("", &[0.0, 5.0])]);
        let prims = layout(RECT, &data);
        let line = prims
            .iter()
            .find_map(|p| match p {
                ChartPrim::Polyline {
                    points,
                    stroke,
                    width,
                    closed: false,
                } if *stroke == PALETTE[0] => Some((points, width)),
                _ => None,
            })
            .expect("ломаная серии");
        assert_close(line.0[0].0, 30.0 + 0.5 * 362.0 / 2.0);
        assert_close(line.0[0].1, 280.0);
        assert_close(line.0[1].0, 30.0 + 1.5 * 362.0 / 2.0);
        assert_close(line.0[1].1, 0.0);
        assert_close(*line.1, 1.5);
    }

    #[test]
    fn single_point_line_draws_no_polyline() {
        let data = chart(ChartKind::Line, &[("", &[3.0])]);
        let prims = layout(RECT, &data);
        assert_eq!(
            count(
                &prims,
                |p| matches!(p, ChartPrim::Polyline { stroke, .. } if *stroke == PALETTE[0])
            ),
            0
        );
        // Оси и рамка на месте: кадр не пустой.
        assert!(prims.len() > 1);
    }

    #[test]
    fn single_point_bar_draws_one_column() {
        let data = chart(ChartKind::Bar, &[("", &[3.0])]);
        let prims = layout(RECT, &data);
        assert_eq!(count(&prims, |p| matches!(p, ChartPrim::Rect { .. })), 1);
    }

    #[test]
    fn scatter_lays_out_marker_circles() {
        let data = chart(ChartKind::Scatter, &[("", &[1.0, 2.0, 3.0])]);
        let prims = layout(RECT, &data);
        let markers: Vec<&ChartPrim> = prims
            .iter()
            .filter(|p| matches!(p, ChartPrim::Circle { .. }))
            .collect();
        assert_eq!(markers.len(), 3);
        match markers[0] {
            ChartPrim::Circle { cx, cy, r, fill } => {
                assert_close(*cx, 30.0 + 0.5 * 362.0 / 3.0);
                assert_close(*cy, 140.0);
                assert_close(*r, 2.5);
                assert_eq!(*fill, PALETTE[0]);
            }
            other => panic!("ожидался маркер, получено {other:?}"),
        }
    }

    #[test]
    fn area_closes_the_polygon_on_the_baseline() {
        let data = chart(ChartKind::Area, &[("", &[1.0, 2.0])]);
        let prims = layout(RECT, &data);
        let area = prims
            .iter()
            .find_map(|p| match p {
                ChartPrim::Polygon { points, fill } if *fill == PALETTE[0] => Some(points),
                _ => None,
            })
            .expect("полигон серии");
        assert_eq!(area.len(), 4);
        assert_eq!(area[0], (30.0, 280.0));
        assert_close(area[3].0, 30.0 + 362.0);
        assert_eq!(area[3].1, 280.0);
        assert_close(area[1].1, 140.0);
    }

    #[test]
    fn pie_sectors_cover_the_full_turn() {
        let data = with_categories(
            chart(ChartKind::Pie, &[("", &[1.0, 1.0, 2.0])]),
            &["a", "b", "c"],
        );
        let prims = layout(RECT, &data);
        let sectors: Vec<&ChartPrim> = prims
            .iter()
            .filter(|p| matches!(p, ChartPrim::Sector { .. }))
            .collect();
        assert_eq!(sectors.len(), 3);
        let sum: f64 = sectors
            .iter()
            .map(|s| match s {
                ChartPrim::Sector { from, to, .. } => to - from,
                _ => unreachable!(),
            })
            .sum();
        assert!(
            (sum - std::f64::consts::TAU).abs() < 1e-3,
            "сумма углов {sum} не равна 2π"
        );
        match sectors[0] {
            ChartPrim::Sector { from, .. } => assert_close(*from, -std::f64::consts::FRAC_PI_2),
            other => panic!("ожидался сектор, получено {other:?}"),
        }
        // Легенда круговой — по категориям: три образца и три подписи.
        assert_eq!(count(&prims, |p| matches!(p, ChartPrim::Rect { .. })), 3);
        assert_eq!(count(&prims, |p| matches!(p, ChartPrim::Text { .. })), 3);
    }

    #[test]
    fn pie_skips_non_positive_shares() {
        let data = chart(ChartKind::Pie, &[("", &[2.0, 0.0, -1.0])]);
        let prims = layout(RECT, &data);
        let sectors: Vec<&ChartPrim> = prims
            .iter()
            .filter(|p| matches!(p, ChartPrim::Sector { .. }))
            .collect();
        assert_eq!(sectors.len(), 1);
        match sectors[0] {
            ChartPrim::Sector { from, to, .. } => {
                assert_close(*from, -std::f64::consts::FRAC_PI_2);
                assert_close(*to, -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU);
            }
            other => panic!("ожидался сектор, получено {other:?}"),
        }
    }

    #[test]
    fn pie_without_positive_shares_lays_out_nothing() {
        let data = chart(ChartKind::Pie, &[("", &[-1.0, 0.0])]);
        assert!(layout(RECT, &data).is_empty());
    }

    #[test]
    fn cartesian_legend_lists_series_names() {
        let data = chart(
            ChartKind::Bar,
            &[("План", &[1.0, 2.0]), ("Факт", &[2.0, 1.0])],
        );
        let prims = layout(RECT, &data);
        let names: Vec<&str> = prims
            .iter()
            .filter_map(|p| match p {
                ChartPrim::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"План"));
        assert!(names.contains(&"Факт"));
        // Два столбца на серию (все высоты ненулевые) плюс два образца легенды.
        assert_eq!(count(&prims, |p| matches!(p, ChartPrim::Rect { .. })), 6);
    }
}
