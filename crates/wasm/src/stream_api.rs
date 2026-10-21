//! Потоковый вход XLSX: чанки копятся в [`SegmentedBuffer`] до конца потока,
//! парс — в том же wasm-модуле (ADR-0031 §6).
//!
//! Реестр потоков живёт в основном модуле, потому что книга XLSX разбирается
//! здесь: байты между wasm-инстансами не передаются без копии через JS, а
//! парс из чанков (`xlsx::open_segmented`) копии и не требует — `Archive`
//! читает сегменты как есть. Копия чанка на границе JS→wasm неизбежна без
//! SAB и принимается (ADR-0031 §6).

use std::cell::RefCell;
use std::collections::HashMap;

use doc_converter_core::SegmentedBuffer;
use wasm_bindgen::prelude::*;

thread_local! {
    /// Открытые потоки по id: чанки копятся, пока не придёт конец потока.
    static STREAMS: RefCell<HashMap<u32, SegmentedBuffer>> = RefCell::new(HashMap::new());
    /// Следующий id потока — монотонный счётчик воркера, повторов не бывает.
    static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
}

/// Завести поток: id, под которым воркер будет слать чанки.
#[wasm_bindgen]
pub fn stream_new() -> u32 {
    NEXT_ID.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next = next.wrapping_add(1);
        STREAMS.with(|streams| {
            streams.borrow_mut().insert(id, SegmentedBuffer::new());
        });
        id
    })
}

/// Дописать чанк. Возвращает накопленный размер — прогресс для диагностики.
///
/// # Errors
/// Если поток с таким id не заведён.
#[wasm_bindgen]
pub fn stream_append(id: u32, bytes: &[u8]) -> Result<u64, JsValue> {
    STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let buf = streams
            .get_mut(&id)
            .ok_or_else(|| JsValue::from_str("stream_append: unknown stream id"))?;
        Ok(buf.append(bytes))
    })
}

/// Отменить поток: буфер освобождается, id больше не действителен.
#[wasm_bindgen]
pub fn stream_free(id: u32) {
    STREAMS.with(|streams| {
        streams.borrow_mut().remove(&id);
    });
}

/// Завершить поток: разобрать накопленный буфер как книгу (ADR-0031 §6).
///
/// Буфер изымается из реестра — поток одноразовый, — книга встаёт в `DOC`,
/// как при `xlsx_open`. Ответ — тот же список листов.
///
/// # Errors
/// Если поток неизвестен, буфер пуст (оборванный поток) или байты не
/// OOXML-пакет.
#[wasm_bindgen]
pub fn stream_open_xlsx(id: u32) -> Result<JsValue, JsValue> {
    let buf = STREAMS
        .with(|streams| streams.borrow_mut().remove(&id))
        .ok_or_else(|| JsValue::from_str("stream_open: unknown stream id"))?;
    if buf.is_empty() {
        return Err(JsValue::from_str("stream aborted"));
    }
    crate::xlsx_api::set_cancel(false);
    let book = doc_converter_xlsx::open_segmented(buf).map_err(crate::xlsx_api::to_js)?;
    crate::xlsx_api::install_book(book)
}
