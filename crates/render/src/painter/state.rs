use crate::display_list::{Color, LineStyle};
use std::collections::HashMap;

/// Стек clip/transform + кэш стилей, чтобы не дёргать `ctx.set*` без нужды.
pub struct PaintState {
    pub clip_depth: u32,
    pub transform_depth: u32,
    current_fill: Option<Color>,
    current_stroke: Option<Color>,
    current_line_width: f32,
    /// `line_dash` живёт в контексте и сам не сбрасывается, поэтому штриховка
    /// входит в ключ кэша наравне с цветом и толщиной.
    current_line_style: Option<LineStyle>,
    /// Последнее выставленное направление текста (`true` — rtl): `direction`
    /// входит в состояние контекста и его тоже откатывает `restore()`, поэтому
    /// кэш живёт здесь и сбрасывается вместе со стилями. `None` — неизвестно.
    pub direction: Option<bool>,
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
            current_line_style: None,
            direction: None,
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
        let size_bits = size_px.to_bits();
        // Попадание ищется по заимствованной строке, без клона семейства:
        // `HashMap::entry` требует владеющий ключ, а raw-entry API в std пока
        // нестабилен. Кэш — единицы записей, поэтому двойной линейный просмотр
        // (проверка + поиск) дешевле аллокации строки на каждой команде Text.
        let hit = self.font_cache.iter().any(|(k, _)| {
            k.0.as_str() == family && k.1 == size_bits && k.2 == bold && k.3 == italic
        });
        if hit {
            // Первый проход гарантировал попадание — здесь `unwrap` невозможен.
            return self
                .font_cache
                .iter()
                .find(|(k, _)| {
                    k.0.as_str() == family && k.1 == size_bits && k.2 == bold && k.3 == italic
                })
                .map(|(_, css)| css.as_str())
                .unwrap();
        }
        // Промах: семейство клонируется один раз — только при вставке.
        self.font_cache
            .entry((family.to_owned(), size_bits, bold, italic))
            .or_insert_with(|| {
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
    pub fn needs_stroke(&mut self, c: Color, w: f32, style: LineStyle) -> bool {
        if self.current_stroke == Some(c)
            && (self.current_line_width - w).abs() < f32::EPSILON
            && self.current_line_style == Some(style)
        {
            return false;
        }
        self.current_stroke = Some(c);
        self.current_line_width = w;
        self.current_line_style = Some(style);
        true
    }

    /// Забывает стили кисти, не трогая глубины клипа и трансформа: нужна там,
    /// где контекст откатился сам.
    ///
    /// `ctx.restore()` возвращает заливку, обводку, штриховку и направление
    /// текста к состоянию на `save()`, а кэш об этом не знает — следующий
    /// `needs_fill` с тем же цветом пропустил бы установку и нарисовал чужим
    /// цветом. Кэши строк (`style_cache`, `font_cache`) от контекста не зависят
    /// и остаются на месте.
    pub fn invalidate_styles(&mut self) {
        self.current_fill = None;
        self.current_stroke = None;
        self.current_line_width = 0.0;
        self.current_line_style = None;
        self.direction = None;
    }

    pub fn reset(&mut self) {
        self.clip_depth = 0;
        self.transform_depth = 0;
        self.invalidate_styles();
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
    fn stroke_dedup_includes_width_and_style() {
        let mut s = PaintState::new();
        let red = Color::rgba(255, 0, 0, 255);
        assert!(s.needs_stroke(red, 1.0, LineStyle::Solid));
        assert!(!s.needs_stroke(red, 1.0, LineStyle::Solid));
        assert!(s.needs_stroke(red, 2.0, LineStyle::Solid));
        // Смена штриховки — тоже повод обновить контекст, иначе пунктир
        // остался бы на сплошной линии.
        assert!(s.needs_stroke(red, 2.0, LineStyle::Dashed));
        assert!(!s.needs_stroke(red, 2.0, LineStyle::Dashed));
        assert!(s.needs_stroke(red, 2.0, LineStyle::Solid));
    }

    #[test]
    fn invalidate_styles_keeps_depths() {
        let mut s = PaintState::new();
        s.clip_depth = 1;
        s.transform_depth = 2;
        let red = Color::rgba(255, 0, 0, 255);
        assert!(s.needs_fill(red));
        assert!(s.needs_stroke(red, 2.0, LineStyle::Dashed));

        // Контекст вернулся к состоянию на `save()`: стили спрашиваются заново,
        // а стек клипа и трансформа остаётся на месте.
        s.invalidate_styles();
        assert!(s.needs_fill(red));
        assert!(s.needs_stroke(red, 2.0, LineStyle::Dashed));
        assert_eq!(s.clip_depth, 1);
        assert_eq!(s.transform_depth, 2);
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
