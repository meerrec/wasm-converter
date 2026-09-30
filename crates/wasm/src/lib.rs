//! WASM-биндинги. Единственное место, где допускается `unsafe` —
//! и то только внутри `wasm-bindgen`-макросов.
#![deny(clippy::pedantic)]

use wasm_bindgen::prelude::*;

use doc_converter_render::{DisplayList, FontRegistry};

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    #[cfg(feature = "tracing")]
    tracing_wasm::set_as_global_default();
}

// ───────────────────── DOCX ─────────────────────

/// Открывает DOCX и отдаёт сводку по документу.
///
/// # Errors
/// Если байты не OOXML-пакет или документ не разбирается.
#[wasm_bindgen]
pub fn open_docx(bytes: &[u8]) -> Result<JsValue, JsValue> {
    let doc = doc_converter_docx::open(bytes.to_vec()).map_err(to_js_err)?;
    Ok(serde_wasm_bindgen::to_value(&serde_json::json!({
        "pageCount": doc.page_count(),
    }))?)
}

/// # Errors
/// Пока всегда: экспорт в PDF не реализован (Фаза 4).
#[wasm_bindgen]
pub fn export_docx_to_pdf(_opts_json: &str) -> Result<Vec<u8>, JsValue> {
    Err(JsValue::from_str(
        "export_docx_to_pdf: not yet implemented (Фаза 4)",
    ))
}

// ───────────────────── XLSX ─────────────────────

/// Открывает XLSX и отдаёт список листов.
///
/// # Errors
/// Если байты не OOXML-пакет или книга не разбирается.
#[wasm_bindgen]
pub fn open_xlsx(bytes: &[u8]) -> Result<JsValue, JsValue> {
    let wb = doc_converter_xlsx::open(bytes.to_vec()).map_err(to_js_err)?;
    Ok(serde_wasm_bindgen::to_value(&serde_json::json!({
        "sheets": wb.sheet_names(),
    }))?)
}

/// # Errors
/// Пока всегда: экспорт листа в PDF не реализован (Фаза 4).
#[wasm_bindgen]
pub fn export_xlsx_sheet_to_pdf(_opts_json: &str) -> Result<Vec<u8>, JsValue> {
    Err(JsValue::from_str(
        "export_xlsx_sheet_to_pdf: not yet implemented (Фаза 4)",
    ))
}

/// # Errors
/// Пока всегда: экспорт книги в PDF не реализован (Фаза 4).
#[wasm_bindgen]
pub fn export_xlsx_workbook_to_pdf(_opts_json: &str) -> Result<Vec<u8>, JsValue> {
    Err(JsValue::from_str(
        "export_xlsx_workbook_to_pdf: not yet implemented (Фаза 4)",
    ))
}

// ───────────────────── Render ─────────────────────

/// Собирает `DisplayList` текущего документа.
///
/// # Errors
/// Если `DisplayList` не сериализуется в JSON.
#[wasm_bindgen]
pub fn build_display_list(_req_json: &str) -> Result<String, JsValue> {
    // TODO (Фаза 3/6): построить DisplayList из текущего документа.
    let dl = DisplayList::empty();
    serde_json::to_string(&dl).map_err(to_js_err)
}

/// Рисует `DisplayList` в `OffscreenCanvas`.
///
/// # Errors
/// TODO (Фаза 2): ошибки painter'а.
#[wasm_bindgen]
pub fn paint_display_list_to_offscreen(
    _ctx: web_sys::OffscreenCanvasRenderingContext2d,
    _dl_json: &str,
) -> Result<(), JsValue> {
    // TODO (Фаза 2): реализовать painter поверх OffscreenCanvas.
    Ok(())
}

#[wasm_bindgen]
pub fn resize_canvas(_dpr: f32) {
    // TODO (Фаза 2): уведомить painter о смене DPR.
}

/// Hit-testing по координатам в CSS-пикселях.
///
/// # Errors
/// TODO (Фаза 2): ошибки разрешения координат.
#[wasm_bindgen]
pub fn hit_test(_x: f32, _y: f32) -> Result<JsValue, JsValue> {
    Ok(JsValue::NULL)
}

// ───────────────────── helper ─────────────────────

fn to_js_err<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[allow(dead_code)]
fn _ensure_registry_is_used() {
    let _ = FontRegistry::new(64);
}
