#![cfg(target_arch = "wasm32")]

use super::bitmap_cache::BitmapCache;
use super::state::PaintState;
use super::text;
use crate::display_list::{
    Color, DecodeError, DisplayList, DrawCommand, LineStyle, TextAlign, TextBaseline,
};
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2d;

#[derive(Debug, Clone, Copy, Default)]
pub struct PaintStats {
    pub cmds: u32,
    pub dropped: bool,
    pub paint_ms: f64,
}

pub struct Painter2D {
    ctx: OffscreenCanvasRenderingContext2d,
    state: PaintState,
    bitmaps: BitmapCache,
}

impl Painter2D {
    pub fn new(ctx: OffscreenCanvasRenderingContext2d) -> Self {
        Self {
            ctx,
            state: PaintState::new(),
            bitmaps: BitmapCache::new(),
        }
    }

    pub fn context(&self) -> &OffscreenCanvasRenderingContext2d {
        &self.ctx
    }

    pub fn bitmaps_mut(&mut self) -> &mut BitmapCache {
        &mut self.bitmaps
    }

    pub fn register_bitmap(&mut self, id: u32, bmp: web_sys::ImageBitmap) {
        self.bitmaps.insert(id, bmp);
    }

    pub fn drop_bitmap(&mut self, id: u32) -> bool {
        self.bitmaps.remove(id)
    }

    pub fn reset_state(&mut self) {
        self.state.reset();
        self.bitmaps.clear();
    }

