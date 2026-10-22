//! WASM-экспорт TextLayer: позиционированный текст открытого DOCX.
//!
//! Геометрия приходит из раскладки, которую [`docx_open`](crate::docx_api::docx_open)
//! уже посчитал: второй раз документ не раскладывается и не парсится. Страница
//! запрашивается по требованию — виртуализация DOM-слоя грузит только окно
//! видимых страниц, — а полный слой нужен семантическому дереву и Ctrl+A.
//!
//! Поля wire-типов — camelCase (`doc_converter_core::wire`), TS-типы
//! генерируются из того же источника (ADR-0028).

use wasm_bindgen::prelude::*;

use doc_converter_core::wire::TextLayerWire;
use doc_converter_docx::text_layer::{text_layer, text_layer_page};

use crate::docx_api::{to_js, with_layout};

/// Сериализовать wire-тип в JS с `null` вместо отсутствующих `Option`-полей.
///
/// `serde_wasm_bindgen` по умолчанию отдаёт `None` как `undefined`; контракт
/// `wire.ts` и TextLayer обещают `null`, и проверки `table !== null` в TS
/// должны видеть именно его.
fn to_value<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true))
        .map_err(to_js)
}

/// TextLayer одной страницы (0-based).
///
/// # Errors
/// Документ не открыт; страница вне диапазона; сериализация в JS не удалась.
#[wasm_bindgen]
pub fn docx_get_text_layer(page: u32) -> Result<JsValue, JsValue> {
    let (layer, pages) = with_layout(|layout| {
        (
            text_layer_page(layout, page),
            u32::try_from(layout.pages.len()).unwrap_or(u32::MAX),
        )
    })
    .ok_or_else(|| JsValue::from_str("no docx document is open"))?;
    let layer = layer.ok_or_else(|| {
        JsValue::from_str(&format!(
            "page {page} is out of range: document has {pages} pages"
        ))
    })?;
    to_value(&layer)
}

/// TextLayer всего документа.
///
/// Для больших документов это дорого: слой содержит run'ы всех страниц.
/// Виртуализированный просмотрщик берёт [`docx_get_text_layer`] постранично,
/// а полный слой — для семантического дерева и копирования целиком.
///
/// # Errors
/// Документ не открыт; сериализация в JS не удалась.
#[wasm_bindgen]
pub fn docx_get_full_text_layer() -> Result<JsValue, JsValue> {
    let layer: TextLayerWire =
        with_layout(text_layer).ok_or_else(|| JsValue::from_str("no docx document is open"))?;
    to_value(&layer)
}
