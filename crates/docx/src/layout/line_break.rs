//! Перенос строк для DOCX: разбивка абзаца на строки с учётом стилей.
//!
//! Использует [`doc_converter_render::text_measure`] для измерения ширин
//! (ADR-0005). Логика та же, что и для XLSX, чтобы точки разрыва не
//! разъезжались между canvas и PDF.

use std::ops::Range;

use doc_converter_render::{
    font::{FontId, FontRegistry},
    text_measure,
};

use crate::model::raw::HalfPoint;

/// Правило межстрочного интервала: как понимать `w:spacing w:line` (`w:lineRule`).
///
/// Каскад отдаёт правило раскладке в пикселях и множителях, а не в единицах XML
/// ([`super::cascade`]): twips в абсолютных правилах переводит `super::engine::twips_to_px`,
/// чтобы перевод остался в одном месте.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum LineRule {
    /// `w:line` не задан: высота строки берётся из метрики шрифта.
    #[default]
    Single,
    /// Множитель метрики; `w:line` в 240-х долях строки (`lineRule="auto"`).
    Auto(f32),
    /// Точная высота строки в пикселях (`lineRule="exact"`).
    Exact(f32),
    /// Высота строки не меньше значения в пикселях (`lineRule="atLeast"`).
    AtLeast(f32),
}

/// Разбиватель строк: кэширует измерения и управляет состоянием.
pub struct LineBreaker<'a> {
    fonts: &'a mut FontRegistry,
    _default_font_id: FontId,
}

impl<'a> LineBreaker<'a> {
    /// Создать новый разбиватель строк.
    #[must_use]
    pub fn new(fonts: &'a mut FontRegistry, default_font_id: FontId) -> Self {
        Self {
            fonts,
            _default_font_id: default_font_id,
        }
    }

    /// Измерить ширину текста в пикселях.
    ///
    /// # Arguments
    /// * `text` - текст для измерения
    /// * `font_id` - идентификатор шрифта
    /// * `font_size_half_points` - размер шрифта в полупунктах (24 = 12 pt)
    #[must_use]
    pub fn measure_text(
        &mut self,
        text: &str,
        font_id: FontId,
        font_size_half_points: HalfPoint,
    ) -> f32 {
        let size_px = half_points_to_px(font_size_half_points.value());
        text_measure::measure_text(self.fonts, font_id, size_px, text)
    }

    /// Разбить текст на строки, которые помещаются в `max_width`.
    ///
    /// # Arguments
    /// * `text` - текст для разбивки
    /// * `font_id` - идентификатор шрифта
    /// * `font_size_half_points` - размер шрифта в полупунктах
    /// * `max_width` - максимальная ширина строки в пикселях
    ///
    /// # Returns
    /// Вектор байтовых диапазонов (`Range<usize>`) по исходному `text`.
    /// Каждый диапазон — отдельная строка.
    #[must_use]
    pub fn break_lines(
        &mut self,
        text: &str,
        font_id: FontId,
        font_size_half_points: HalfPoint,
        max_width: f32,
    ) -> Vec<Range<usize>> {
        let size_px = half_points_to_px(font_size_half_points.value());
        text_measure::break_lines(self.fonts, font_id, size_px, text, max_width)
    }

    /// Вычислить высоту строки на основе размера шрифта.
    ///
    /// Использует коэффициент 1.2 для межстрочного интервала по умолчанию.
    #[must_use]
    pub fn line_height(&self, font_size_half_points: HalfPoint) -> f32 {
        let size_px = half_points_to_px(font_size_half_points.value());
        size_px * 1.2
    }

    /// Вычислить высоту строки с учётом межстрочного интервала.
    ///
    /// # Arguments
    /// * `font_size_half_points` - размер шрифта
    /// * `line_spacing` - межстрочный интервал в 240-х долях (None = 1.0, Some(240) = 1.0, Some(480) = 2.0)
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn line_height_with_spacing(
        &self,
        font_size_half_points: HalfPoint,
        line_spacing: Option<i32>,
    ) -> f32 {
        let base_height = self.line_height(font_size_half_points);

        match line_spacing {
            Some(spacing) => {
                // spacing в полупунктах: 240 = 12pt = 1.0x
                let spacing_multiplier = f64::from(spacing) / 240.0;
                base_height * spacing_multiplier.max(1.0) as f32
            }
            None => base_height,
        }
    }

