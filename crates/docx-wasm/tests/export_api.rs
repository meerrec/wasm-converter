#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста экспорта DOCX: Markdown, HTML и обычный текст.
//!
//! Фикстура вкомпилирована (`include_bytes!`), как и в `tests/docx_api.rs`: у
//! теста под wasm нет файловой системы, а путь от `CARGO_MANIFEST_DIR` тот же,
//! что у нативных тестов.

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Одностраничная фикстура: один абзац «Hello, World!».
const ONE_PARAGRAPH: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/docx/simple/one_paragraph.docx"
));

/// Байты фикстуры в том виде, в каком их шлёт воркер.
fn fixture() -> js_sys::Uint8Array {
    js_sys::Uint8Array::from(ONE_PARAGRAPH)
}

/// Вывод начинается с текста абзаца: у Markdown он идёт как есть — абзац без
/// начертаний разметки не получает.
#[wasm_bindgen_test]
fn markdown_starts_with_the_paragraph_text() {
    let markdown = doc_converter_docx_wasm::export_api::docx_export_markdown(fixture())
        .expect("фикстура экспортируется");
    assert!(markdown.starts_with("Hello, World!"), "вывод: {markdown:?}");
}

/// HTML — документ целиком, а не фрагмент.
#[wasm_bindgen_test]
fn html_is_a_whole_document() {
    let html = doc_converter_docx_wasm::export_api::docx_export_html(fixture())
        .expect("фикстура экспортируется");
    assert!(html.starts_with("<!DOCTYPE html>"), "начало: {html:.40}");
    assert!(html.contains("Hello, World!"), "текст абзаца на месте");
    assert!(html.ends_with("</html>\n"), "конец документа на месте");
}

/// Обычный текст — содержимое абзаца одной строкой, без разметки.
#[wasm_bindgen_test]
fn text_is_the_content_without_markup() {
    let text = doc_converter_docx_wasm::export_api::docx_export_text(fixture())
        .expect("фикстура экспортируется");
    assert_eq!(text, "Hello, World!\n");
}

/// Мусор вместо пакета — ошибка с сообщением для интерфейса, а не пустая
/// строка и не паника.
#[wasm_bindgen_test]
fn garbage_is_an_error() {
    let error = doc_converter_docx_wasm::export_api::docx_export_markdown(
        js_sys::Uint8Array::from(b"not a docx".as_slice()),
    )
    .expect_err("мусор не экспортируется");
    let message = error.as_string().unwrap_or_default();
    assert!(!message.is_empty(), "ошибка приходит сообщением");
}
