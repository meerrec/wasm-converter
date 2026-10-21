//! PPTX: разбор PresentationML-пакета в нормализованную модель презентации
//! (ADR-0033).
//!
//! Модель (`model`) повторяет `PresentationML`: слайд — шейпы `p:sp`/`p:pic`/
//! `p:graphicFrame`, тема `a:theme` — шрифты и цвета, размер слайда — `p:sldSz`.
//! Каскад текста разрешается на разборе: `a:rPr` run'а → `a:defRPr` абзаца →
//! стиль уровня мастера → умолчания. Диаграммы — общая модель
//! `doc_converter_render::chart::ChartData` (ADR-0011).
//!
//! Раскладки и `DisplayList` здесь нет: они вынесены в `doc-converter-pptx-layout`
//! и `doc-converter-pptx-render`, PDF — в `doc-converter-pptx-pdf`, чтобы
//! нативный разбор не тянул метрики шрифта и рендер (ADR-0033, решение
//! владельца).
//!
//! Разбор синхронный (ADR-0030 §в): потоковый вход буферизуется вызывающим до
//! [`parse`]; отмена — кооперативный флаг между слайдами (ADR-0032).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]
// Свойства знака OOXML — набор bool-атрибутов (`RunProps`).
#![allow(clippy::struct_excessive_bools)]

pub mod error;
pub mod export;
pub mod model;
pub mod parse;
pub(crate) mod xml;

pub use error::{Error, Result};
pub use model::Presentation;

use doc_converter_core::zip_limits::ZipLimits;

/// Открыть PPTX из сырых байт.
///
/// Владение буфером берётся, чтобы отдать его разбору без копии (та же
/// конвенция, что у `doc_converter_docx::open`). Срезовый [`parse`] остаётся
/// для вызывающих, у которых есть только срез.
///
/// # Errors
/// Если байты не OOXML-пакет, в нём нет `ppt/presentation.xml`, пакет нарушает
/// лимиты ADR-0015 или в презентации нет слайдов.
pub fn open(bytes: Vec<u8>) -> Result<Presentation> {
    parse::parse_owned(bytes, ZipLimits::default())
}
