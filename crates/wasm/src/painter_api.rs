//! WASM-экспорты жизненного цикла painter'а.
//!
//! Логика живёт в `render::canvas` (ADR-0003); здесь только wasm-bindgen-обвязка
//! и сериализация статистики.

use doc_converter_core::wire::PaintStatsWire;
use doc_converter_render::canvas::OffscreenPainter;
use doc_converter_render::painter::PaintStats;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2d;

/// Инициализирует painter ровно один раз. Повторный вызов — ошибка.
#[wasm_bindgen]
pub fn init_painter(ctx: OffscreenCanvasRenderingContext2d) -> Result<(), JsValue> {
    OffscreenPainter::install(ctx)
}

/// Инициализирует painter поверх main-thread canvas.
///
/// Типизированный [`init_painter`] остаётся для worker-пути: там контекст —
/// `OffscreenCanvasRenderingContext2d`. `init_painter_main` нужен main-режиму,
/// где документ рисуется в обычный `<canvas>`: `ctx.into()` конвертирует
/// typed handle в `JsValue`, а форму контекста проверяет
/// `OffscreenPainter::install_js` (duck-типизация общих Canvas2D-методов).
#[wasm_bindgen]
pub fn init_painter_main(ctx: web_sys::CanvasRenderingContext2d) -> Result<(), JsValue> {
    OffscreenPainter::install_js(ctx.into())
}

#[wasm_bindgen]
pub fn dispose_painter() {
    OffscreenPainter::dispose();
}

/// Прямой путь: байты DL копируются в Rust Vec (fallback).
#[wasm_bindgen]
pub fn paint_display_list_bytes(bytes: &[u8]) -> Result<JsValue, JsValue> {
    OffscreenPainter::with(|painter| to_stats(painter.paint_bytes(bytes)?))?
}

/// SAB-путь: читает самый свежий слот ring'а и рисует. Освобождает слот.
///
/// Промежуточные готовые кадры отпускаются — сборщик успевает положить второй
/// кадр, пока painter рисует первый, и рисовать их по очереди значило бы
/// показывать вчерашнюю прокрутку. Число пропущенных кадров видно в статистике
/// (`dropped`): это и есть backpressure глубины два.
#[wasm_bindgen]
pub fn paint_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
) -> Result<JsValue, JsValue> {
    // Ring здесь строится второй раз (второй — внутри `paint_sab`): это две
    // обёртки над тем же SAB, копирования нет, а границу painter'а ради одного
    // `Atomics.store` размыкать не хочется.
    let mut ring =
        doc_converter_render::sab::SabRing::from_sab(sab.clone(), slot_capacity as usize)?;
    let skipped = ring.advance_to_latest()?;
    let mut stats = OffscreenPainter::with(|painter| painter.paint_sab(sab, slot_capacity))??;
    if skipped > 0 {
        stats.dropped = true;
    }
    to_stats(stats)
}

/// Пересчитывает canvas под DPR и сбрасывает состояние painter'а.
#[wasm_bindgen]
pub fn resize_canvas(css_w: f32, css_h: f32, dpr: f32) -> Result<(), JsValue> {
    OffscreenPainter::with(|painter| painter.resize(css_w, css_h, dpr))?
}

fn to_stats(stats: PaintStats) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&PaintStatsWire {
        cmds: stats.cmds,
        dropped: stats.dropped,
        paint_ms: stats.paint_ms,
    })
    .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
}
