//! Рисование диаграмм из блоба `DrawCommand::Chart`.
//!
//! Painter — единственный, кто умеет диаграммы: в кадре лежат только данные
//! ([`crate::chart::ChartData`]) и прямоугольник, а примитивы (столбцы, линии,
//! дуги) строит сам 2D-контекст. Так круговая диаграмма остаётся выразимой —
//! среди команд кадра нет ни дуг, ни заливок по контуру.

use crate::chart::{ChartData, ChartKind};
use crate::display_list::Color;
use web_sys::OffscreenCanvasRenderingContext2d;

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

/// Высота заголовка и легенды, отступы под подписи осей.
const TITLE_H: f64 = 18.0;
const LEGEND_H: f64 = 18.0;
const VALUE_MARGIN: f64 = 30.0;
const CATEGORY_MARGIN: f64 = 20.0;

/// Прямоугольник построения: `(x, y, w, h)`.
type Plot = (f64, f64, f64, f64);

/// Нарисовать диаграмму в прямоугольнике. Битые данные painter пропускает.
pub fn paint(
    ctx: &OffscreenCanvasRenderingContext2d,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    data: &ChartData,
) {
    if w <= 0.0 || h <= 0.0 || data.series.is_empty() {
        return;
    }
    let x = f64::from(x);
    let y = f64::from(y);
    let w = f64::from(w);
    let h = f64::from(h);

    let title_h = if data.title.is_some() { TITLE_H } else { 0.0 };
    let legend_h = if has_legend(data) { LEGEND_H } else { 0.0 };

    ctx.save();
    ctx.begin_path();
    ctx.rect(x, y, w, h);
    ctx.clip();

    if let Some(title) = &data.title {
        text(
            ctx,
            x + w / 2.0,
            y + 14.0,
            title,
            "12px sans-serif",
            "#000000",
            "center",
        );
    }

    let plot = (
        x + VALUE_MARGIN,
        y + title_h,
        (w - VALUE_MARGIN - 8.0).max(0.0),
        (h - title_h - legend_h - CATEGORY_MARGIN).max(0.0),
    );

    match data.kind {
        ChartKind::Pie => paint_pie(ctx, plot, data),
        ChartKind::Bar => {
            let range = paint_axes(ctx, plot, data, true);
            paint_bars(ctx, plot, data, range);
        }
        ChartKind::Line => {
            let range = paint_axes(ctx, plot, data, false);
            paint_lines(ctx, plot, data, range);
        }
        ChartKind::Scatter => {
            let range = paint_axes(ctx, plot, data, false);
            paint_scatter(ctx, plot, data, range);
        }
        ChartKind::Area => {
            let range = paint_axes(ctx, plot, data, true);
            paint_areas(ctx, plot, data, range);
        }
    }
    paint_legend(ctx, x, y + h - legend_h, w, legend_h, data);
    ctx.restore();
}

fn has_legend(data: &ChartData) -> bool {
    match data.kind {
        ChartKind::Pie => !data.categories.is_empty(),
        _ => data.series.iter().any(|series| !series.name.is_empty()),
    }
}

