#![cfg(target_arch = "wasm32")]

//! Кооперативная отмена кадра XLSX (ADR-0032): взведённый флаг обрывает
//! сборку на первой полосе, снятый — возвращает воркер в рабочий режим.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Маленькая книга из общего корпуса фикстур.
const BOOK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/xlsx/cf-cell-is.xlsx"
));

/// Окно и оформление кадра в том виде, в каком их шлёт воркер.
fn frame_args() -> (JsValue, JsValue) {
    let viewport = js_sys::Object::new();
    for (name, value) in [
        ("scrollX", 0.0),
        ("scrollY", 0.0),
        ("width", 800.0),
        ("height", 600.0),
        ("scale", 1.0),
    ] {
        js_sys::Reflect::set(
            &viewport,
            &JsValue::from_str(name),
            &JsValue::from_f64(value),
        )
        .expect("поле окна записывается");
    }
    // Пустой объект — оформление по умолчанию: недостающие поля
    // `RenderOptionsWire` заполняет `serde(default)`.
    (viewport.into(), js_sys::Object::new().into())
}

#[wasm_bindgen_test]
fn cancel_stops_frame_build_and_worker_stays_healthy() {
    doc_converter_wasm::xlsx_api::xlsx_open(BOOK.to_vec()).expect("книга открывается");

    let (viewport, options) = frame_args();
    doc_converter_wasm::xlsx_api::set_cancel(true);
    let err =
        doc_converter_wasm::xlsx_api::xlsx_build_display_list(0, viewport.clone(), options.clone())
            .expect_err("взведённый флаг отменяет сборку кадра");
    assert!(
        err.as_string()
            .is_some_and(|m| m.starts_with("[cancelled]")),
        "ошибка: {err:?}"
    );

    doc_converter_wasm::xlsx_api::set_cancel(false);
    let frame = doc_converter_wasm::xlsx_api::xlsx_build_display_list(0, viewport, options)
        .expect("после снятия флага кадр собирается");
    assert!(frame.len() >= 20, "кадр несёт заголовок DisplayList");

    doc_converter_wasm::xlsx_api::xlsx_close();
}
