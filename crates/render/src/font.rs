use std::collections::HashMap;
use std::num::NonZeroUsize;

use lru::LruCache;

pub type FontId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextKey { pub font_id: FontId, pub size_bits: u32, pub text_hash: u64 }

#[derive(Debug, Clone, Copy, Default)]
pub struct TextMetrics { pub width: f32, pub ascent: f32, pub descent: f32, pub line_height: f32 }

/// Реестр TTF/OTF-шрифтов + LRU-кэш метрик.
///
/// TODO (Фаза 4): подключить `rustybuzz` для shaping/kerning/ligatures.
/// TODO (Фаза 6): `unicode-bidi` для RTL/CJK.
pub struct FontRegistry {
    faces: HashMap<FontId, Vec<u8>>,
    cache: LruCache<TextKey, TextMetrics>,
}

impl FontRegistry {
    pub fn new(cache_size: usize) -> Self {
        Self {
            faces: HashMap::new(),
            cache: LruCache::new(NonZeroUsize::new(cache_size.max(1)).unwrap()),
        }
    }

    pub fn register(&mut self, id: FontId, bytes: Vec<u8>) {
        self.faces.insert(id, bytes);
    }

    pub fn has(&self, id: FontId) -> bool { self.faces.contains_key(&id) }

    pub fn measure(&mut self, font_id: FontId, size_px: f32, text: &str) -> TextMetrics {
        let key = TextKey {
            font_id,
            size_bits: size_px.to_bits(),
            text_hash: fnv1a(text.as_bytes()),
        };
        if let Some(m) = self.cache.get(&key) { return *m; }

        // TODO (Фаза 4): реальные метрики через ttf-parser + rustybuzz.
        let n = text.chars().count() as f32;
        let m = TextMetrics {
            width: n * size_px * 0.55,
            ascent: size_px * 0.8,
            descent: size_px * 0.2,
            line_height: size_px * 1.2,
        };
        self.cache.put(key, m);
        m
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes { h ^= u64::from(*b); h = h.wrapping_mul(0x100_0000_01b3); }
    h
}
