use wasm_bindgen::prelude::*;

/// Аллоцирует SharedArrayBuffer под ring с двумя слотами по `slot_capacity`.
/// Требует crossOriginIsolated.
#[wasm_bindgen]
pub fn alloc_sab(slot_capacity: u32) -> Result<js_sys::SharedArrayBuffer, JsValue> {
    let total = doc_converter_render::sab::HEADER_BYTES + (slot_capacity as usize) * 2;
    Ok(js_sys::SharedArrayBuffer::new(total as u32))
}

#[wasm_bindgen]
pub fn sab_total_bytes(slot_capacity: u32) -> u32 {
    (doc_converter_render::sab::HEADER_BYTES + (slot_capacity as usize) * 2) as u32
}
