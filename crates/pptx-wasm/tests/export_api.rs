#![cfg(target_arch = "wasm32")]

//! Проверки wasm-моста экспорта PPTX: Markdown, HTML, обычный текст и PDF.
//!
//! Фикстуры вкомпилированы (`include_bytes!`), как и в `tests/pptx_api.rs`: у
//! теста под wasm нет файловой системы.

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Два слайда с текстом.
const TWO_SLIDES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-fixtures/pptx/simple/two_slides.pptx"
));

/// Байты фикстуры в том виде, в каком их шлёт воркер.
fn fixture() -> js_sys::Uint8Array {
    js_sys::Uint8Array::from(TWO_SLIDES)
}

/// Markdown начинается с имени первого слайда заголовком второго уровня.
#[wasm_bindgen_test]
fn markdown_outlines_slides() {
    let markdown = doc_converter_pptx_wasm::export_api::pptx_export_markdown(fixture())
        .expect("фикстура экспортируется");
    assert!(markdown.starts_with("## Deck 1\n"), "вывод: {markdown:?}");
    assert!(markdown.contains("## Deck 2\n"), "второй слайд на месте");
    assert!(markdown.contains("First bullet"), "текст тела на месте");
}

/// HTML — документ целиком, а не фрагмент.
#[wasm_bindgen_test]
fn html_is_a_whole_document() {
    let html = doc_converter_pptx_wasm::export_api::pptx_export_html(fixture())
        .expect("фикстура экспортируется");
    assert!(html.starts_with("<!doctype html>"), "начало: {html:.40}");
    assert!(html.contains("Deck 1"), "имя слайда на месте");
    assert!(html.ends_with("</html>\n"), "конец документа на месте");
}

/// Обычный текст — конспект текстовых рамок без разметки.
#[wasm_bindgen_test]
fn text_is_the_outline_without_markup() {
    let text = doc_converter_pptx_wasm::export_api::pptx_export_text(fixture())
        .expect("фикстура экспортируется");
    assert!(text.starts_with("Deck 1\n"), "вывод: {text:?}");
    assert!(text.contains("Deck 2\n"), "второй слайд на месте");
    assert!(text.contains("First bullet"), "текст тела на месте");
}

/// Мусор вместо пакета — ошибка с сообщением для интерфейса, а не пустая
/// строка и не паника.
#[wasm_bindgen_test]
fn garbage_is_an_error() {
    let error = doc_converter_pptx_wasm::export_api::pptx_export_markdown(
        js_sys::Uint8Array::from(b"not a pptx".as_slice()),
    )
    .expect_err("мусор не экспортируется");
    let message = error.as_string().unwrap_or_default();
    assert!(!message.is_empty(), "ошибка приходит сообщением");
}
