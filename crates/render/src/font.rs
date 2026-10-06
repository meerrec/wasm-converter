//! Реестр TTF/OTF-шрифтов и метрик глифов на `skrifa`.
//!
//! Источник ширин — таблицы шрифта, а не оценка: advance берётся из `hmtx`,
//! ascent/descent/line gap — из `OS/2`/`hhea` (ADR-0002, ADR-0005). Метрика
//! одного глифа кэшируется в LRU по `(font_id, glyph_id, size)`, ширина строки
//! собирается суммой кэшированных advance'ов.
//!
//! Политика подстановки (решение Спринта 5.5): id без зарегистрированного
//! шрифта измеряется шрифтом по умолчанию, это не ошибка — у реестра всегда
//! есть хотя бы один рабочий шрифт, и измерение не может «не выйти».

use std::collections::HashMap;
use std::num::NonZeroUsize;

use lru::LruCache;
use skrifa::instance::{Location, Size};
use skrifa::{FontRef, MetadataProvider};

/// Идентификатор шрифта в реестре.
pub type FontId = u32;

/// Шрифт по умолчанию: метрический запас для вызовов с незарегистрированным id.
pub const DEFAULT_FONT_ID: FontId = 0;

/// Carlito, подрезанный до ASCII + Latin-1 + кириллицы и знаков пунктуации.
///
/// Carlito метрически совместим с Calibri — на ней построена система ширин
/// колонок Excel («0» в 11 pt = 7 px), поэтому оценка `#####` остаётся
/// согласованной с шириной колонок. Лицензия SIL OFL 1.1 — `fonts/OFL.txt`.
const DEFAULT_FONT: &[u8] = include_bytes!("fonts/carlito-subset.ttf");

/// Байты шрифта по умолчанию.
///
/// Нужен потребителям, которые встраивают тот же шрифт в свой формат вывода
/// (PDF-экспорт, `crates/pdf`): держать вторую копию `include_bytes!` они не
/// должны, иначе шрифты экрана и экспорта разъедутся.
#[must_use]
pub fn default_font_bytes() -> &'static [u8] {
    DEFAULT_FONT
}

/// Ключ LRU-кэша: метрика зависит от шрифта, глифа и кегля.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: FontId,
    /// Ключ по символу, а не по id глифа: без shaping символ → глиф
    /// детерминирован, а `cmap`-поиск на горячем пути не нужен. Кэшируется
    /// при этом метрика одного глифа — по ADR-0005.
    ch: u32,
    size_bits: u32,
}

/// Метрика одного глифа, все величины — в пикселях при `size_px`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphMetrics {
    pub advance: f32,
    /// Расстояние от базовой линии до верха строки (положительное).
    pub ascender: f32,
    /// Расстояние от базовой линии до низа строки (отрицательное).
    pub descender: f32,
    /// Рекомендуемый зазор между строками.
    pub line_gap: f32,
}

/// Метрика строки текста: сумма advance'ов глифов + линейные метрики шрифта.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_height: f32,
}

/// Ошибка регистрации шрифта.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("invalid font data: {0}")]
    InvalidFont(String),
}

/// Реестр шрифтов: байты по [`FontId`] и LRU-кэши метрик.
pub struct FontRegistry {
    faces: HashMap<FontId, Vec<u8>>,
    glyph_cache: LruCache<GlyphKey, f32>,
    /// `(font_id, size_bits) -> (ascent, descent, line_gap)`; линейные метрики
    /// у шрифта одни на кегль, отдельный HashMap их дешевле LRU по глифам.
    line_cache: HashMap<(FontId, u32), (f32, f32, f32)>,
}

impl FontRegistry {
    /// Создаёт реестр с уже зарегистрированным шрифтом по умолчанию.
    ///
    /// `cache_size == 0` трактуется как минимальный ненулевой размер LRU.
    #[must_use]
    pub fn new(cache_size: usize) -> Self {
        let mut registry = Self {
            faces: HashMap::new(),
            glyph_cache: LruCache::new(NonZeroUsize::new(cache_size).unwrap_or(NonZeroUsize::MIN)),
            line_cache: HashMap::new(),
        };
        registry
            .faces
            .insert(DEFAULT_FONT_ID, DEFAULT_FONT.to_vec());
        registry
    }

    /// Регистрирует (или заменяет) шрифт под `id`.
    ///
    /// Байты разбираются сразу: повреждённый файл в реестр не попадает, и уже
    /// зарегистрированный под этим id шрифт остаётся на месте. Замена чистит
    /// кэши метрик — старые advance'ы к новому шрифту отношения не имеют, а
    /// выборочно выселять LRU по id дороже полной очистки при редкой замене.
    ///
    /// # Errors
    /// [`FontError::InvalidFont`], если байты — не TTF/OTF.
    pub fn register(&mut self, id: FontId, bytes: Vec<u8>) -> Result<(), FontError> {
        if FontRef::new(&bytes).is_err() {
            return Err(FontError::InvalidFont("not a TTF/OTF font".to_owned()));
        }
        self.faces.insert(id, bytes);
        self.glyph_cache.clear();
        self.line_cache.clear();
        Ok(())
    }

