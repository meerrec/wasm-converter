//! ODF (`OpenDocument` Format) → модели OOXML (`doc-converter-xlsx`,
//! `doc-converter-pptx`, `doc-converter-docx`).
//!
//! Пакет читается через [`doc_converter_core::Archive`], каталог — из
//! `META-INF/manifest.xml`. Срез ODS доводит `content.xml` до
//! [`Workbook`](doc_converter_xlsx::Workbook), срез ODP — до `Presentation`
//! модели `doc-converter-pptx`, срез ODT — до
//! [`Document`](doc_converter_docx::Document). Просмотр, раскладка и PDF у
//! презентаций и документов работают на PPTX/DOCX-семействах без отдельного
//! движка ODF (ADR-0046).
//!
//! ```no_run
//! # #[cfg(feature = "ods")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("table.ods")?;
//! let book = doc_converter_odf::open(bytes)?;
//! println!("листов: {}", book.sheets().len());
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "ods"))]
//! # fn main() {}
//! ```
//!
//! # Чего в этом срезе нет
//!
//! Стили ячеек ODS (шрифт, заливка, рамки), печать, картинки и диаграммы —
//! следующие срезы; числовые форматы и геометрия поддержаны частично. ODP
//! переносит текст, кегль/цвет/начертание прогонов, заливку/контур шейпов,
//! картинки и размер слайда; маркеры списков, анимации и диаграммы — следующие
//! срезы (см. модуль `odp`). ODT переносит заголовки, абзацы, прогоны,
//! гиперссылки, списки (плоскими абзацами), таблицы с горизонтальными
//! объединениями, картинки и раскладку страницы; маркеры списков, колонтитулы,
//! границы ячеек и нумерация — следующие срезы (см. модуль `odt`).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod error;
#[cfg(any(feature = "ods", feature = "odp", feature = "odt"))]
pub mod manifest;
#[cfg(feature = "odp")]
pub mod odp;
#[cfg(feature = "ods")]
pub mod ods;
#[cfg(feature = "odt")]
pub mod odt;
#[cfg(any(feature = "ods", feature = "odp", feature = "odt"))]
pub mod plugin;

#[cfg(any(feature = "ods", feature = "odp", feature = "odt"))]
mod xml;

#[cfg(feature = "ods")]
pub use ods::open;
// Корневой `open` — только когда включён ровно один формат: при нескольких
// включённых фичах имена `ods::open`/`odp::open`/`odt::open` конфликтовали бы,
// поэтому приоритет такой: ODS, затем ODP, затем ODT — остальные доступны как
// `odp::open`/`odt::open`.
#[cfg(all(feature = "odp", not(feature = "ods")))]
pub use odp::open;
#[cfg(all(feature = "odt", not(feature = "ods"), not(feature = "odp")))]
pub use odt::open;
