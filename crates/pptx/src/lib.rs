//! PPTX: разбор PresentationML-пакета в нормализованную модель презентации
//! и раскладка слайдов фиксированным размером (ADR-0033).
//!
//! Модель (`model`) повторяет `PresentationML`: слайд — шейпы `p:sp`/`p:pic`/
//! `p:graphicFrame`, тема `a:theme` — шрифты и цвета, размер слайда — `p:sldSz`.
//! Каскад текста разрешается на разборе: `a:rPr` run'а → `a:defRPr` абзаца →
//! стиль уровня мастера → умолчания. Диаграммы — общая модель
//! `doc_converter_render::chart::ChartData` (ADR-0011).
//!
//! Раскладка (`layout`) — фиксированный размер слайда без пагинации: слайд —
//! одна страница рендера, координаты EMU переводятся в пиксели при 96 dpi.
//! Рендер (`render`) собирает `DisplayList` — единственный контракт кадра.
//!
//! Разбор синхронный (ADR-0030 §в): потоковый вход буферизуется вызывающим до
//! [`parse`]; отмена — кооперативный флаг между слайдами (ADR-0032).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]
// Имена типов повторяют имена модулей модели (`layout::SlideLayout`).
#![allow(clippy::module_name_repetitions)]
// Свойства знака OOXML — набор bool-атрибутов (`RunProps`).
#![allow(clippy::struct_excessive_bools)]

pub mod error;
pub mod export;
pub mod layout;
pub mod model;
pub mod parse;
pub mod plugin;
pub mod render;
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
