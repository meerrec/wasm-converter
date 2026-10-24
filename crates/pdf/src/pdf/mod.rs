//! Свой PDF-бэкенд вместо вендоренного форка (ADR-0022).
//!
//! Модуль даёт минимальный писатель PDF на модели значений `lopdf`:
//! сериализация объектов и xref (`writer`), модель операций страницы и её
//! трансляция в content stream (`ops`, `content`), шрифты (`fonts`) и сессия
//! постраничной записи (`session`). Публичный API крейта не меняется; наружу
//! бэкенд открыт для экспортёров форматов вне крейта (`doc-converter-pptx-pdf`,
//! ADR-0033), кроме `writer` — он внутренняя деталь сессии.

pub mod content;
pub mod export;
pub mod fonts;
pub mod image;
pub mod ops;
pub mod session;
mod writer;
