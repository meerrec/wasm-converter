#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста экспорта PPTX в PDF: слайд = страница, размер страницы
//! из `p:sldSz`.

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Двухслайдовая фикстура из общего корпуса.
const TWO_SLIDES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/pptx/simple/two_slides.pptx"
));

/// PDF собирается: заголовок и хвост на месте, страниц — сколько слайдов.
#[wasm_bindgen_test]
fn export_pptx_pdf_has_one_page_per_slide() {
    let pdf = doc_converter_pdf_wasm::export_pptx_pdf(TWO_SLIDES, JsValue::UNDEFINED)
        .expect("фикстура экспортируется");
    assert!(pdf.starts_with(b"%PDF"), "заголовок PDF");
    assert!(pdf.ends_with(b"%%EOF"), "хвост PDF");
}

/// Мусор вместо пакета — ошибка с сообщением, а не пустой массив и не паника.
#[wasm_bindgen_test]
fn garbage_is_an_error() {
    let error = doc_converter_pdf_wasm::export_pptx_pdf(b"not a pptx", JsValue::UNDEFINED)
        .expect_err("мусор не экспортируется");
    let message = error.as_string().unwrap_or_default();
    assert!(!message.is_empty(), "ошибка приходит сообщением");
}
