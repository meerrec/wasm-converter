#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста TextLayer: runs страницы, полный слой, ошибки диапазона.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const ONE_PARAGRAPH: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/docx/simple/one_paragraph.docx"
));

fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &JsValue::from_str(name)).expect("поле есть")
}

#[wasm_bindgen_test]
fn text_layer_has_runs_and_full_layer_has_page_map() {
    doc_converter_docx_wasm::docx_api::docx_open(ONE_PARAGRAPH.to_vec())
        .expect("фикстура открывается");

    let layer =
        doc_converter_docx_wasm::text_api::docx_get_text_layer(0).expect("страница 0 существует");
    let runs = field(&layer, "runs");
    let runs: js_sys::Array = runs.dyn_into().expect("runs — массив");
    assert!(runs.length() > 0, "на странице есть текст");
    let first = runs.get(0);
    let text = field(&first, "text")
        .as_string()
        .expect("текст run'а — строка");
    assert!(!text.is_empty());

    let full = doc_converter_docx_wasm::text_api::docx_get_full_text_layer()
        .expect("полный слой собирается");
    let page_map: js_sys::Array = field(&full, "pageMap")
        .dyn_into()
        .expect("pageMap — массив");
    assert_eq!(page_map.length(), 1);

    let err = doc_converter_docx_wasm::text_api::docx_get_text_layer(9)
        .expect_err("страница вне диапазона");
    assert!(
        err.as_string()
            .is_some_and(|message| message.contains("out of range")),
        "ошибка: {err:?}"
    );

    doc_converter_docx_wasm::docx_api::docx_close();
}

#[wasm_bindgen_test]
fn text_layer_requires_an_open_document() {
    doc_converter_docx_wasm::docx_api::docx_close();
    let err = doc_converter_docx_wasm::text_api::docx_get_text_layer(0)
        .expect_err("без документа слоя нет");
    assert!(
        err.as_string()
            .is_some_and(|message| message.contains("no docx document is open")),
        "ошибка: {err:?}"
    );
}