    /// Есть ли шрифт под `id`.
    #[must_use]
    pub fn has(&self, id: FontId) -> bool {
        self.faces.contains_key(&id)
    }

    /// Метрика одного глифа: advance из `hmtx`, линейные — из таблиц шрифта.
    ///
    /// Незарегистрированный id измеряется шрифтом по умолчанию. Символ без
    /// глифа в шрифте получает консервативную ширину `0.55 em` — глифа нет,
    /// значит, и настоящего advance нет, а падать из-за эмодзи измерение не
    /// должно.
    #[must_use]
    pub fn glyph_metrics(&mut self, id: FontId, size_px: f32, ch: char) -> GlyphMetrics {
        let (ascender, descender, line_gap) = self.line_metrics(id, size_px);
        let advance = self.glyph_advance(id, size_px, ch);
        GlyphMetrics {
            advance,
            ascender,
            descender,
            line_gap,
        }
    }

    /// Ширина строки и линейные метрики шрифта.
    ///
    /// `FontRef` разбирается один раз на строку, а не на символ: разбор таблиц
    /// шрифта на каждый глиф съел бы выигрыш LRU-кэша.
    #[must_use]
    pub fn measure(&mut self, id: FontId, size_px: f32, text: &str) -> TextMetrics {
        if size_px <= 0.0 {
            return TextMetrics::default();
        }
        let id = if self.has(id) { id } else { DEFAULT_FONT_ID };
        let (ascender, descender, line_gap) = self.line_metrics(id, size_px);

        let bytes = &self.faces[&id];
        let font = match FontRef::new(bytes) {
            Ok(font) => font,
            // Зарегистрированные шрифты валидны, встроенный — тоже: сюда код
            // попасть не может, но измерение обязано быть тотальным.
            Err(_) => return estimate(text, size_px),
        };
        let charmap = font.charmap();
        let location = Location::default();
        let glyph_metrics =
            skrifa::metrics::GlyphMetrics::new(&font, Size::new(size_px), &location);

        let mut width = 0.0;
        for ch in text.chars() {
            let key = GlyphKey {
                font: id,
                ch: u32::from(ch),
                size_bits: size_px.to_bits(),
            };
            // `peek`, а не `get`: на горячем пути промахов почти нет, а
            // продвижение элемента в начало LRU — лишняя работа на каждый глиф.
            let advance = if let Some(advance) = self.glyph_cache.peek(&key) {
                *advance
            } else {
                let glyph = charmap.map(ch);
                let advance = glyph
                    .and_then(|glyph| glyph_metrics.advance_width(glyph))
                    .unwrap_or(FALLBACK_ADVANCE_EM * size_px);
                self.glyph_cache.put(key, advance);
                advance
            };
            width += advance;
        }

        TextMetrics {
            width,
            ascent: ascender,
            descent: descender,
            line_height: ascender - descender + line_gap,
        }
    }

    fn glyph_advance(&mut self, id: FontId, size_px: f32, ch: char) -> f32 {
        if size_px <= 0.0 {
            return 0.0;
        }
        let id = if self.has(id) { id } else { DEFAULT_FONT_ID };
        let bytes = &self.faces[&id];
        let font = match FontRef::new(bytes) {
            Ok(font) => font,
            Err(_) => return FALLBACK_ADVANCE_EM * size_px,
        };
        let glyph = font.charmap().map(ch);
        let key = GlyphKey {
            font: id,
            ch: u32::from(ch),
            size_bits: size_px.to_bits(),
        };
        if let Some(advance) = self.glyph_cache.get(&key) {
            return *advance;
        }
        let location = Location::default();
        let glyph_metrics =
            skrifa::metrics::GlyphMetrics::new(&font, Size::new(size_px), &location);
        let advance = glyph
            .and_then(|glyph| glyph_metrics.advance_width(glyph))
            .unwrap_or(FALLBACK_ADVANCE_EM * size_px);
        self.glyph_cache.put(key, advance);
        advance
    }

    fn line_metrics(&mut self, id: FontId, size_px: f32) -> (f32, f32, f32) {
        if size_px <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let id = if self.has(id) { id } else { DEFAULT_FONT_ID };
        let key = (id, size_px.to_bits());
        if let Some(metrics) = self.line_cache.get(&key) {
            return *metrics;
        }
        let bytes = &self.faces[&id];
        let metrics = match FontRef::new(bytes) {
            Ok(font) => {
                let location = Location::default();
                let metrics = skrifa::metrics::Metrics::new(&font, Size::new(size_px), &location);
                (metrics.ascent, metrics.descent, metrics.leading)
            }
            // Битый шрифт в реестр не попадает, но измерение тотально.
            Err(_) => return estimate_line(size_px),
        };
        let metrics = if metrics.0 > 0.0 {
            metrics
        } else {
            estimate_line(size_px)
        };
        self.line_cache.insert(key, metrics);
        metrics
    }
}

