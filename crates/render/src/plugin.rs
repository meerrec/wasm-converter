//! Общий контракт форматов: `FormatPlugin`, `DocumentModel` и `CanvasPainter`
//! (ADR-0030).
//!
//! Формат-плагин разбирает байты пакета в модель, модель отдаёт число единиц
//! рендера и собирает кадр; painter рисует байты DisplayList. DisplayList —
//! единственный контракт кадра: вторая командная иерархия (sketch `RenderCommand`
//! из аудита) не вводится, `build_display_list` возвращает те же байты, что
//! painter читает из SAB.
//!
//! Разбор синхронный (`async_trait` не используется): потоковый вход
//! буферизуется вызывающим до вызова [`FormatPlugin::parse`]. Диспетчеризация
//! форматов — на TS-стороне по явному `DocFormat`; магические байты не
//! нюхаются, реестр плагинов не создаётся (ADR-0030).
//!
//! `CanvasPainter` и `PaintError` — только под `#[cfg(target_arch = "wasm32")]`:
//! оба painter'а (`Painter2D`, `OffscreenPainter`) и `PaintStats` существуют
//! лишь в wasm, на нативном таргете рисовать нечем.

use doc_converter_core::ZipLimits;

/// Контекст сборки кадра: масштаб и физический размер вьюпорта.
///
/// Координаты кадра — физические пиксели canvas, поэтому ширина и высота —
/// физические пиксели, а `scale` уже включает зум и DPR. Первый кадр строится
/// от начала документа (лист 0 / страница 0); конкретные кадры wasm-клей
/// собирает существующими per-формат путями.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderContext {
    /// Масштаб: зум, умноженный на DPR.
    pub scale: f32,
    /// Физическая ширина вьюпорта, пиксели.
    pub viewport_width: u32,
    /// Физическая высота вьюпорта, пиксели.
    pub viewport_height: u32,
}

/// Единая ошибка trait-слоя форматов.
///
/// Конкретные ошибки крейтов (`docx::Error`, `xlsx::XlsxError`) сводятся к
/// строке на границе: переносить иерархии ошибок через `Box<dyn Error>` —
/// значит тащить их типы в общий контракт.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// Пакет не разобрался: битый ZIP/XML, чужая структура.
    #[error("parse error: {0}")]
    Parse(String),
    /// Раскладка не сошлась: не удалось измерить текст или распределить
    /// содержимое.
    #[error("layout error: {0}")]
    Layout(String),
    /// Операция отменена вызовом (ADR-0032): кооперативная отмена по флагу,
    /// состояние движка после неё цело.
    #[error("operation cancelled")]
    Cancelled,
}

/// Плагин формата: разбирает байты пакета в модель документа.
pub trait FormatPlugin: Send + Sync {
    /// Идентификатор формата — совпадает с `DocFormat` протокола TS.
    fn id(&self) -> &'static str;

    /// Разобрать пакет целиком.
    ///
    /// # Errors
    /// [`EngineError::Parse`], если байты не являются пакетом этого формата
    /// или пакет битый.
    fn parse(
        &self,
        bytes: &[u8],
        limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError>;
}

/// Разобранная модель документа: отдаёт число единиц рендера и собирает кадр.
pub trait DocumentModel: Send + Sync {
    /// Число единиц рендера: листов у XLSX, страниц у DOCX, слайдов у PPTX.
    fn page_count(&self) -> u32;

    /// Собрать первый кадр (лист 0 / страница 0) в байты DisplayList.
    ///
    /// # Errors
    /// [`EngineError::Layout`], если содержимое не удалось разложить,
    /// [`EngineError::Cancelled`], если операцию отменили.
    fn build_display_list(&self, ctx: &RenderContext) -> Result<Vec<u8>, EngineError>;
}

/// Painter кадра: рисует байты DisplayList в canvas-контекст.
///
/// Только wasm32: оба painter'а существуют лишь в wasm-сборке.
#[cfg(target_arch = "wasm32")]
pub trait CanvasPainter {
    /// Нарисовать кадр из байтов DisplayList.
    ///
    /// # Errors
    /// [`PaintError`], если байты не DisplayList или вызов JS-контекста упал.
    fn paint_bytes(&mut self, bytes: &[u8]) -> Result<crate::painter::PaintStats, PaintError>;
}

/// Ошибка рисования кадра.
///
/// Обе конкретные реализации возвращают `JsValue` — нативный код этого типа
/// не видит (wasm-only граница), поэтому ошибка сводится к строке: сообщение
/// JS-исключения либо «bytes are not a DisplayList».
#[cfg(target_arch = "wasm32")]
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PaintError(pub String);

/// Свести `JsValue` к строке сообщения для [`PaintError`].
#[cfg(target_arch = "wasm32")]
pub fn js_error_string(e: &wasm_bindgen::JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}
