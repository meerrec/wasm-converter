/// WASM / JS API, only enabled on WASM
// Правка форка (ADR-0010): в апстриме модуль объявлен без гейта, и его
// #[wasm_bindgen]-функции (Pdf_*Sync) попадают в wasm-модуль любого
// потребителя printpdf. Наш workspace фичу `js-sys` не включает
// (default-features = false), так что гейт закрывает утечку.
#[cfg(feature = "js-sys")]
pub mod api;
/// WASM API Datastructures, can be used even on non-WASM targets
pub mod structs;
