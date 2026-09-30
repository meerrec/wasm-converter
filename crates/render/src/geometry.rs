use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }
    pub fn right(&self)  -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right() && self.right() > other.x
            && self.y < other.bottom() && self.bottom() > other.y
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Viewport { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub scale: f32 }

impl Default for Viewport {
    fn default() -> Self { Self { x: 0.0, y: 0.0, w: 1024.0, h: 768.0, scale: 1.0 } }
}
