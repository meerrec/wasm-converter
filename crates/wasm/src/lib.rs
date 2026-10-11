//! WASM-биндинги. Все экспорты завязаны на `web-sys`/`wasm-bindgen` и
//! собираются только под `wasm32`. На нативном таргете крейт пуст — так
//! `cargo clippy --workspace --all-targets` и `cargo test --workspace`
//! проходят без выбора таргета.

#![cfg(target_arch = "wasm32")]

pub mod bitmap_api;
pub mod painter_api;
pub mod sab_api;
pub mod xlsx_api;
