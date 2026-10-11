//! Перенос строк для DOCX: разбивка абзаца на строки с учётом стилей.
//!
//! Использует [`doc_converter_render::text_measure`] для измерения ширин
//! (ADR-0005). Логика та же, что и для XLSX, чтобы точки разрыва не
//! разъезжались между canvas и PDF.
//!
//! Абзац Word — один поток текста, а не набор независимых отрезков: прогоны с
//! разным кеглем и гарнитурой складываются в одну строку, пока её суммарная
//! ширина помещается в полосу набора ([`LineBreaker::break_spanned`]). Поэтому
//! абзац режется по границам слов, а не по границам прогонов; метрики частей
//! строки приходят спанами [`MetricSpan`].

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

/// Отрезок текста абзаца с одной метрикой: шрифт и кегль.
///
/// Спаны покрывают общий текст абзаца без дыр и наложений; их границы — стыки
/// прогонов, поэтому они всегда лежат на границах символов.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricSpan {
    /// Байтовый диапазон в общем тексте абзаца.
    pub range: Range<usize>,
    /// Шрифт отрезка в реестре.
    pub font_id: FontId,
    /// Кегль отрезка в полупунктах.
    pub size: HalfPoint,
}

/// Строка после переноса составного текста.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedLine {
    /// Байтовый диапазон строки в общем тексте абзаца.
    pub range: Range<usize>,
    /// Ширина строки — сумма ширин её частей.
    pub width: f32,
    /// Наибольший кегль среди частей строки: по нему Word считает высоту строки,
    /// а не по кеглю первого прогона.
    pub max_size: HalfPoint,
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

    /// Шрифт по имени семейства (`w:rFonts`): неизвестное имя и `None` — шрифт
    /// по умолчанию, как в [`FontRegistry::resolve_family`].
    #[must_use]
    pub fn resolve_family(&self, family: Option<&str>) -> FontId {
        self.fonts.resolve_family(family)
    }

    /// Имя, которым строка с такой гарнитурой уходит в кадр.
    ///
    /// То же имя, по которому [`Self::resolve_family`] нашёл лицо: измеренное и
    /// нарисованное не разъезжаются (см. [`FontRegistry::drawable_family`]).
    #[must_use]
    pub fn drawable_family<'b>(&'b self, family: Option<&'b str>) -> Option<&'b str> {
        self.fonts.drawable_family(family)
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

    /// Ширина диапазона составного текста: сумма ширин его частей.
    ///
    /// Каждая часть меряется своим шрифтом и кеглем — ширина строки не может
    /// быть посчитана одним лицом, если внутри неё их несколько.
    #[must_use]
    pub fn measure_spanned(
        &mut self,
        text: &str,
        spans: &[MetricSpan],
        range: Range<usize>,
    ) -> f32 {
        let mut width = 0.0;
        for span in spans {
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            if start >= end {
                continue;
            }
            width += self.measure_text(&text[start..end], span.font_id, span.size);
        }
        width
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

    /// Разбить составной текст на строки от байта `from`.
    ///
    /// Ширина строки — сумма ширин её частей ([`Self::measure_spanned`]): перенос
    /// идёт по суммарной ширине, поэтому прогоны с разным кеглем, начертанием и
    /// гарнитурой складываются в одну строку, а не ломаются каждый сам по себе.
    /// Интервалы между словами меряются тем прогоном, в котором записаны.
    ///
    /// Правила переноса — те же, что у [`text_measure::break_lines`]: слова не
    /// разрываются, слишком широкое слово режется по символам, `\n` форсирует
    /// разрыв, пробелы по краям строки в диапазон не входят.
    ///
    /// `limit` ограничивает число строк: первую строку абзаца отрезают своей,
    /// более узкой полосой набора (`w:firstLine`/`w:hanging`), и остальной текст
    /// разбирается со следующего за ней байта.
    #[must_use]
    pub fn break_spanned(
        &mut self,
        text: &str,
        spans: &[MetricSpan],
        max_width: f32,
        from: usize,
        limit: Option<usize>,
    ) -> Vec<SpannedLine> {
        let mut lines = Vec::new();
        if spans.is_empty() || from >= text.len() || max_width <= 0.0 {
            return lines;
        }

        // Байтовый диапазон собираемой строки: от первого символа первого слова
        // до последнего символа последнего; пробелы в диапазон не входят.
        let mut line_start: Option<usize> = None;
        let mut line_end = 0usize;
        let mut line_width = 0.0;

        let mut byte = from;
        while byte < text.len() {
            let ch = text[byte..]
                .chars()
                .next()
                .expect("byte is a char boundary");
            if ch == '\n' {
                byte += 1;
                if let Some(start) = line_start.take() {
                    lines.push(self.spanned_line(spans, start..line_end, line_width));
                    if limit.is_some_and(|limit| lines.len() >= limit) {
                        return lines;
                    }
                }
                line_width = 0.0;
                continue;
            }
            if ch.is_whitespace() {
                byte += ch.len_utf8();
                continue;
            }

            // Слово — до первого пробела или перевода строки.
            let word_start = byte;
            let mut word_end = byte + ch.len_utf8();
            while word_end < text.len() {
                let next = text[word_end..]
                    .chars()
                    .next()
                    .expect("byte is a char boundary");
                if next.is_whitespace() {
                    break;
                }
                word_end += next.len_utf8();
            }
            let width = self.measure_spanned(text, spans, word_start..word_end);
            byte = word_end;

            // Слово встаёт в текущую строку; слишком широкое для пустой строки
            // режется посимвольно, для занятой — открывает новую.
            let fits = width <= max_width;
            match line_start {
                None => {
                    if fits {
                        line_start = Some(word_start);
                        line_end = word_end;
                        line_width = width;
                    } else {
                        self.break_wide_word(
                            text,
                            spans,
                            word_start..word_end,
                            max_width,
                            &mut lines,
                        );
                    }
                }
                Some(start) => {
                    // Пробелы и табуляции между словами занимают место в строке:
                    // их ширина — часть суммарной.
                    let gap = self.measure_spanned(text, spans, line_end..word_start);
                    if line_width + gap + width <= max_width {
                        line_width += gap + width;
                        line_end = word_end;
                    } else {
                        lines.push(self.spanned_line(spans, start..line_end, line_width));
                        if limit.is_some_and(|limit| lines.len() >= limit) {
                            return lines;
                        }
                        line_start = None;
                        line_width = 0.0;
                        if fits {
                            line_start = Some(word_start);
                            line_end = word_end;
                            line_width = width;
                        } else {
                            self.break_wide_word(
                                text,
                                spans,
                                word_start..word_end,
                                max_width,
                                &mut lines,
                            );
                        }
                    }
                }
            }
        }
        if let Some(start) = line_start {
            lines.push(self.spanned_line(spans, start..line_end, line_width));
        }
        lines
    }

    /// Собрать строку: диапазон, измеренную ширину и наибольший кегль.
    fn spanned_line(
        &mut self,
        spans: &[MetricSpan],
        range: Range<usize>,
        width: f32,
    ) -> SpannedLine {
        SpannedLine {
            max_size: largest_size(spans, &range),
            range,
            width,
        }
    }

    /// Режет слово шире `max_width` по символам; каждый кусок непустой.
    fn break_wide_word(
        &mut self,
        text: &str,
        spans: &[MetricSpan],
        word: Range<usize>,
        max_width: f32,
        lines: &mut Vec<SpannedLine>,
    ) {
        let mut chunk_start: Option<usize> = None;
        let mut chunk_end = 0usize;
        let mut chunk_width = 0.0;

        for (byte, ch) in text[word.clone()].char_indices() {
            let start = word.start + byte;
            let end = start + ch.len_utf8();
            let advance = self.measure_spanned(text, spans, start..end);
            if chunk_start.is_some() && chunk_width + advance > max_width {
                let start = chunk_start.expect("chunk has a start");
                lines.push(self.spanned_line(spans, start..chunk_end, chunk_width));
                chunk_start = None;
                chunk_width = 0.0;
            }
            // Один символ шире строки всё равно образует кусок.
            if chunk_start.is_none() {
                chunk_start = Some(start);
            }
            chunk_end = end;
            chunk_width += advance;
        }
        if let Some(start) = chunk_start {
            lines.push(self.spanned_line(spans, start..chunk_end, chunk_width));
        }
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

/// Наибольший кегль среди спанов, пересекающих диапазон.
///
/// Высота строки в Word — по самому крупному прогону строки, а не по первому:
/// иначе крупное слово провалилось бы в высоту соседей. Диапазон без спанов
/// (пустая строка) даёт нулевой кегль — высоту ей считать не по чему.
#[must_use]
fn largest_size(spans: &[MetricSpan], range: &Range<usize>) -> HalfPoint {
    spans
        .iter()
        .filter(|span| span.range.start < range.end && range.start < span.range.end)
        .map(|span| span.size)
        .max_by_key(|size| size.value())
        .unwrap_or(HalfPoint::new(0))
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
