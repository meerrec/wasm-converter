use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color { pub r: u8, pub g: u8, pub b: u8, pub a: u8 }

impl Color {
    pub const TRANSPARENT: Self = Self { r: 0, g: 0, b: 0, a: 0 };
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self { Self { r, g, b, a } }
    pub fn to_css(self) -> String {
        format!("rgba({},{},{},{})", self.r, self.g, self.b, f32::from(self.a) / 255.0)
    }
    pub fn to_rgb_f32(self) -> [f32; 3] {
        [f32::from(self.r) / 255.0, f32::from(self.g) / 255.0, f32::from(self.b) / 255.0]
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LineCap { Butt, Round, Square }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LineJoin { Miter, Round, Bevel }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StrokeStyle {
    pub color: Color,
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub dash: Option<[f32; 2]>,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self { color: Color::BLACK, width: 1.0, cap: LineCap::Butt, join: LineJoin::Miter, dash: None }
    }
}
