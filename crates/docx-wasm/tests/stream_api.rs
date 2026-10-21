#![cfg(target_arch = "wasm32")]

//! Проверки потокового входа DOCX (ADR-0031): чанки копятся в ленивом модуле,
//! парс — по `stream_open_docx` без сборки контигуального буфера.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Одностраничная фикстура из общего корпуса.
const ONE_PARAGRAPH: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/docx/simple/one_paragraph.docx"
));

#[wasm_bindgen_test]
fn stream_open_parses_chunked_document() {
    let id = doc_converter_docx_wasm::stream_api::stream_new();
    let mut total = 0u64;
    for chunk in ONE_PARAGRAPH.chunks(7) {
        total =
            doc_converter_docx_wasm::stream_api::stream_append(id, chunk).expect("чанк дописан");
    }
    assert_eq!(total, ONE_PARAGRAPH.len() as u64);

    let pages = doc_converter_docx_wasm::stream_api::stream_open_docx(id)
        .expect("документ разобран из чанков");
    assert!(pages.is_array(), "ответ — массив страниц");
    let length = js_sys::Reflect::get(&pages, &JsValue::from_str("length"))
        .expect("length читается")
        .as_f64()
        .expect("length — число");
    assert!(length >= 1.0, "у документа есть хотя бы одна страница");
}

#[wasm_bindgen_test]
fn empty_stream_is_an_open_error() {
    let id = doc_converter_docx_wasm::stream_api::stream_new();
    let err = doc_converter_docx_wasm::stream_api::stream_open_docx(id)
        .expect_err("пустой поток — ошибка");
    assert!(err
        .as_string()
        .is_some_and(|msg| msg.contains("stream aborted")));
}
