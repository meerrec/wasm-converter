#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста экспорта DOCX → PDF: фикстура превращается в PDF.
//!
//! Фикстура вкомпилирована (`include_bytes!`): у теста под wasm нет файловой
//! системы, а путь от `CARGO_MANIFEST_DIR` тот же, что у нативных тестов.

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Одностраничная фикстура: один абзац «Hello, World!».
const ONE_PARAGRAPH: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/docx/simple/one_paragraph.docx"
));

/// Опции из интерфейса в том же виде, в каком их шлёт воркер.
fn options(fields: &[(&str, &str)]) -> JsValue {
    let object = js_sys::Object::new();
    for (name, value) in fields {
        js_sys::Reflect::set(&object, &JsValue::from_str(name), &JsValue::from_str(value))
            .expect("поле опций записывается");
    }
    object.into()
}

/// Без опций документ собирается с умолчаниями, а не отказывает.
#[wasm_bindgen_test]
fn exports_a_docx_without_options() {
    let pdf = doc_converter_pdf_wasm::export_docx_pdf(ONE_PARAGRAPH, JsValue::UNDEFINED)
        .expect("фикстура экспортируется");
    assert!(pdf.starts_with(b"%PDF"), "файл начинается с %PDF");
    assert!(pdf.len() > 100, "PDF непустой: {} байт", pdf.len());
}

/// Опции разбираются тем же путём, что у `export_pdf`: объект с известными
/// полями не ломает экспорт. Размер и ориентация страниц у документа свои —
/// они берутся из `w:sectPr`, — но разбор полей обязан пройти.
#[wasm_bindgen_test]
fn options_come_from_the_interface() {
    let pdf = doc_converter_pdf_wasm::export_docx_pdf(
        ONE_PARAGRAPH,
        options(&[("pageSize", "A3"), ("orientation", "landscape")]),
    )
    .expect("фикстура экспортируется с опциями");
    assert!(pdf.starts_with(b"%PDF"), "файл начинается с %PDF");
}

/// Мусор вместо пакета — ошибка с сообщением для интерфейса, а не паника.
#[wasm_bindgen_test]
fn garbage_is_an_error() {
    let error = doc_converter_pdf_wasm::export_docx_pdf(b"not a docx", JsValue::UNDEFINED)
        .expect_err("мусор не экспортируется");
    let message = error.as_string().unwrap_or_default();
    assert!(!message.is_empty(), "ошибка приходит сообщением");
}
