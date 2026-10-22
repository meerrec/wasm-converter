#![cfg(target_arch = "wasm32")]

//! Проверки fitToPage в wasm-мосте XLSX → PDF: поля разбираются и экспорт жив.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const TABLE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/xlsx/content-table.xlsx"
));

fn options(fields: &[(&str, JsValue)]) -> JsValue {
    let object = js_sys::Object::new();
    for (name, value) in fields {
        js_sys::Reflect::set(&object, &JsValue::from_str(name), value)
            .expect("поле опций записывается");
    }
    object.into()
}

fn assert_pdf(bytes: Vec<u8>) {
    assert!(bytes.starts_with(b"%PDF"), "файл начинается с %PDF");
    assert!(bytes.len() > 100, "PDF непустой: {} байт", bytes.len());
}

/// `fitToPage: true` — лист умещается на одну страницу (1×1).
#[wasm_bindgen_test]
fn fit_to_page_is_accepted() {
    let pdf =
        doc_converter_pdf_wasm::export_pdf(TABLE, 0, options(&[("fitToPage", JsValue::TRUE)]))
            .expect("лист экспортируется с fitToPage");
    assert_pdf(pdf);
}

/// Явные числа сильнее флага; `0` — без ограничения.
#[wasm_bindgen_test]
fn explicit_fit_numbers_are_accepted() {
    let pdf = doc_converter_pdf_wasm::export_pdf(
        TABLE,
        0,
        options(&[
            ("fitToPage", JsValue::TRUE),
            ("fitToWidth", JsValue::from_f64(2.0)),
            ("fitToHeight", JsValue::from_f64(0.0)),
        ]),
    )
    .expect("лист экспортируется с fitToWidth/Height");
    assert_pdf(pdf);
}

/// Мусор в числах — ошибка разбора, а не паника.
#[wasm_bindgen_test]
fn negative_fit_width_is_an_error() {
    let error = doc_converter_pdf_wasm::export_pdf(
        TABLE,
        0,
        options(&[("fitToWidth", JsValue::from_f64(-1.0))]),
    )
    .expect_err("отрицательная ширина не разбирается");
    assert!(!error.as_string().unwrap_or_default().is_empty());
}
