use crate::display_list::Color;
use std::collections::HashMap;

/// Стек clip/transform + кэш стилей, чтобы не дёргать `ctx.set*` без нужды.
pub struct PaintState {
    pub clip_depth: u32,
    pub transform_depth: u32,
    current_fill: Option<Color>,
    current_stroke: Option<Color>,
    current_line_width: f32,
    pub style_cache: HashMap<Color, String>,
    pub font_cache: HashMap<(u32, u32), String>,
}

impl PaintState {
    pub fn new() -> Self {
        Self {
            clip_depth: 0,
            transform_depth: 0,
            current_fill: None,
            current_stroke: None,
            current_line_width: 0.0,
            style_cache: HashMap::with_capacity(64),
            font_cache: HashMap::with_capacity(16),
        }
    }

    #[inline]
    pub fn css_for(&mut self, c: Color) -> &str {
        self.style_cache.entry(c).or_insert_with(|| c.to_css())
    }

    #[inline]
    pub fn font_css(&mut self, font_id: u32, size_px: f32) -> &str {
        self.font_cache
            .entry((font_id, size_px.to_bits()))
            .or_insert_with(|| format!("{size_px}px font{font_id}"))
    }

    #[inline]
    pub fn needs_fill(&mut self, c: Color) -> bool {
        if self.current_fill == Some(c) {
            return false;
        }
        self.current_fill = Some(c);
        true
    }

    #[inline]
    pub fn needs_stroke(&mut self, c: Color, w: f32) -> bool {
        if self.current_stroke == Some(c) && (self.current_line_width - w).abs() < f32::EPSILON {
            return false;
        }
        self.current_stroke = Some(c);
        self.current_line_width = w;
        true
    }

    pub fn reset(&mut self) {
        self.clip_depth = 0;
        self.transform_depth = 0;
        self.current_fill = None;
        self.current_stroke = None;
        self.current_line_width = 0.0;
    }
}

impl Default for PaintState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_style_dedup() {
        let mut s = PaintState::new();
        let red = Color::rgba(255, 0, 0, 255);
        assert!(s.needs_fill(red));
        assert!(!s.needs_fill(red));
        assert!(s.needs_fill(Color::BLACK));
    }

    #[test]
    fn stroke_dedup_includes_width() {
        let mut s = PaintState::new();
        let red = Color::rgba(255, 0, 0, 255);
        assert!(s.needs_stroke(red, 1.0));
        assert!(!s.needs_stroke(red, 1.0));
        assert!(s.needs_stroke(red, 2.0));
    }

    #[test]
    fn css_cached() {
        let mut s = PaintState::new();
        let c = Color::rgba(10, 20, 30, 255);
        let a = s.css_for(c).to_owned();
        let b = s.css_for(c).to_owned();
        assert_eq!(a, b);
        assert_eq!(a, "#0a141e");
    }

    #[test]
    fn font_css_stable() {
        let mut s = PaintState::new();
        assert_eq!(s.font_css(3, 14.0), "14px font3");
        assert_eq!(s.font_css(3, 14.0), "14px font3");
    }
}
