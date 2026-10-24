//! Сборка кадра DisplayList и его доставка транспорту — общая для XLSX/DOCX/PPTX.
//!
//! Канон — бывший `with_frame_bytes` из `crates/wasm/src/xlsx_api.rs`: сборка
//! кадра, сериализация в переиспользуемый `SCRATCH` и запись байтов через
//! `write` — общие у всех путей (SAB и без него), потому что решение о
//! транспорте не должно разъезжаться с решением о содержимом кадра. Слот
//! ring'а и буфер JS отличаются только тем, куда байты деваются, — это и есть
//! `write`.
//!
//! Специфика формата (взятие книги/документа из `DOC`, проверка отмены,
//! маппинг ошибок формата в `JsValue`) остаётся в `build`-замыкании вызывающего:
//! типы ошибок и thread_local-статика у форматов разные, а сериализация — нет.
//! `FRAME`/`SCRATCH` каждый форматный крейт объявляет сам по той же причине.

use std::cell::RefCell;
use std::thread::LocalKey;

use doc_converter_core::wire::{BuildStatsWire, ViewportWire};
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::sab::SabRing;
use doc_converter_render::viewport::Viewport;
use wasm_bindgen::JsValue;

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Таймер статистики сборки кадра.
pub fn now() -> f64 {
    js_sys::Date::now()
}

/// Перевести окно из JS в представление рендера. Форма окна общая у всех
/// форматов (`ViewportWire`).
///
/// # Errors
/// Если окно описано не так, как ждёт `ViewportWire`.
pub fn viewport_of(value: JsValue) -> Result<Viewport, JsValue> {
    let js: ViewportWire = serde_wasm_bindgen::from_value(value).map_err(to_js)?;
    Ok(Viewport::new(
        js.scroll_x,
        js.scroll_y,
        js.width,
        js.height,
        js.scale,
    ))
}

/// Собрать кадр и отдать его байты транспорту вызывающего.
///
/// `build` заполняет `FRAME` и возвращает число команд (`cmds`) или ошибку;
/// `write` решает, куда байты деваются (слот ring'а, буфер вызывающего,
/// обычный `Vec`) и отвечает, влезли ли они (`written`).
///
/// # Errors
/// Если `build` не собрал кадр или запись не удалась.
pub fn with_frame_bytes(
    frame: &'static LocalKey<RefCell<DisplayList>>,
    scratch: &'static LocalKey<RefCell<Vec<u8>>>,
    build: impl FnOnce(&mut DisplayList) -> Result<u32, JsValue>,
    write: impl FnOnce(&[u8]) -> Result<bool, JsValue>,
) -> Result<BuildStatsWire, JsValue> {
    let started = now();
    frame.with(|frame| {
        let mut frame = frame.borrow_mut();
        let cmds = build(&mut frame)?;

        scratch.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            frame.to_bytes_into(&mut scratch);
            let bytes = u32::try_from(scratch.len()).unwrap_or(u32::MAX);
            Ok(BuildStatsWire {
                written: write(&scratch)?,
                cmds,
                bytes,
                build_ms: now() - started,
            })
        })
    })
}

/// Собрать кадр сразу в слот ring'а (`SharedArrayBuffer`).
///
/// Возвращает `written: false`, если свободного слота не было: кадр теряется,
/// но следующий за ним рисуется — так и задумано, догонять прокрутку
/// устаревшими кадрами незачем. Второй случай `written: false` — кадр больше
/// слота: `bytes` в статистике скажет, насколько, а по нему воркер решит,
/// растить ли ring.
///
/// # Errors
/// Если кадр не собрался, запись не удалась или SAB не тот.
pub fn build_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
    frame: &'static LocalKey<RefCell<DisplayList>>,
    scratch: &'static LocalKey<RefCell<Vec<u8>>>,
    build: impl FnOnce(&mut DisplayList) -> Result<u32, JsValue>,
) -> Result<BuildStatsWire, JsValue> {
    with_frame_bytes(frame, scratch, build, |bytes| {
        let mut ring = SabRing::from_sab(sab, slot_capacity as usize)?;
        ring.try_write(bytes)
    })
}

/// Собрать кадр в буфер вызывающего — путь без `SharedArrayBuffer`.
///
/// Буфер переиспользуется: воркер держит два и заполняет то один, то другой,
/// поэтому на кадр нет ни аллокации, ни копии из памяти wasm в JS — только
/// запись в готовый массив. `written: false` вместе с `bytes` больше длины
/// буфера означает, что буфер мал: воркер вырастит его и повторит сборку.
///
/// # Errors
/// Если кадр не собрался.
pub fn build_into(
    out: &js_sys::Uint8Array,
    frame: &'static LocalKey<RefCell<DisplayList>>,
    scratch: &'static LocalKey<RefCell<Vec<u8>>>,
    build: impl FnOnce(&mut DisplayList) -> Result<u32, JsValue>,
) -> Result<BuildStatsWire, JsValue> {
    with_frame_bytes(frame, scratch, build, |bytes| {
        let length = bytes.len() as u32;
        if length > out.length() {
            return Ok(false);
        }
        out.subarray(0, length).copy_from(bytes);
        Ok(true)
    })
}
