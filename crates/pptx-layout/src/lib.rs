//! Раскладка слайдов PPTX: фиксированный размер, без пагинации (ADR-0033 §4).
//!
//! Слайд — одна страница рендера; координаты EMU переводятся в пиксели при
//! 96 dpi ([`doc_converter_pptx::model::emu_to_px`]). Текст переносится по
//! словам теми же метриками, что у DOCX (`doc_converter_render::text_measure`):
//! прогон знает свой кегль, строка собирается жадным word wrap'ом, высота
//! строки — максимум кеглей прогонов × 1.2. Автоподбор кегля
//! (`a:normAutofit`) не применяется.
//!
//! Раскладка не зависит от окна и зума: их знает рендер
//! (`doc-converter-pptx-render`), поэтому считается один раз при открытии
//! презентации. `DisplayList` сюда не входит — этот крейт только геометрия
//! слайда, пригодная и canvas-пути, и PDF-экспорту (`doc-converter-pptx-pdf`).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod layout;

pub use layout::{layout_slide, Item, PlacedRun, SlideLayout, TextLine};
