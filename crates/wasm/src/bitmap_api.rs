use wasm_bindgen::prelude::*;
use web_sys::ImageBitmap;

use crate::painter_api::with_painter;

#[wasm_bindgen]
pub fn register_bitmap(id: u32, bmp: ImageBitmap) -> Result<(), JsValue> {
    with_painter(|p| {
        p.register_bitmap(id, bmp);
    })
}

#[wasm_bindgen]
pub fn drop_bitmap(id: u32) -> Result<bool, JsValue> {
    let mut out = false;
    with_painter(|p| {
        out = p.drop_bitmap(id);
    })?;
    Ok(out)
}
