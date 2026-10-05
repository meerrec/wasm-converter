#![cfg(target_arch = "wasm32")]

use super::bitmap_cache::BitmapCache;
use super::state::PaintState;
use super::text;
use crate::display_list::{DecodeError, DisplayList, DrawCommand, TextBaseline};
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
                    let css = self.state.css_for(stroke).to_owned();
                    if self.state.needs_stroke(stroke, stroke_w) {
                        self.ctx.set_stroke_style_str(&css);
                        self.ctx.set_line_width(stroke_w as f64);
                    }
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
            } => {
                if stroke.0 == 0 || stroke_w <= 0.0 {
                    return;
                }
                let css = self.state.css_for(stroke).to_owned();
                if self.state.needs_stroke(stroke, stroke_w) {
                    self.ctx.set_stroke_style_str(&css);
                    self.ctx.set_line_width(stroke_w as f64);
                }
                self.ctx.begin_path();
                self.ctx.move_to(x1 as f64, y1 as f64);
                self.ctx.line_to(x2 as f64, y2 as f64);
                self.ctx.stroke();
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
            } => {
                let s = reader.string(text_ref);
                let family = reader.string(font_ref);
                let font = self.state.font_css(family, size, bold, italic).to_owned();
                self.ctx.set_font(&font);

                let css = self.state.css_for(color).to_owned();
                self.ctx.set_fill_style_str(&css);
                self.ctx.set_text_align(text::text_align_str(align));
                self.ctx.set_text_baseline(match baseline {
                    TextBaseline::Alphabetic => "alphabetic",
                    TextBaseline::Top => "top",
                    TextBaseline::Middle => "middle",
                    TextBaseline::Bottom => "bottom",
                });
                let _ = self.ctx.fill_text(s, x as f64, y as f64);
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
