//! Общие хелперы wasm-крейтов: потоковый вход и транспорты кадра DisplayList.
//!
//! Три форматных модуля (`crates/wasm`, `crates/docx-wasm`, `crates/pptx-wasm`)
//! дублировали один и тот же код: реестр потоков (`stream_api.rs` ×3) и обвязку
//! сборки кадра `with_frame_bytes` с тремя транспортами (`_into`/`_sab`/Vec).
//! Здесь живёт канон; форматные крейты оставляют только свои `thread_local!`-пары
//! и `#[wasm_bindgen]`-обёртки.
//!
//! **`#[wasm_bindgen]`-функций здесь нет намеренно** (см. `Cargo.toml`): биндинги
//! из зависимости попали бы в `.d.ts` всех модулей и уронили бы гейт
//! `check exports` в CI. Ошибки `JsValue` дословно те же, что были в копиях, —
//! TS-сторона различает их по тексту.

#![cfg(target_arch = "wasm32")]
#![deny(clippy::pedantic)]
#![forbid(unsafe_code)]

pub mod frame;
pub mod streams;