    /// Вычислить высоту строки по правилу `w:line` (`w:lineRule`).
    ///
    /// Единицы приходят из каскада ([`super::cascade`]): `Auto` — множитель
    /// базового интерлиньяжа, `Exact`/`AtLeast` — пиксели.
    #[must_use]
    pub fn line_height_for(&self, size: HalfPoint, rule: LineRule) -> f32 {
        let base_height = self.line_height(size);

        match rule {
            LineRule::Single => base_height,
            LineRule::Auto(multiplier) => base_height * multiplier,
            LineRule::Exact(height) => {
                // Ноль в `Exact` — испорченный документ: строка нулевой высоты
                // схлопнула бы весь абзац, поэтому базовый интервал.
                if height > 0.0 {
                    height
                } else {
                    base_height
                }
            }
            LineRule::AtLeast(height) => base_height.max(height),
        }
    }
}

/// Преобразовать полупункты в пиксели: 1 pt = 1/72 дюйма, 1 px = 1/96 дюйма.
/// Отношение: (1/72) / (1/96) = 96/72 = 4/3.
///
/// Полупункты: размер в XML записан как 24 для 12pt, поэтому делим на 2.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn half_points_to_px(half_points: i32) -> f32 {
    (f64::from(half_points) / 2.0 * (96.0 / 72.0)) as f32
}

/// Информация о строке после разбивки.
#[derive(Debug, Clone)]
pub struct LineInfo {
    /// Байтовый диапазон строки в исходном тексте.
    pub range: Range<usize>,
    /// Ширина строки в пикселях.
    pub width: f32,
    /// Высота строки в пикселях.
    pub height: f32,
}

/// Разбить текст на строки с полной информацией.
#[must_use]
pub fn break_lines_with_info(
    fonts: &mut FontRegistry,
    font_id: FontId,
    font_size_half_points: HalfPoint,
    text: &str,
    max_width: f32,
) -> Vec<LineInfo> {
    let size_px = half_points_to_px(font_size_half_points.value());
    let line_height = size_px * 1.2;

    let ranges = text_measure::break_lines(fonts, font_id, size_px, text, max_width);

    ranges
        .into_iter()
        .map(|range| {
            let width = text_measure::measure_text(fonts, font_id, size_px, &text[range.clone()]);
            LineInfo {
                range,
                width,
                height: line_height,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_converter_render::font::DEFAULT_FONT_ID;

    const PX_11PT: f32 = 11.0 * 96.0 / 72.0;

    /// Сравнение пикселей с допуском: `f32` в `assert_eq!` не пройдёт `clippy::float_cmp`.
    #[track_caller]
    fn assert_px(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    }

    #[test]
    fn line_height_follows_the_rule() {
        let mut fonts = FontRegistry::new(64);
        let breaker = LineBreaker::new(&mut fonts, DEFAULT_FONT_ID);
        // 12 pt = 16 px раскладки, базовый интерлиньяж — 19.2 px.
        let size = HalfPoint::new(24);

        assert_px(breaker.line_height_for(size, LineRule::Single), 19.2);
        assert_px(breaker.line_height_for(size, LineRule::Auto(2.0)), 38.4);
        assert_px(breaker.line_height_for(size, LineRule::Auto(1.0)), 19.2);
        assert_px(breaker.line_height_for(size, LineRule::Exact(24.0)), 24.0);
        assert_px(breaker.line_height_for(size, LineRule::AtLeast(10.0)), 19.2);
        assert_px(breaker.line_height_for(size, LineRule::AtLeast(30.0)), 30.0);
        assert_px(breaker.line_height_for(size, LineRule::Exact(0.0)), 19.2);
    }

    #[test]
    fn test_half_points_to_px() {
        // 24 полупункта = 12 pt
        assert!((half_points_to_px(24) - PX_11PT * 12.0 / 11.0).abs() < 0.01);
    }

    #[test]
    fn test_line_breaker_empty_text() {
        let mut fonts = FontRegistry::new(64);
        let mut breaker = LineBreaker::new(&mut fonts, DEFAULT_FONT_ID);

        let lines = breaker.break_lines("", DEFAULT_FONT_ID, HalfPoint::new(24), 100.0);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_line_breaker_single_word() {
        let mut fonts = FontRegistry::new(64);
        let mut breaker = LineBreaker::new(&mut fonts, DEFAULT_FONT_ID);

        let lines = breaker.break_lines("hello", DEFAULT_FONT_ID, HalfPoint::new(24), 1000.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], 0..5);
    }

    #[test]
    fn test_line_breaker_multiple_lines() {
        let mut fonts = FontRegistry::new(64);
        let mut breaker = LineBreaker::new(&mut fonts, DEFAULT_FONT_ID);

        let text = "word1 word2 word3";
        // Measure width of "word1 word2 "
        let width_2_words =
            breaker.measure_text("word1 word2 ", DEFAULT_FONT_ID, HalfPoint::new(24));

        let lines = breaker.break_lines(text, DEFAULT_FONT_ID, HalfPoint::new(24), width_2_words);
        assert_eq!(lines.len(), 2);
    }
}
