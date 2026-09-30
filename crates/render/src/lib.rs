//! Общий `DisplayList` и painter — единый контракт между всеми форматами
//! и всеми бэкендами (`OffscreenCanvas` в WASM, PDF в native).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

mod color;
mod display_list;
mod font;
mod geometry;

pub use color::*;
pub use display_list::*;
pub use font::*;
pub use geometry::*;
