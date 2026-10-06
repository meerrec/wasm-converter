//! Canvas-бэкенд диаграммы: геометрию даёт [`crate::chart::layout`] (ADR-0011),
//! painter только исполняет примитивы.
//!
//! Порядок обхода, стили, шрифты (`Npx sans-serif`) и клип по прямоугольнику
//! диаграммы повторяют прежний цикл один в один: вид просмотрщика —
//! зафиксированный результат, менять его нельзя.

use crate::chart::{layout, ChartData, ChartPrim, Point, TextAlign};
use crate::display_list::Color;
use crate::geometry::Rect;
use web_sys::OffscreenCanvasRenderingContext2d;

/// Нарисовать диаграмму в прямоугольнике. Пустая раскладка — нечего рисовать.
pub fn paint(
    ctx: &OffscreenCanvasRenderingContext2d,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    data: &ChartData,
) {
    let prims = layout(Rect::new(x, y, w, h), data);
    if prims.is_empty() {
        return;
    }

    ctx.save();
    ctx.begin_path();
    ctx.rect(f64::from(x), f64::from(y), f64::from(w), f64::from(h));
    ctx.clip();
    for prim in &prims {
        draw(ctx, prim);
    }
    ctx.restore();
}

fn draw(ctx: &OffscreenCanvasRenderingContext2d, prim: &ChartPrim) {
    match prim {
        ChartPrim::Rect { x, y, w, h, fill } => {
            set_fill(ctx, *fill);
            ctx.fill_rect(*x, *y, *w, *h);
        }
        ChartPrim::Polyline {
            points,
            stroke,
            width,
            closed,
        } => {
            path(ctx, points);
            if *closed {
                ctx.close_path();
            }
            set_stroke(ctx, *stroke, *width);
            ctx.stroke();
        }
        ChartPrim::Polygon { points, fill } => {
            path(ctx, points);
            ctx.close_path();
            set_fill(ctx, *fill);
            ctx.fill();
        }
        ChartPrim::Circle { cx, cy, r, fill } => {
            ctx.begin_path();
            let _ = ctx.arc(*cx, *cy, *r, 0.0, std::f64::consts::TAU);
            set_fill(ctx, *fill);
            ctx.fill();
        }
        ChartPrim::Sector {
            cx,
            cy,
            r,
            from,
            to,
            fill,
        } => {
            ctx.begin_path();
            ctx.move_to(*cx, *cy);
            // Крошечный прямоугольник даёт отрицательный радиус, а это ошибка
            // DOM: прежний canvas-путь её так же игнорировал и заливал пустой
            // путь.
            let _ = ctx.arc(*cx, *cy, *r, *from, *to);
            ctx.close_path();
            set_fill(ctx, *fill);
            ctx.fill();
        }
        ChartPrim::Text {
            x,
            y,
            text,
            size,
            align,
            fill,
        } => {
            ctx.set_font(&format!("{size}px sans-serif"));
            set_fill(ctx, *fill);
            ctx.set_text_align(canvas_align(*align));
            ctx.set_text_baseline("alphabetic");
            let _ = ctx.fill_text(text, *x, *y);
        }
    }
}

fn path(ctx: &OffscreenCanvasRenderingContext2d, points: &[Point]) {
    ctx.begin_path();
    let mut iter = points.iter();
    if let Some((x, y)) = iter.next() {
        ctx.move_to(*x, *y);
    }
    for (x, y) in iter {
        ctx.line_to(*x, *y);
    }
}

fn set_fill(ctx: &OffscreenCanvasRenderingContext2d, color: Color) {
    ctx.set_fill_style_str(&color.to_css());
}

fn set_stroke(ctx: &OffscreenCanvasRenderingContext2d, color: Color, width: f64) {
    ctx.set_stroke_style_str(&color.to_css());
    ctx.set_line_width(width);
}

fn canvas_align(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
}
