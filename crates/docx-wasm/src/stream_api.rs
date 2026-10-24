//! Потоковый вход DOCX: чанки копятся в [`doc_converter_core::SegmentedBuffer`] до конца потока,
//! парс — в том же wasm-модуле (ADR-0031 §6).
//!
//! Реестр потоков живёт в ленивом модуле DOCX, потому что документ разбирается
//! здесь: байты между wasm-инстансами не передаются без копии через JS, а
//! парс из чанков (`parse_docx_segmented`) копии и не требует — `Archive`
//! читает сегменты как есть. Копия чанка на границе JS→wasm неизбежна без
//! SAB и принимается (ADR-0031 §6).
//!
//! Логика реестра общая с XLSX/PPTX и живёт в `doc_converter_wasm_common::streams`;
//! здесь — только `thread_local!`-пара (статики импортировать нельзя) и
//! `#[wasm_bindgen]`-обёртки.

use std::cell::RefCell;
use std::collections::HashMap;

use doc_converter_wasm_common::streams::StreamMap;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Открытые потоки по id: чанки копятся, пока не придёт конец потока.
    static STREAMS: StreamMap = RefCell::new(HashMap::new());
    /// Следующий id потока — монотонный счётчик воркера, повторов не бывает.
    static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
}

/// Завести поток: id, под которым воркер будет слать чанки.
#[wasm_bindgen]
pub fn stream_new() -> u32 {
    doc_converter_wasm_common::streams::stream_new(&STREAMS, &NEXT_ID)
}

/// Дописать чанк. Возвращает накопленный размер — прогресс для диагностики.
///
/// # Errors
/// Если поток с таким id не заведён.
#[wasm_bindgen]
pub fn stream_append(id: u32, bytes: &[u8]) -> Result<u64, JsValue> {
    doc_converter_wasm_common::streams::stream_append(&STREAMS, id, bytes)
}

/// Отменить поток: буфер освобождается, id больше не действителен.
#[wasm_bindgen]
pub fn stream_free(id: u32) {
    doc_converter_wasm_common::streams::stream_free(&STREAMS, id);
}

/// Завершить поток: разобрать накопленный буфер как документ (ADR-0031 §6).
///
/// Буфер изымается из реестра — поток одноразовый, — документ встаёт в `DOC`,
/// как при `docx_open`. Ответ — тот же список страниц.
///
/// # Errors
/// Если поток неизвестен, буфер пуст (оборванный поток) или байты не
/// OOXML-пакет.
#[wasm_bindgen]
pub fn stream_open_docx(id: u32) -> Result<JsValue, JsValue> {
    let buf = doc_converter_wasm_common::streams::stream_take(&STREAMS, id)?;
    if buf.is_empty() {
        return Err(JsValue::from_str("stream aborted"));
    }
    // Флаг отмены не сбрасываем: он мог быть взведён за время загрузки чанков
    // (ADR-0032), и раскладка в `install_document` — первая точка проверки.
    let document =
        doc_converter_docx::parse_docx_segmented(buf, doc_converter_core::ZipLimits::default())
            .map_err(crate::docx_api::to_js)?;
    crate::docx_api::install_document(document)
}