    pub fn paint_bytes(&mut self, bytes: &[u8]) -> Result<PaintStats, JsValue> {
        let t0 = now_ms();
        let reader = DisplayList::from_bytes(bytes)
            .map_err(|e| JsValue::from_str(&format!("DisplayList decode: {e}")))?;

        let mut cmds = 0u32;
        for item in reader.iter() {
            match item {
                Ok(cmd) => {
                    self.dispatch(&reader, cmd);
                    cmds += 1;
                }
                Err(DecodeError::UnknownTag(t)) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "painter: unknown tag {t:#x}, skipping rest"
                    )));
                    break;
                }
                Err(e) => {
                    return Err(JsValue::from_str(&format!("DisplayList decode: {e}")));
                }
            }
        }
        let t1 = now_ms();
        Ok(PaintStats {
            cmds,
            dropped: false,
            paint_ms: t1 - t0,
        })
    }

    fn dispatch(&mut self, reader: &crate::display_list::DisplayListReader, cmd: DrawCommand) {
        match cmd {
            DrawCommand::Clear => {
                let c = self.ctx.canvas();
                let w = c.width() as f64;
                let h = c.height() as f64;
                self.ctx.clear_rect(0.0, 0.0, w, h);
            }
            DrawCommand::Rect {
                x,
                y,
                w,
                h,
                fill,
                stroke,
                stroke_w,
                radius,
            } => {
                self.begin_path_rounded(x, y, w, h, radius);
                if fill.0 != 0 {
                    let css = self.state.css_for(fill).to_owned();
                    if self.state.needs_fill(fill) {
                        self.ctx.set_fill_style_str(&css);
                    }
                    self.ctx.fill();
                }
                if stroke.0 != 0 && stroke_w > 0.0 {
                    self.apply_stroke(stroke, stroke_w, LineStyle::Solid);
                    self.ctx.stroke();
                }
            }
            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                stroke_w,
                style,
            } => {
                self.stroke_line(x1, y1, x2, y2, stroke, stroke_w, style);
            }
            DrawCommand::Text {
                x,
                y,
                text: text_ref,
                font: font_ref,
                size,
                color,
                align,
                baseline,
                bold,
                italic,
                underline,
            } => {
                let s = reader.string(text_ref);
                let family = reader.string(font_ref);
                let font = self.state.font_css(family, size, bold, italic).to_owned();
                self.ctx.set_font(&font);

                let css = self.state.css_for(color).to_owned();
                self.ctx.set_fill_style_str(&css);
                self.ctx.set_text_align(text::text_align_str(align));

                // Canvas подчёркивания не рисует, а его положение берётся из
                // метрик глифов. `TextMetrics` отсчитываются от текущего
                // `text_baseline`, поэтому меряем от альфабетической линии:
                // иначе подчёркивание уехало бы вместе с вертикальным
                // выравниванием.
                let metrics = if underline {
                    self.ctx.set_text_baseline("alphabetic");
                    self.ctx.measure_text(s).ok()
                } else {
                    None
                };

                self.ctx.set_text_baseline(match baseline {
                    TextBaseline::Alphabetic => "alphabetic",
                    TextBaseline::Top => "top",
                    TextBaseline::Middle => "middle",
                    TextBaseline::Bottom => "bottom",
                });
                let _ = self.ctx.fill_text(s, x as f64, y as f64);

                if let Some(metrics) = metrics {
                    self.draw_underline(&metrics, x, y, size, color, align, baseline);
                }
            }
            DrawCommand::Image {
                x,
                y,
                w,
                h,
                bitmap_id,
            } => {
                if let Some(bmp) = self.bitmaps.get(bitmap_id) {
                    let _ = self.ctx.draw_image_with_image_bitmap_and_dw_and_dh(
                        bmp, x as f64, y as f64, w as f64, h as f64,
                    );
                }
            }
            DrawCommand::PushClip { x, y, w, h } => {
                self.ctx.save();
                self.state.clip_depth += 1;
                self.ctx.begin_path();
                self.ctx.rect(x as f64, y as f64, w as f64, h as f64);
                self.ctx.clip();
            }
            DrawCommand::PopClip => {
                if self.state.clip_depth > 0 {
                    self.ctx.restore();
                    self.state.clip_depth -= 1;
                }
            }
            DrawCommand::PushTransform { a, b, c, d, e, f } => {
                self.ctx.save();
                self.state.transform_depth += 1;
                let _ = self
                    .ctx
                    .set_transform(a as f64, b as f64, c as f64, d as f64, e as f64, f as f64);
            }
            DrawCommand::PopTransform => {
                if self.state.transform_depth > 0 {
                    self.ctx.restore();
                    self.state.transform_depth -= 1;
                }
            }
        }
    }

    /// Переносит стиль обводки в контекст, если он разошёлся с кэшем.
    ///
    /// Штриховка — часть стиля: `line_dash` живёт в контексте и сам не
    /// сбрасывается, поэтому сплошная линия после пунктира обязана явно
    /// снять массив штрихов.
    fn apply_stroke(&mut self, stroke: Color, width: f32, style: LineStyle) {
        let css = self.state.css_for(stroke).to_owned();
        if self.state.needs_stroke(stroke, width, style) {
            self.ctx.set_stroke_style_str(&css);
            self.ctx.set_line_width(f64::from(width));
            apply_dash(&self.ctx, style, width);
        }
    }

    // Координаты линии — четыре числа подряд из `DrawCommand::Line`; свернуть
    // их в структуру можно, но вызывается это из одного места.
    #[allow(clippy::too_many_arguments)]
    fn stroke_line(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Color,
        stroke_w: f32,
        style: LineStyle,
    ) {
        if stroke.0 == 0 || stroke_w <= 0.0 {
            return;
        }
        // `Double` — две линии по трети толщины и такой же зазор между ними:
        // в сумме ровно `stroke_w`, как рисует Excel.
        let width = if style == LineStyle::Double {
            stroke_w / 3.0
        } else {
            stroke_w
        };
        self.apply_stroke(stroke, width, style);

        let (dx, dy) = (x2 - x1, y2 - y1);
        let len = dx.hypot(dy);
        if len <= f32::EPSILON {
            return;
        }

        self.ctx.begin_path();
        if style == LineStyle::Double {
            let off = stroke_w / 3.0;
            let (nx, ny) = (-dy / len * off, dx / len * off);
            self.ctx.move_to(f64::from(x1 + nx), f64::from(y1 + ny));
            self.ctx.line_to(f64::from(x2 + nx), f64::from(y2 + ny));
            self.ctx.move_to(f64::from(x1 - nx), f64::from(y1 - ny));
            self.ctx.line_to(f64::from(x2 - nx), f64::from(y2 - ny));
        } else {
            self.ctx.move_to(f64::from(x1), f64::from(y1));
            self.ctx.line_to(f64::from(x2), f64::from(y2));
        }
        self.ctx.stroke();
    }

    // Аргументы — поля `DrawCommand::Text` без гарнитуры: подчёркивание
    // считается от уже снятых метрик и якоря текста.
    #[allow(clippy::too_many_arguments)]
    fn draw_underline(
        &mut self,
        metrics: &web_sys::TextMetrics,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        let ascent = metrics.font_bounding_box_ascent();
        let descent = metrics.font_bounding_box_descent();
        // Метрики сняты от альфабетической линии, а `fill_text` отсчитывает
        // `y` от выбранного выравнивания — возвращаем якорь в систему
        // альфабетической базовой линии.
        let baseline_y = match baseline {
            TextBaseline::Alphabetic => f64::from(y),
            TextBaseline::Top => f64::from(y) + ascent,
            TextBaseline::Middle => f64::from(y) + (ascent - descent) / 2.0,
            TextBaseline::Bottom => f64::from(y) - descent,
        };
        let width = metrics.width();
        let (from, to) = match align {
            TextAlign::Center => (f64::from(x) - width / 2.0, f64::from(x) + width / 2.0),
            TextAlign::Right => (f64::from(x) - width, f64::from(x)),
            TextAlign::Left | TextAlign::Justify => (f64::from(x), f64::from(x) + width),
        };
        // Толщина и отступ — доли кегля: у Calibri подчёркивание проходит
        // примерно на 0.12 em ниже базовой линии.
        let thickness = (size * 0.06).max(1.0);
        let uy = baseline_y + f64::from(size) * 0.12;
        self.apply_stroke(color, thickness, LineStyle::Solid);
        self.ctx.begin_path();
        self.ctx.move_to(from, uy);
        self.ctx.line_to(to, uy);
        self.ctx.stroke();
    }

    fn begin_path_rounded(&self, x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) {
        self.ctx.begin_path();
        let uniform = r[0] == r[1] && r[1] == r[2] && r[2] == r[3];
        if uniform && r[0] > 0.0 {
            rounded_rect_arc_to(
                &self.ctx,
                x as f64,
                y as f64,
                w as f64,
                h as f64,
                r[0] as f64,
                r[0] as f64,
                r[0] as f64,
                r[0] as f64,
            );
        } else if !uniform {
            rounded_rect_arc_to(
                &self.ctx,
                x as f64,
                y as f64,
                w as f64,
                h as f64,
                r[0] as f64,
                r[1] as f64,
                r[2] as f64,
                r[3] as f64,
            );
        } else {
            self.ctx.rect(x as f64, y as f64, w as f64, h as f64);
        }
    }
}

