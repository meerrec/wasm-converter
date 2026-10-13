//! WASM-экспорт сабсетов Carlito для предпросмотра.
//!
//! Painter рисует текст браузерным canvas'ом, и гарнитура в кадре — это имя
//! семейства, которое уезжает в `ctx.font` (`painter/state.rs::font_css`).
//! Пока в области видимости воркера нет `FontFace` с этим именем, браузер
//! подставляет своё семейство, и ширины строк предпросмотра расходятся с
//! метриками раскладки и с PDF (там встроены те же сабсеты, `font.rs`).
//! Отсюда экспорт: байты уходят копией в JS, который регистрирует их как
//! `FontFace` ровно под именем кадра — `Carlito`.

use wasm_bindgen::prelude::*;

use doc_converter_render::font;

/// Байты сабсета Carlito для лица `face` — копией в JS.
///
/// Номера лиц повторяют порядок аксессоров [`font`]: 0 — regular, 1 — bold,
/// 2 — italic, 3 — bold-italic. Копия, а не вью в память wasm: `Uint8Array`
/// остаётся жить после возврата, а линейная память может вырасти и переехать.
///
/// # Errors
///
/// `face` вне `0..=3`: неизвестный номер молча означал бы не тот сабсет, а
/// расхождение шрифта — тихое, поэтому это ошибка с английским сообщением.
#[wasm_bindgen]
pub fn font_bytes(face: u32) -> Result<js_sys::Uint8Array, JsValue> {
    let bytes = match face {
        0 => font::default_font_bytes(),
        1 => font::bold_font_bytes(),
        2 => font::italic_font_bytes(),
        3 => font::bold_italic_font_bytes(),
        _ => {
            return Err(JsValue::from_str(&format!(
                "font face {face} is out of range: expected 0 (regular), 1 (bold), 2 (italic) \
                 or 3 (bold-italic)"
            )))
        }
    };
    Ok(js_sys::Uint8Array::from(bytes))
}
