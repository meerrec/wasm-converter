use wasm_bindgen::prelude::*;
use web_sys::ImageBitmap;

use doc_converter_render::canvas::OffscreenPainter;

#[wasm_bindgen]
pub fn register_bitmap(id: u32, bmp: ImageBitmap) -> Result<(), JsValue> {
    OffscreenPainter::with(|painter| painter.register_bitmap(id, bmp))
}

#[wasm_bindgen]
pub fn drop_bitmap(id: u32) -> Result<bool, JsValue> {
    OffscreenPainter::with(|painter| painter.drop_bitmap(id))
}