/// Штриховка линии; пустой массив — сплошная, так снимается штрих от
/// предыдущей команды. Длины сегментов — в единицах толщины, чтобы рисунок
/// не зависел от того, тонкая линия или толстая.
fn apply_dash(ctx: &OffscreenCanvasRenderingContext2d, style: LineStyle, width: f32) {
    let segments = js_sys::Array::new();
    let w = f64::from(width);
    match style {
        LineStyle::Dashed => {
            segments.push(&JsValue::from_f64(w * 3.0));
            segments.push(&JsValue::from_f64(w * 2.0));
        }
        LineStyle::Dotted => {
            segments.push(&JsValue::from_f64(w));
            segments.push(&JsValue::from_f64(w * 2.0));
        }
        LineStyle::Solid | LineStyle::Double => {}
    }
    // Отказ `set_line_dash` оставил бы чужой штрих, но значения массива —
    // конечные положительные числа, на которых он не срабатывает.
    let _ = ctx.set_line_dash(&JsValue::from(segments));
}

// Девять чисел — это прямоугольник и четыре радиуса: свернуть их в структуру
// можно, но вызывается функция из одного места и ровно с этими значениями.
#[allow(clippy::too_many_arguments)]
fn rounded_rect_arc_to(
    ctx: &OffscreenCanvasRenderingContext2d,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    tl: f64,
    tr: f64,
    br: f64,
    bl: f64,
) {
    ctx.move_to(x + tl, y);
    ctx.line_to(x + w - tr, y);
    // `arc_to` — единственный из path-методов, который бросает (при radius < 0),
    // поэтому web-sys отдаёт `Result`. Скругления берём из DisplayList как есть.
    let _ = ctx.arc_to(x + w, y, x + w, y + tr, tr);
    ctx.line_to(x + w, y + h - br);
    let _ = ctx.arc_to(x + w, y + h, x + w - br, y + h, br);
    ctx.line_to(x + bl, y + h);
    let _ = ctx.arc_to(x, y + h, x, y + h - bl, bl);
    ctx.line_to(x, y + tl);
    let _ = ctx.arc_to(x, y, x + tl, y, tl);
    ctx.close_path();
}

#[inline]
fn now_ms() -> f64 {
    js_sys::Date::now()
}
