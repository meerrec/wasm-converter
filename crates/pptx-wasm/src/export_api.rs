//! WASM-обвязка экспорта PPTX: Markdown, HTML, обычный текст и PDF.
//!
//! В отличие от [`crate::pptx_api`], модуль не держит состояния между
//! вызовами: экспорт — разовая операция над байтами пакета, и разобранная
//! презентация после неё не нужна. Поэтому ни `thread_local`, ни общего с
//! просмотрщиком документа здесь нет — экспорт не затирает открытую
//! презентацию и не мешает просмотру.

use wasm_bindgen::prelude::*;

use crate::pptx_api::to_js;

/// Экспортировать презентацию в Markdown: слайды заголовками второго уровня,
/// маркеры списков — `-`/`1.` с отступом уровня.
///
/// Разметка — у [`doc_converter_pptx::export::to_markdown`].
///
/// # Errors
/// Если байты не OOXML-пакет или `ppt/presentation.xml` не разбирается.
#[wasm_bindgen]
pub fn pptx_export_markdown(bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
    let presentation = open_presentation(bytes)?;
    Ok(doc_converter_pptx::export::to_markdown(&presentation))
}

/// Экспортировать презентацию в HTML — самостоятельную страницу со
/// встроенными стилями, а не фрагмент.
///
/// Разметка — у [`doc_converter_pptx::export::to_html`].
///
/// # Errors
/// Если байты не OOXML-пакет или `ppt/presentation.xml` не разбирается.
#[wasm_bindgen]
pub fn pptx_export_html(bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
    let presentation = open_presentation(bytes)?;
    Ok(doc_converter_pptx::export::to_html(&presentation))
}

/// Экспортировать презентацию в обычный текст: конспект из текстовых рамок.
///
/// Разметка — у [`doc_converter_pptx::export::to_text`].
///
/// # Errors
/// Если байты не OOXML-пакет или `ppt/presentation.xml` не разбирается.
#[wasm_bindgen]
pub fn pptx_export_text(bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
    let presentation = open_presentation(bytes)?;
    Ok(doc_converter_pptx::export::to_text(&presentation))
}

/// Разобрать пакет, приехавший из JS: все три экспортёра начинаются с этого.
fn open_presentation(
    bytes: js_sys::Uint8Array,
) -> Result<doc_converter_pptx::Presentation, JsValue> {
    doc_converter_pptx::open(bytes.to_vec()).map_err(to_js)
}
