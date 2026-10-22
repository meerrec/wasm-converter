//! Раскладка DOCX-документа по страницам.
//!
//! Единая точка входа — [`layout_document`], постраничную раскладку ведёт [`Paginator`].
//! Логика переноса строк опирается на [`doc_converter_render::text_measure`]
//! (ADR-0005), чтобы точки разрыва не разъезжались между canvas и PDF.
//!
//! # Пример
//!
//! ```ignore
//! use docx::layout::{layout_document, LayoutOptions};
//! use docx::model::Document;
//!
//! let doc: Document = ...;
//! let options = LayoutOptions::default();
//! let layout = layout_document(&doc, &options)?;
//!
//! assert_eq!(layout.pages.len(), 3);
//! ```

pub mod bidi;
pub mod cascade;
pub mod engine;
pub mod fields;
pub mod float;
pub mod line_break;
pub mod pagination;
pub mod paragraph;
pub mod tables;

pub use engine::{
    half_points_to_px, layout_document, layout_document_with_cancel, twips_to_px, LayoutError,
    LayoutItem, LayoutOptions, LayoutState, Page, PageChrome, PageLayout, Rect, TableCellLayout,
    VAlign,
};
pub use float::{layout_float, layout_floats_on_page, FloatElement, FloatKind, FloatLayoutContext};
pub use line_break::LineBreaker;
pub use pagination::Paginator;
// Рядом с `TableCellLayout`: строка — такой же элемент `LayoutItem::Table`, и
// потребителю (canvas- и PDF-рендер) нужен её тип в том же пространстве имён.
pub use tables::TableRowLayout;
