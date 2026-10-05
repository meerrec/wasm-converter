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
    /// Ключ — семейство, кегль в битах, полужирный и курсив.
    pub font_cache: HashMap<(String, u32, bool, bool), String>,
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

    /// CSS-описание шрифта для `ctx.font`.
    ///
    /// Порядок частей в CSS-сокращении фиксирован: стиль, вариант, вес,
    /// размер, семейство. Кегль — обязательная часть, поэтому он здесь и
    /// единственный, что не опускается.
    #[inline]
    pub fn font_css(&mut self, family: &str, size_px: f32, bold: bool, italic: bool) -> &str {
        let key = (family.to_owned(), size_px.to_bits(), bold, italic);
        self.font_cache.entry(key).or_insert_with(|| {
            let style = if italic { "italic " } else { "" };
            let weight = if bold { "bold " } else { "" };
            format!("{style}{weight}{size_px}px {family}, sans-serif")
        })
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
        assert_eq!(
            s.font_css("Calibri", 14.0, false, false),
            "14px Calibri, sans-serif"
        );
        assert_eq!(
            s.font_css("Calibri", 14.0, false, false),
            "14px Calibri, sans-serif"
        );
        assert_eq!(
            s.font_css("Calibri", 14.0, true, true),
            "italic bold 14px Calibri, sans-serif"
        );
    }
}