/// Консервативная ширина символа, глифа для которого в шрифте нет.
const FALLBACK_ADVANCE_EM: f32 = 0.55;

/// Консервативные линейные метрики: доли em, как в старых оценках.
fn estimate_line(size_px: f32) -> (f32, f32, f32) {
    (size_px * 0.8, -size_px * 0.2, size_px * 0.2)
}

/// Оценка строки целиком — последний рубеж, если шрифт вдруг не разобрался.
fn estimate(text: &str, size_px: f32) -> TextMetrics {
    let (ascent, descent, line_gap) = estimate_line(size_px);
    #[allow(clippy::cast_precision_loss)]
    let chars = text.chars().count() as f32;
    TextMetrics {
        width: chars * size_px * FALLBACK_ADVANCE_EM,
        ascent,
        descent,
        line_height: ascent - descent + line_gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PX_11PT: f32 = 11.0 * 96.0 / 72.0;
    /// Advance «0» в Carlito: 1038/2048 em (эталон снят `hb-shape`).
    const ZERO_11PT: f32 = 1038.0 / 2048.0 * 11.0 * 96.0 / 72.0;

    #[test]
    fn default_font_is_registered() {
        let registry = FontRegistry::new(64);
        assert!(registry.has(DEFAULT_FONT_ID));
    }

    #[test]
    fn register_accepts_font_and_rejects_garbage() {
        let mut registry = FontRegistry::new(64);
        assert!(registry.register(7, DEFAULT_FONT.to_vec()).is_ok());
        assert!(registry.has(7));

        let result = registry.register(8, vec![0u8; 64]);
        assert!(matches!(result, Err(FontError::InvalidFont(_))));
        assert!(!registry.has(8));
    }

    #[test]
    fn digit_advance_matches_carlito_tables() {
        let mut registry = FontRegistry::new(64);
        let advance = registry
            .glyph_metrics(DEFAULT_FONT_ID, PX_11PT, '0')
            .advance;
        // Допуск покрывает квантование 16.16-фиксированной точки внутри skrifa.
        assert!((advance - ZERO_11PT).abs() < 2e-2, "{advance}");

        let text = registry.measure(DEFAULT_FONT_ID, PX_11PT, "0123456789");
        assert!((text.width - ZERO_11PT * 10.0).abs() < 0.2);
    }

    #[test]
    fn cyrillic_measures_without_panic() {
        let mut registry = FontRegistry::new(64);
        for ch in ['Ф', 'ё'] {
            assert!(registry.glyph_metrics(DEFAULT_FONT_ID, PX_11PT, ch).advance > 0.0);
        }
    }

    #[test]
    fn missing_glyph_falls_back_without_panic() {
        let mut registry = FontRegistry::new(64);
        let advance = registry
            .glyph_metrics(DEFAULT_FONT_ID, PX_11PT, '\u{1F600}')
            .advance;
        assert!((advance - FALLBACK_ADVANCE_EM * PX_11PT).abs() < 1e-4);
    }

    #[test]
    fn line_metrics_are_sane() {
        let mut registry = FontRegistry::new(64);
        let glyph = registry.glyph_metrics(DEFAULT_FONT_ID, PX_11PT, 'a');
        let metrics = registry.measure(DEFAULT_FONT_ID, PX_11PT, "");
        assert_eq!(metrics.width, 0.0);
        assert!(metrics.ascent > 0.0);
        assert!(metrics.descent < 0.0);
        let expected = glyph.ascender - glyph.descender + glyph.line_gap;
        assert!((metrics.line_height - expected).abs() < 1e-4);
    }

    #[test]
    fn zero_and_negative_sizes_measure_zero() {
        let mut registry = FontRegistry::new(64);
        assert_eq!(
            registry.measure(DEFAULT_FONT_ID, 0.0, "123"),
            TextMetrics::default()
        );
        assert_eq!(
            registry.measure(DEFAULT_FONT_ID, -1.0, "123"),
            TextMetrics::default()
        );
    }

    #[test]
    fn lru_repeat_is_stable_and_min_size_works() {
        let mut registry = FontRegistry::new(0);
        let first = registry.measure(DEFAULT_FONT_ID, PX_11PT, "Calibri");
        let second = registry.measure(DEFAULT_FONT_ID, PX_11PT, "Calibri");
        assert_eq!(first, second);
    }
}
