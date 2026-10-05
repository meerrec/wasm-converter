//! WASM-экспорты жизненного цикла painter'а.
//!
//! Логика живёт в `render::canvas` (ADR-0003); здесь только wasm-bindgen-обвязка
//! и сериализация статистики.

use doc_converter_render::canvas::OffscreenPainter;
use doc_converter_render::painter::PaintStats;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2d;

/// Инициализирует painter ровно один раз. Повторный вызов — ошибка.
#[wasm_bindgen]
pub fn init_painter(ctx: OffscreenCanvasRenderingContext2d) -> Result<(), JsValue> {
    OffscreenPainter::install(ctx)
}

#[wasm_bindgen]
pub fn dispose_painter() {
    OffscreenPainter::dispose();
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStatsJs {
    cmds: u32,
    dropped: bool,
    paint_ms: f64,
}

impl From<PaintStats> for PaintStatsJs {
    fn from(s: PaintStats) -> Self {
        Self {
            cmds: s.cmds,
            dropped: s.dropped,
            paint_ms: s.paint_ms,
        }
    }
}

/// Прямой путь: байты DL копируются в Rust Vec (fallback).
#[wasm_bindgen]
pub fn paint_display_list_bytes(bytes: &[u8]) -> Result<JsValue, JsValue> {
    OffscreenPainter::with(|painter| to_stats(painter.paint_bytes(bytes)?))?
}

/// SAB-путь: читает текущий слот ring'а и рисует. Освобождает слот.
#[wasm_bindgen]
pub fn paint_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
) -> Result<JsValue, JsValue> {
    OffscreenPainter::with(|painter| to_stats(painter.paint_sab(sab, slot_capacity)?))?
}

/// Пересчитывает canvas под DPR и сбрасывает состояние painter'а.
#[wasm_bindgen]
pub fn resize_canvas(css_w: f32, css_h: f32, dpr: f32) -> Result<(), JsValue> {
    OffscreenPainter::with(|painter| painter.resize(css_w, css_h, dpr))?
}

fn to_stats(stats: PaintStats) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
        .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
}
