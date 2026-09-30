use serde::{Deserialize, Serialize};

use crate::{Color, Rect, StrokeStyle, Viewport};

pub type FontId = u32;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TextAlign    { Left, Center, Right, Justify }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TextBaseline { Top, Middle, Alphabetic, Bottom }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TextOverflow { Clip, Ellipsis, Overflow }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Crop { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DrawCommand {
    Rect { x: f32, y: f32, w: f32, h: f32,
           fill: Option<Color>, stroke: Option<StrokeStyle>, radius: [f32; 4] },
    Text { x: f32, y: f32, text: String, font_id: FontId, size_px: f32,
           color: Color, align: TextAlign, baseline: TextBaseline,
           max_width: Option<f32>, overflow: TextOverflow },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, stroke: StrokeStyle },
    Polygon { points: Vec<(f32, f32)>, fill: Option<Color>, stroke: Option<StrokeStyle> },
    Image { x: f32, y: f32, w: f32, h: f32, bitmap_id: u32, crop: Option<Crop> },
    PushClip { x: f32, y: f32, w: f32, h: f32 },
    PopClip,
    PushTransform { a: f32, b: f32, c: f32, d: f32, e: f32, f: f32 },
    PopTransform,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayList {
    pub commands: Vec<DrawCommand>,
    pub viewport: Viewport,
    pub scale_factor: f32,
    pub content_bounds: Rect,
    pub visible_bounds: Rect,
    pub fonts_used: Vec<FontId>,
    pub bitmaps_used: Vec<u32>,
}

impl DisplayList {
    pub fn empty() -> Self {
        Self {
            commands: Vec::new(),
            viewport: Viewport::default(),
            scale_factor: 1.0,
            content_bounds: Rect::default(),
            visible_bounds: Rect::default(),
            fonts_used: Vec::new(),
            bitmaps_used: Vec::new(),
        }
    }

    /// Переиспользует capacity между кадрами.
    pub fn recycle(&mut self) -> Vec<DrawCommand> {
        let mut v = std::mem::take(&mut self.commands);
        v.clear();
        v
    }
}
