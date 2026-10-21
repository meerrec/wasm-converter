//! Жизненный цикл OffscreenCanvas-контекста: создание, resize, сброс состояния
//! и оба пути рисования (SAB и копия).
//!
//! Только wasm32: на нативном таргете canvas нет (ADR-0003). Коалесирование
//! кадров через `requestAnimationFrame` остаётся в воркере
//! (`packages/core/src/worker/frame_loop.ts`): цикл событий JS не принадлежит
//! Rust, и переносить его сюда значило бы дублировать протокол воркера.
//! Спринт 5.5 закрывает вынос жизненного цикла контекста из `crates/wasm`; сам
//! кадровый цикл — существующая TS-часть.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use crate::painter::{PaintStats, Painter2D};
use crate::sab::SabRing;
use wasm_bindgen::prelude::*;
use web_sys::OffscreenCanvasRenderingContext2d;

thread_local! {
    /// Painter воркера: один на поток, воркер однопоточный.
    static PAINTER: RefCell<Option<OffscreenPainter>> = const { RefCell::new(None) };
}

/// Painter поверх OffscreenCanvas с жизненным циклом контекста.
pub struct OffscreenPainter {
    inner: Painter2D,
}

/// Реализация общего контракта painter'ов (ADR-0030): обёртка жизненного
/// цикла делегирует в тот же `Painter2D`, что и worker-путь.
impl crate::plugin::CanvasPainter for OffscreenPainter {
    fn paint_bytes(&mut self, bytes: &[u8]) -> Result<PaintStats, crate::plugin::PaintError> {
        self.paint_bytes(bytes)
            .map_err(|e| crate::plugin::PaintError(crate::plugin::js_error_string(&e)))
    }
}

impl OffscreenPainter {
    #[must_use]
    pub fn new(ctx: OffscreenCanvasRenderingContext2d) -> Self {
        Self {
            inner: Painter2D::new(ctx),
        }
    }

