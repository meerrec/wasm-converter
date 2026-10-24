//! Потоковый вход: чанки копятся в [`SegmentedBuffer`] до конца потока,
//! парс — в форматном wasm-модуле (ADR-0031 §6).
//!
//! Реестр потоков (`STREAMS`/`NEXT_ID`) каждый форматный крейт объявляет сам —
//! `thread_local!` статики нельзя импортировать, — а вся логика здесь: парс из
//! чанков (`xlsx::open_segmented` и аналоги) копии не требует, `Archive` читает
//! сегменты как есть. Копия чанка на границе JS→wasm неизбежна без SAB и
//! принимается (ADR-0031 §6).

use std::cell::RefCell;
use std::collections::HashMap;
use std::thread::LocalKey;

use doc_converter_core::SegmentedBuffer;
use wasm_bindgen::JsValue;

/// Открытые потоки по id: чанки копятся, пока не придёт конец потока.
pub type StreamMap = RefCell<HashMap<u32, SegmentedBuffer>>;

/// Завести поток: id, под которым воркер будет слать чанки.
pub fn stream_new(
    streams: &'static LocalKey<StreamMap>,
    next_id: &'static LocalKey<RefCell<u32>>,
) -> u32 {
    next_id.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next = next.wrapping_add(1);
        streams.with(|streams| {
            streams.borrow_mut().insert(id, SegmentedBuffer::new());
        });
        id
    })
}

/// Дописать чанк. Возвращает накопленный размер — прогресс для диагностики.
///
/// # Errors
/// Если поток с таким id не заведён.
pub fn stream_append(
    streams: &'static LocalKey<StreamMap>,
    id: u32,
    bytes: &[u8],
) -> Result<u64, JsValue> {
    streams.with(|streams| {
        let mut streams = streams.borrow_mut();
        let buf = streams
            .get_mut(&id)
            .ok_or_else(|| JsValue::from_str("stream_append: unknown stream id"))?;
        Ok(buf.append(bytes))
    })
}

/// Отменить поток: буфер освобождается, id больше не действителен.
pub fn stream_free(streams: &'static LocalKey<StreamMap>, id: u32) {
    streams.with(|streams| {
        streams.borrow_mut().remove(&id);
    });
}

/// Изымает накопленный буфер из реестра — поток одноразовый, — отдавая его
/// форматному `stream_open_*` (ADR-0031 §6).
///
/// # Errors
/// Если поток с таким id не заведён.
pub fn stream_take(
    streams: &'static LocalKey<StreamMap>,
    id: u32,
) -> Result<SegmentedBuffer, JsValue> {
    streams
        .with(|streams| streams.borrow_mut().remove(&id))
        .ok_or_else(|| JsValue::from_str("stream_open: unknown stream id"))
}
