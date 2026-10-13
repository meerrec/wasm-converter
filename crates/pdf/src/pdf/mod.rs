//! Свой PDF-бэкенд вместо вендоренного форка (ADR-0022).
//!
//! Модуль даёт минимальный писатель PDF на модели значений `lopdf`:
//! сериализация объектов и xref (`writer`), модель операций страницы и её
//! трансляция в content stream (`ops`, `content`), шрифты (`fonts`) и сессия
//! постраничной записи (`session`). Публичный API крейта не меняется.

pub(crate) mod content;
pub(crate) mod fonts;
pub(crate) mod image;
pub(crate) mod ops;
pub(crate) mod session;
pub(crate) mod writer;
