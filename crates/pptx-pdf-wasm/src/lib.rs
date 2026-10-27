//! Отдельный wasm-модуль экспорта PPTX в PDF.
//!
//! Тянет `doc-converter-pptx-pdf` (экспортёр PDF) и разбор презентации,
//! поэтому в основной модуль не входит: просмотрщику он нужен только по клику
//! «Экспорт в PDF», и воркер подгружает его по требованию. Модуль
//! самодостаточен — никакого `thread_local`: презентацию он получает байтами и
//! разбирает сам (ADR-0033: просмотрщик и экспортёр разнесены по модулям).
//!
//! Как и `crates/wasm`, крейт собирается только под `wasm32`: на нативном
//! таргете он пуст, и `cargo clippy --workspace --all-targets` проходит без
//! выбора цели.

#![cfg(target_arch = "wasm32")]

use doc_converter_core::wire::PdfOptionsWire;
use doc_converter_pdf::PdfOptions;
use wasm_bindgen::prelude::*;

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Экспортировать презентацию PPTX в PDF: слайд = страница, размер страницы
/// из `p:sldSz`. Презентация приходит байтами OOXML и разбирается здесь же: у
/// модуля нет общего с просмотрщиком состояния.
///
/// `options` принимаются для симметрии протокола с XLSX/DOCX, но не читаются:
/// размер, поля, ориентация и масштаб страницы задаёт сама презентация
/// (`p:sldSz`), а метаданных в модели PPTX нет. Действует `compress` умолчаний.
///
/// # Errors
/// Если байты не OOXML-пакет, `ppt/presentation.xml` не разбирается, в
/// презентации нет слайдов или PDF не собрался.
#[wasm_bindgen]
pub fn export_pptx_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    // Опции протокола у PPTX не читаются (см. доккомментарий): валидируем
    // форму, чтобы битые настройки не проходили молча.
    if !options.is_undefined() && !options.is_null() {
        let _ = serde_wasm_bindgen::from_value::<PdfOptionsWire>(options).map_err(to_js)?;
    }
    let presentation = doc_converter_pptx::open(bytes.to_vec()).map_err(to_js)?;
    doc_converter_pptx_pdf::export(&presentation, &PdfOptions::default()).map_err(to_js)
}

/// Экспортировать презентацию ODP в PDF: слайд = страница, размер страницы из
/// раскладки. Презентация приходит байтами ODF и разбирается здесь же: модель —
/// та же, что у PPTX ([`doc_converter_odf::odp::open`] доводит `content.xml` до
/// презентации PPTX), поэтому раскладку и PDF даёт PPTX-семейство (ADR-0046).
///
/// `options` принимаются для симметрии протокола с PPTX, но не читаются: как у
/// [`export_pptx_pdf`], размер, поля, ориентация и масштаб страницы задаёт сама
/// презентация, а метаданных в модели PPTX нет.
///
/// # Errors
/// Если байты не ODP-пакет (`mimetype` не презентация, нет `content.xml`), в
/// презентации нет слайдов или PDF не собрался.
#[wasm_bindgen]
pub fn export_odp_pdf(bytes: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    // Опции протокола у ODP не читаются (см. доккомментарий): валидируем
    // форму, чтобы битые настройки не проходили молча.
    if !options.is_undefined() && !options.is_null() {
        let _ = serde_wasm_bindgen::from_value::<PdfOptionsWire>(options).map_err(to_js)?;
    }
    let presentation = doc_converter_odf::odp::open(bytes.to_vec()).map_err(to_js)?;
    doc_converter_pptx_pdf::export(&presentation, &PdfOptions::default()).map_err(to_js)
}
