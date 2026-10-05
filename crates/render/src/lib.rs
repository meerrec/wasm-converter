//! doc-converter-render — painter, DisplayList, SAB ring.
//!
//! Фаза 2: OffscreenCanvas painter + двойная буферизация DisplayList.

pub mod display_list;
pub mod painter;
pub mod sab;

pub use display_list::{
    Color, DecodeError, DisplayList, DisplayListReader, DrawCommand, StringRef, TextAlign,
    TextBaseline,
};
