#![cfg(target_arch = "wasm32")]

//! Проверки потокового входа XLSX (ADR-0031): чанки копятся в wasm-модуле,
//! парс — по `stream_open_xlsx` без сборки контигуального буфера.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Маленькая книга из общего корпуса фикстур (`scripts/gen-fixtures.ts`).
const BOOK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/xlsx/cf-cell-is.xlsx"
));

#[wasm_bindgen_test]
fn stream_open_parses_chunked_book() {
    let id = doc_converter_wasm::stream_api::stream_new();
    let mut total = 0u64;
    // Чанки по 7 байт — мельче любой части книги: чтение обязано переходить
    // границы сегментов в обе стороны.
    for chunk in BOOK.chunks(7) {
        total = doc_converter_wasm::stream_api::stream_append(id, chunk).expect("чанк дописан");
    }
    assert_eq!(total, BOOK.len() as u64);

    let sheets =
        doc_converter_wasm::stream_api::stream_open_xlsx(id).expect("книга разобрана из чанков");
    assert!(sheets.is_array(), "ответ — массив листов");
    let length = js_sys::Reflect::get(&sheets, &JsValue::from_str("length"))
        .expect("length читается")
        .as_f64()
        .expect("length — число");
    assert!(length >= 1.0, "у книги есть хотя бы один лист");
}

#[wasm_bindgen_test]
fn empty_stream_is_an_open_error() {
    let id = doc_converter_wasm::stream_api::stream_new();
    let err =
        doc_converter_wasm::stream_api::stream_open_xlsx(id).expect_err("пустой поток — ошибка");
    assert!(err
        .as_string()
        .is_some_and(|msg| msg.contains("stream aborted")));
}

#[wasm_bindgen_test]
fn unknown_stream_ids_are_errors_and_free_releases() {
    let id = doc_converter_wasm::stream_api::stream_new();
    assert!(
        doc_converter_wasm::stream_api::stream_append(id + 1, b"x").is_err(),
        "чужой id — ошибка"
    );
    assert!(
        doc_converter_wasm::stream_api::stream_open_xlsx(id + 1).is_err(),
        "чужой id — ошибка"
    );
    doc_converter_wasm::stream_api::stream_free(id);
    assert!(
        doc_converter_wasm::stream_api::stream_append(id, b"x").is_err(),
        "освобождённый id — ошибка"
    );
}