    /// Устанавливает единственный экземпляр воркера. Повторный вызов — ошибка.
    ///
    /// # Errors
    /// Если painter уже инициализирован.
    pub fn install(ctx: OffscreenCanvasRenderingContext2d) -> Result<(), JsValue> {
        PAINTER.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return Err(JsValue::from_str("painter already initialised"));
            }
            *slot = Some(Self::new(ctx));
            Ok(())
        })
    }

    /// Устанавливает painter поверх main-thread canvas-контекста,
    /// пришедшего как `JsValue`.
    ///
    /// Контекст проверяется duck-типами, а не приведением типа: вызывающий
    /// передаёт обычный `CanvasRenderingContext2d`, а painter работает с
    /// `OffscreenCanvasRenderingContext2d`. Оба контекста реализуют общие
    /// Canvas2D-миксины, painter трогает только их и `canvas.width/height`,
    /// поэтому после успешной проверки `unchecked_into` безопасен. Enum или
    /// generic над двумя контекстами отвергнут: он потребовал бы
    /// диспетчеризации 20+ методов ради одного места вызова.
    ///
    /// # Errors
    /// Если painter уже инициализирован, если у `ctx` нет свойства `canvas`
    /// (или это не объект) либо если какая-то из функций, которые painter
    /// вызывает, отсутствует.
    pub fn install_js(ctx: JsValue) -> Result<(), JsValue> {
        let canvas = js_sys::Reflect::get(&ctx, &JsValue::from_str("canvas"))
            .map_err(|_| JsValue::from_str("painter: ctx is not a Canvas2D context"))?;
        if !canvas.is_object() {
            return Err(JsValue::from_str(
                "painter: ctx is not a Canvas2D context (`canvas` is not an object)",
            ));
        }

        // Ровно те члены, которые Painter2D и его помощники дёргают на ctx.
        // Добавишь вызов в painter — добавь имя сюда, иначе main-режим
        // упадёт на существующем объекте только в рантайме.
        const REQUIRED_MEMBERS: &[&str] = &[
            "fillRect",
            "clearRect",
            "fillText",
            "measureText",
            "drawImage",
            "beginPath",
            "fill",
            "stroke",
            "setTransform",
            "save",
            "restore",
            "moveTo",
            "lineTo",
            "rect",
            "arc",
            "arcTo",
            "closePath",
            "clip",
            "setLineDash",
        ];
        for name in REQUIRED_MEMBERS {
            let member = js_sys::Reflect::get(&ctx, &JsValue::from_str(name)).map_err(|_| {
                JsValue::from_str(&format!("painter: cannot read `{name}` from ctx"))
            })?;
            if !member.is_function() {
                return Err(JsValue::from_str(&format!(
                    "painter: ctx is not a Canvas2D context (`{name}` is not a function)"
                )));
            }
        }

        // Комментарий-обоснование выше: проверены и `canvas`, и все методы,
        // которые painter зовёт; общие Canvas2D-миксины делают представление
        // OffscreenCanvasRenderingContext2d корректным для обоих контекстов.
        let ctx: OffscreenCanvasRenderingContext2d = ctx.unchecked_into();
        Self::install(ctx)
    }

    /// Снимает painter: воркер закрывает книгу или переинициализируется.
    pub fn dispose() {
        PAINTER.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }

    /// Доступ к установленному экземпляру.
    ///
    /// # Errors
    /// Если painter не инициализирован.
    pub fn with<R>(f: impl FnOnce(&mut Self) -> R) -> Result<R, JsValue> {
        PAINTER.with(|slot| {
            let mut slot = slot.borrow_mut();
            let painter = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("painter not initialised"))?;
            Ok(f(painter))
        })
    }

    /// Fallback-путь: байты DisplayList уже скопированы в Rust.
    ///
    /// # Errors
    /// Если байты — не DisplayList.
    pub fn paint_bytes(&mut self, bytes: &[u8]) -> Result<PaintStats, JsValue> {
        self.inner.paint_bytes(bytes)
    }

    /// SAB-путь: читает текущий слот ring'а, копирует, освобождает, рисует.
    ///
    /// # Errors
    /// Если слот не читается или DisplayList не разбирается.
    pub fn paint_sab(
        &mut self,
        sab: js_sys::SharedArrayBuffer,
        slot_capacity: u32,
    ) -> Result<PaintStats, JsValue> {
        let mut ring = SabRing::from_sab(sab, slot_capacity as usize)?;
        let mut buf = Vec::new();
        if !ring.copy_current_into(&mut buf)? {
            return Ok(PaintStats {
                cmds: 0,
                dropped: true,
                paint_ms: 0.0,
            });
        }
        // Слот освобождается сразу после копирования: без этого счётчик читателя
        // стоит на месте, оба слота считаются занятыми, и после второго кадра
        // запись навсегда упирается в «нет свободного слота».
        ring.release_current()?;
        self.paint_bytes(&buf)
    }

    /// Пересчитывает canvas под DPR и сбрасывает состояние painter'а.
    ///
    /// Установка `width`/`height` сбрасывает состояние 2D-контекста, поэтому кэш
    /// кисти обязан сброситься вместе с ним. Кэш `ImageBitmap` чистится тем же
    /// вызовом; сами битмапы к размеру холста не привязаны, и воркер
    /// регистрирует их заново сразу после resize.
    ///
    /// # Errors
    /// Если размер после умножения на DPR даёт нулевую сторону.
    pub fn resize(&mut self, css_w: f32, css_h: f32, dpr: f32) -> Result<(), JsValue> {
        let canvas = self.inner.context().canvas();
        let w = (css_w * dpr).round() as u32;
        let h = (css_h * dpr).round() as u32;
        if w == 0 || h == 0 {
            return Err(JsValue::from_str("canvas size must be > 0"));
        }
        canvas.set_width(w);
        canvas.set_height(h);
        self.inner.reset_state();
        Ok(())
    }

    /// Регистрирует `ImageBitmap` для команд `Image`.
    pub fn register_bitmap(&mut self, id: u32, bmp: web_sys::ImageBitmap) {
        self.inner.register_bitmap(id, bmp);
    }

    /// Снимает битмап; `true`, если он был зарегистрирован.
    #[must_use]
    pub fn drop_bitmap(&mut self, id: u32) -> bool {
        self.inner.drop_bitmap(id)
    }
}
