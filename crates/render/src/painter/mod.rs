pub mod state;
pub mod text;

#[cfg(target_arch = "wasm32")]
pub mod bitmap_cache;
#[cfg(target_arch = "wasm32")]
pub mod chart;
#[cfg(target_arch = "wasm32")]
pub mod painter_2d;

#[cfg(target_arch = "wasm32")]
pub use painter_2d::{PaintStats, Painter2D};
