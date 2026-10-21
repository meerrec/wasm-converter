#![cfg(target_arch = "wasm32")]

//! Проверки потокового входа PPTX (ADR-0031): чанки копятся в ленивом модуле,
//! парс — по `stream_open_pptx` без сборки контигуального буфера.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Двухслайдовая фикстура из общего корпуса.
const TWO_SLIDES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/pptx/simple/two_slides.pptx"
));

#[wasm_bindgen_test]
fn stream_open_parses_chunked_presentation() {
    let id = doc_converter_pptx_wasm::stream_api::stream_new();
    let mut total = 0u64;
    for chunk in TWO_SLIDES.chunks(7) {
        total =
            doc_converter_pptx_wasm::stream_api::stream_append(id, chunk).expect("чанк дописан");
    }
    assert_eq!(total, TWO_SLIDES.len() as u64);

    let slides = doc_converter_pptx_wasm::stream_api::stream_open_pptx(id)
        .expect("презентация разобрана из чанков");
    assert!(slides.is_array(), "ответ — массив слайдов");
    let length = js_sys::Reflect::get(&slides, &JsValue::from_str("length"))
        .expect("length читается")
        .as_f64()
        .expect("length — число");
    assert_eq!(length, 2.0, "у презентации два слайда");
}

#[wasm_bindgen_test]
fn empty_stream_is_an_open_error() {
    let id = doc_converter_pptx_wasm::stream_api::stream_new();
    let err = doc_converter_pptx_wasm::stream_api::stream_open_pptx(id)
        .expect_err("пустой поток — ошибка");
    assert!(err
        .as_string()
        .is_some_and(|msg| msg.contains("stream aborted")));
}

#[wasm_bindgen_test]
fn unknown_stream_is_an_error() {
    let err = doc_converter_pptx_wasm::stream_api::stream_open_pptx(0)
        .expect_err("потока с таким id нет");
    assert!(err
        .as_string()
        .is_some_and(|msg| msg.contains("unknown stream")));
}
