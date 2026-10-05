//! doc-converter-render — painter, DisplayList, SAB ring, геометрия и метрики текста.
//!
//! Фаза 2: OffscreenCanvas painter + двойная буферизация DisplayList.
//! Спринт 5.5: `font`, `text_measure`, `geometry`, `viewport`, `hit_test` —
//! общая граница с `xlsx`/`docx` (ADR-0003); `canvas` — жизненный цикл
//! OffscreenCanvas-контекста (только wasm32).

pub mod display_list;
pub mod font;
pub mod geometry;
pub mod hit_test;
pub mod painter;
pub mod sab;
pub mod text_measure;
pub mod viewport;

#[cfg(target_arch = "wasm32")]
pub mod canvas;

pub use display_list::{
    Color, DecodeError, DisplayList, DisplayListReader, DrawCommand, LineStyle, StringRef,
    TextAlign, TextBaseline,
};