/// Рисует сетку и подписи осей, возвращает размах значений `max - min`.
fn paint_axes(
    ctx: &OffscreenCanvasRenderingContext2d,
    plot: Plot,
    data: &ChartData,
    include_zero: bool,
) -> f64 {
    let (px, py, pw, ph) = plot;
    let (vmin, vmax) = value_range(data, include_zero);
    let categories = data
        .series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(0);

    stroke(ctx, Color::rgba(0xCC, 0xCC, 0xCC, 0xFF), 1.0);
    for tick in 0..=4 {
        let fraction = tick as f64 / 4.0;
        let value = vmin + (vmax - vmin) * fraction;
        let line_y = py + ph - ph * fraction;
        ctx.begin_path();
        ctx.move_to(px, line_y);
        ctx.line_to(px + pw, line_y);
        ctx.stroke();
        text(
            ctx,
            px - 4.0,
            line_y + 3.0,
            &fmt_tick(value),
            "10px sans-serif",
            "#444444",
            "right",
        );
    }

    if categories > 1 {
        let step = (categories as f64 / 6.0).ceil().max(1.0) as usize;
        for index in (0..categories).step_by(step) {
            let cx = px + (index as f64 + 0.5) * pw / categories as f64;
            let label = data
                .categories
                .get(index)
                .map_or_else(|| index.to_string(), String::clone);
            text(
                ctx,
                cx,
                py + ph + 14.0,
                &label,
                "10px sans-serif",
                "#444444",
                "center",
            );
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

fn paint_bars(ctx: &OffscreenCanvasRenderingContext2d, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let categories = data
        .series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(0);
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
            fill(ctx, x, y, bar_w, height.abs(), color);
        }
    }
}

fn paint_lines(ctx: &OffscreenCanvasRenderingContext2d, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let categories = data
        .series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(1)
        .max(1);
    for (series_index, series) in data.series.iter().enumerate() {
        let color = PALETTE[series_index % PALETTE.len()];
        ctx.begin_path();
        for (index, value) in series.values.iter().enumerate() {
            let x = px + (index as f64 + 0.5) * pw / categories as f64;
            let y = py + ph - f64::from(*value) / range * ph;
            if index == 0 {
                ctx.move_to(x, y);
            } else {
                ctx.line_to(x, y);
            }
        }
        stroke(ctx, color, 1.5);
    }
}

fn paint_scatter(
    ctx: &OffscreenCanvasRenderingContext2d,
    plot: Plot,
    data: &ChartData,
    range: f64,
) {
    let (px, py, pw, ph) = plot;
    let categories = data
        .series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(1)
        .max(1);
    for (series_index, series) in data.series.iter().enumerate() {
        let css = PALETTE[series_index % PALETTE.len()].to_css();
        for (index, value) in series.values.iter().enumerate() {
            let x = px + (index as f64 + 0.5) * pw / categories as f64;
            let y = py + ph - f64::from(*value) / range * ph;
            ctx.begin_path();
            let _ = ctx.arc(x, y, 2.5, 0.0, std::f64::consts::TAU);
            ctx.set_fill_style_str(&css);
            ctx.fill();
        }
    }
}

fn paint_areas(ctx: &OffscreenCanvasRenderingContext2d, plot: Plot, data: &ChartData, range: f64) {
    let (px, py, pw, ph) = plot;
    let baseline = py + ph;
    let categories = data
        .series
        .iter()
        .map(|series| series.values.len())
        .max()
        .unwrap_or(1)
        .max(1);
    for (series_index, series) in data.series.iter().enumerate() {
        let color = PALETTE[series_index % PALETTE.len()];
        ctx.begin_path();
        ctx.move_to(px, baseline);
        for (index, value) in series.values.iter().enumerate() {
            let x = px + (index as f64 + 0.5) * pw / categories as f64;
            let y = py + ph - f64::from(*value) / range * ph;
            ctx.line_to(x, y);
        }
        ctx.line_to(px + pw, baseline);
        ctx.close_path();
        let css = color.to_css();
        ctx.set_fill_style_str(&css);
        ctx.fill();
    }
}

fn paint_pie(ctx: &OffscreenCanvasRenderingContext2d, plot: Plot, data: &ChartData) {
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
        if value <= 0.0 {
            continue;
        }
        let end = angle + value / total * std::f64::consts::TAU;
        ctx.begin_path();
        ctx.move_to(cx, cy);
        let _ = ctx.arc(cx, cy, radius, angle, end);
        ctx.close_path();
        let css = PALETTE[index % PALETTE.len()].to_css();
        ctx.set_fill_style_str(&css);
        ctx.fill();
        angle = end;
    }
}

fn paint_legend(
    ctx: &OffscreenCanvasRenderingContext2d,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    data: &ChartData,
) {
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
        fill(
            ctx,
            x + 8.0 + index as f64 * slot,
            y + (h - 8.0) / 2.0,
            8.0,
            8.0,
            color,
        );
        text(
            ctx,
            x + 20.0 + index as f64 * slot,
            y + h - 5.0,
            name,
            "10px sans-serif",
            "#444444",
            "left",
        );
    }
}

fn fill(ctx: &OffscreenCanvasRenderingContext2d, x: f64, y: f64, w: f64, h: f64, color: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let css = color.to_css();
    ctx.set_fill_style_str(&css);
    ctx.fill_rect(x, y, w, h);
}

fn stroke(ctx: &OffscreenCanvasRenderingContext2d, color: Color, width: f64) {
    let css = color.to_css();
    ctx.set_stroke_style_str(&css);
    ctx.set_line_width(width);
    ctx.stroke();
}

fn text(
    ctx: &OffscreenCanvasRenderingContext2d,
    x: f64,
    y: f64,
    text: &str,
    font: &str,
    color: &str,
    align: &str,
) {
    ctx.set_font(font);
    ctx.set_fill_style_str(color);
    ctx.set_text_align(align);
    ctx.set_text_baseline("alphabetic");
    let _ = ctx.fill_text(text, x, y);
}

/// Подпись деления: не больше двух знаков после запятой, без хвостовых нулей.
fn fmt_tick(value: f64) -> String {
    if value.abs() < 1e-9 {
        return "0".to_owned();
    }
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
