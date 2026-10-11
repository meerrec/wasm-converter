//! WASM-обвязка экспорта DOCX: Markdown, HTML и обычный текст.
//!
//! В отличие от [`crate::docx_api`], модуль не держит состояния между
//! вызовами: экспорт — разовая операция над байтами пакета, и разобранный
//! документ после неё не нужен. Поэтому ни `thread_local`, ни общего с
//! просмотрщиком документа здесь нет — экспорт не затирает открытый документ
//! и не мешает просмотру.

use doc_converter_docx::Document;
use wasm_bindgen::prelude::*;

use crate::docx_api::to_js;

/// Экспортировать документ в Markdown.
///
/// Разметка — у
/// [`doc_converter_docx::export::markdown::to_markdown`]: заголовки, списки,
/// GFM-таблицы, начертания, ссылки.
///
/// # Errors
/// Если байты не OOXML-пакет или `word/document.xml` не разбирается.
#[wasm_bindgen]
pub fn docx_export_markdown(bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
    let document = open_document(bytes)?;
    Ok(doc_converter_docx::export::markdown::to_markdown(&document))
}

/// Экспортировать документ в HTML — самостоятельную страницу со встроенными
/// стилями, а не фрагмент.
///
/// Разметка — у [`doc_converter_docx::export::html::to_html`].
///
/// # Errors
/// Если байты не OOXML-пакет или `word/document.xml` не разбирается.
#[wasm_bindgen]
pub fn docx_export_html(bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
    let document = open_document(bytes)?;
    Ok(doc_converter_docx::export::html::to_html(&document))
}

/// Экспортировать документ в обычный текст: содержимое без разметки.
///
/// Разметка — у [`doc_converter_docx::export::text::to_text`].
///
/// # Errors
/// Если байты не OOXML-пакет или `word/document.xml` не разбирается.
#[wasm_bindgen]
pub fn docx_export_text(bytes: js_sys::Uint8Array) -> Result<String, JsValue> {
    let document = open_document(bytes)?;
    Ok(doc_converter_docx::export::text::to_text(&document))
}

/// Разобрать пакет, приехавший из JS: все три экспортёра начинаются с этого.
fn open_document(bytes: js_sys::Uint8Array) -> Result<Document, JsValue> {
    doc_converter_docx::open(bytes.to_vec()).map_err(to_js)
}
