use doc_converter_render::painter::{PaintStats, Painter2D};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2d;

thread_local! {
    static PAINTER: RefCell<Option<Painter2D>> = const { RefCell::new(None) };
}

pub(crate) fn with_painter<R>(f: impl FnOnce(&mut Painter2D) -> R) -> Result<R, JsValue> {
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        let painter = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("painter not initialised"))?;
        Ok(f(painter))
    })
}

/// Инициализирует painter ровно один раз. Повторный вызов — ошибка.
#[wasm_bindgen]
pub fn init_painter(ctx: OffscreenCanvasRenderingContext2d) -> Result<(), JsValue> {
    PAINTER.with(|p| {
        let mut slot = p.borrow_mut();
        if slot.is_some() {
            return Err(JsValue::from_str("painter already initialised"));
        }
        *slot = Some(Painter2D::new(ctx));
        Ok(())
    })
}

#[wasm_bindgen]
pub fn dispose_painter() {
    PAINTER.with(|p| {
        *p.borrow_mut() = None;
    });
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
    with_painter(|painter| {
        let stats = painter.paint_bytes(bytes)?;
        serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })?
}

/// SAB-путь: читает текущий слот ring'а и рисует. Освобождает слот.
#[wasm_bindgen]
pub fn paint_display_list_sab(
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: u32,
) -> Result<JsValue, JsValue> {
    let mut ring = doc_converter_render::sab::SabRing::from_sab(sab, slot_capacity as usize)?;
    let mut buf = Vec::new();
    if !ring.copy_current_into(&mut buf)? {
        let empty = PaintStatsJs {
            cmds: 0,
            dropped: true,
            paint_ms: 0.0,
        };
        return Ok(serde_wasm_bindgen::to_value(&empty).unwrap());
    }
    // Слот освобождается сразу после копирования: без этого счётчик читателя
    // стоит на месте, оба слота считаются занятыми, и после второго кадра
    // запись навсегда упирается в «нет свободного слота».
    ring.release_current()?;
    with_painter(|painter| {
        let stats = painter.paint_bytes(&buf)?;
        serde_wasm_bindgen::to_value(&PaintStatsJs::from(stats))
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })?
}

/// Пересчитывает canvas под DPR и сбрасывает состояние painter'а.
#[wasm_bindgen]
pub fn resize_canvas(
    ctx: OffscreenCanvasRenderingContext2d,
    css_w: f32,
    css_h: f32,
    dpr: f32,
) -> Result<(), JsValue> {
    let canvas = ctx.canvas();
    let w = (css_w * dpr).round() as u32;
    let h = (css_h * dpr).round() as u32;
    if w == 0 || h == 0 {
        return Err(JsValue::from_str("canvas size must be > 0"));
    }
    canvas.set_width(w);
    canvas.set_height(h);

    PAINTER.with(|p| {
        if let Some(painter) = p.borrow_mut().as_mut() {
            painter.reset_state();
        }
    });
    Ok(())
}
