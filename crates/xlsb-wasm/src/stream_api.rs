//! Потоковый вход XLSB: чанки копятся в [`doc_converter_core::SegmentedBuffer`] до конца потока,
//! парс — в том же wasm-модуле (ADR-0031 §6).
//!
//! У XLSB нет `open_segmented`, поэтому накопленный поток собирается в
//! `Vec<u8>` и открывается `doc_converter_xlsb::open` — одна копия целого
//! файла на границе «поток → парс». Это цена потокового входа без
//! специализированного парсера чанков; для остальных форматов `Archive` читает
//! сегменты как есть (ADR-0031 §6).
//!
//! Логика реестра общая с XLSX/DOCX/PPTX и живёт в `doc_converter_wasm_common::streams`;
//! здесь — только `thread_local!`-пара (статики импортировать нельзя) и
//! `#[wasm_bindgen]`-обёртки.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;

use doc_converter_core::SegmentedBuffer;
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

/// Завершить поток: собрать накопленный буфер в `Vec<u8>` и разобрать как
/// книгу (ADR-0031 §6).
///
/// Буфер изымается из реестра — поток одноразовый, — книга встаёт в `DOC`, как
/// при `xlsb_open`. Ответ — тот же список листов.
///
/// # Errors
/// Если поток неизвестен, буфер пуст (оборванный поток) или байты не
/// OOXML-пакет.
#[wasm_bindgen]
pub fn stream_open_xlsb(id: u32) -> Result<JsValue, JsValue> {
    let buf = doc_converter_wasm_common::streams::stream_take(&STREAMS, id)?;
    if buf.is_empty() {
        return Err(JsValue::from_str("stream aborted"));
    }
    // Флаг отмены не сбрасываем: он мог быть взведён за время загрузки чанков
    // (ADR-0032), и сборка кадра — первая точка проверки.
    let book = doc_converter_xlsb::open(to_vec(buf)?).map_err(crate::xlsb_api::to_js)?;
    crate::xlsb_api::install_book(book)
}

/// Собрать накопленный поток в `Vec<u8>`.
///
/// `SegmentedBuffer` не отдаёт сегменты наружу, а `read_to_end` не годится:
/// `Read` для него сигналит конец данных `UnexpectedEof`, а не `Ok(0)`, —
/// поэтому читаем вручную до этой ошибки.
fn to_vec(mut buf: SegmentedBuffer) -> Result<Vec<u8>, JsValue> {
    let capacity = usize::try_from(buf.total()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match buf.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(JsValue::from_str(&format!("stream read failed: {error}"))),
        }
    }
    Ok(bytes)
}
