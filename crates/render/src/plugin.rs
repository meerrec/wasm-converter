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

use doc_converter_core::wire::{DiagnosticCodeWire, DiagnosticLayerWire, DiagnosticWire};
use doc_converter_core::ZipLimits;

/// Вид документа, который обслуживает плагин.
///
/// Вид задаёт единицу рендера: лист у таблиц, страница у текстовых
/// документов, слайд у презентаций. Диспетчеризация форматов живёт на
/// TS-стороне, но навигация и счётчики берут вид отсюда, а не выводят его из
/// `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    /// Таблицы: XLSX и родственные книги.
    Sheets,
    /// Текстовые документы: DOCX и родственные.
    Pages,
    /// Презентации: PPTX и родственные.
    Slides,
}

/// Паспорт плагина: идентификатор, вид документа и расширения пакета.
///
/// `extensions` — расширения без точки, в нижнем регистре: по ним интерфейс
/// выбирает воркер, не открывая байты. Паспорт статичен: строка
/// идентификатора и срез расширений живут всё время работы плагина, поэтому
/// копирование `PluginInfo` ничего не стоит.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginInfo {
    /// Идентификатор формата — совпадает с `DocFormat` протокола TS.
    pub id: &'static str,
    /// Вид документа — задаёт единицу рендера.
    pub kind: FormatKind,
    /// Расширения пакета без точки: `["docx", "docm"]`.
    pub extensions: &'static [&'static str],
}

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
    /// Экспорт не удался: кадр или документ не собрался в целевой формат.
    #[error("export error: {0}")]
    Export(String),
    /// Формат или его часть не поддержаны этой сборкой.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Лимит ресурсов исчерпан (ADR-0015): пакет больше допустимого.
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// Внутренняя ошибка движка: нарушен инвариант, вины пользователя нет.
    #[error("internal error: {0}")]
    Internal(String),
}

impl EngineError {
    /// Структурная диагностика для wire-протокола (ADR-0028).
    ///
    /// `format` остаётся `None`: какой формат обрабатывался, знает вызывающий,
    /// а не ошибка trait-слоя. `retryable` истинно только у отмены — повтор
    /// операции после неё корректен, у остальных вариантов повтор упадёт так
    /// же.
    #[must_use]
    pub fn diagnostic(&self) -> DiagnosticWire {
        let (code, layer, retryable) = match self {
            Self::Parse(_) => (
                DiagnosticCodeWire::MalformedPackage,
                DiagnosticLayerWire::Parse,
                false,
            ),
            Self::Layout(_) => (
                DiagnosticCodeWire::LayoutFailed,
                DiagnosticLayerWire::Layout,
                false,
            ),
            Self::Cancelled => (
                DiagnosticCodeWire::Cancelled,
                DiagnosticLayerWire::Orchestration,
                true,
            ),
            Self::Export(_) => (
                DiagnosticCodeWire::ExportFailed,
                DiagnosticLayerWire::Export,
                false,
            ),
            Self::Unsupported(_) => (
                DiagnosticCodeWire::UnsupportedFormat,
                DiagnosticLayerWire::Orchestration,
                false,
            ),
            Self::Limit(_) => (
                DiagnosticCodeWire::LimitExceeded,
                DiagnosticLayerWire::Parse,
                false,
            ),
            Self::Internal(_) => (
                DiagnosticCodeWire::Internal,
                DiagnosticLayerWire::Internal,
                false,
            ),
        };
        DiagnosticWire {
            code,
            layer,
            message: self.to_string(),
            format: None,
            retryable,
        }
    }
}

/// Плагин формата: разбирает байты пакета в модель документа.
pub trait FormatPlugin: Send + Sync {
    /// Паспорт плагина: идентификатор, вид документа и расширения.
    fn info(&self) -> PluginInfo;

    /// Идентификатор формата — совпадает с `DocFormat` протокола TS.
    ///
    /// Удобная обёртка над [`Self::info`]: реализациям достаточно описать
    /// паспорт, отдельный метод `id` не нужен.
    fn id(&self) -> &'static str {
        self.info().id
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_is_a_malformed_package() {
        let diag = EngineError::Parse("bad zip".to_owned()).diagnostic();
        assert_eq!(diag.code, DiagnosticCodeWire::MalformedPackage);
        assert_eq!(diag.layer, DiagnosticLayerWire::Parse);
        assert_eq!(diag.message, "parse error: bad zip");
        assert!(!diag.retryable);
        assert_eq!(diag.format, None);
    }

    #[test]
    fn cancelled_is_retryable_and_orchestrated() {
        let diag = EngineError::Cancelled.diagnostic();
        assert_eq!(diag.code, DiagnosticCodeWire::Cancelled);
        assert_eq!(diag.layer, DiagnosticLayerWire::Orchestration);
        // Отмена — не поломка состояния: повтор операции корректен.
        assert!(diag.retryable);
    }

    #[test]
    fn export_error_is_an_export_failure() {
        let diag = EngineError::Export("pdf writer failed".to_owned()).diagnostic();
        assert_eq!(diag.code, DiagnosticCodeWire::ExportFailed);
        assert_eq!(diag.layer, DiagnosticLayerWire::Export);
        assert!(!diag.retryable);
    }
}
