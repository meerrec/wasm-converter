//! Рендер PPTX: раскладка слайдов в `DisplayList` и плагин формата по
//! общему контракту ADR-0030/0033.
//!
//! `render` собирает кадр одного слайда из готовой раскладки
//! (`doc-converter-pptx-layout`): команды — те же `Rect`/`Line`/`Text`/
//! `Image`/`Chart`, что у DOCX/XLSX; формат `DisplayList` и `DL_VERSION` не
//! меняются. `plugin` реализует `FormatPlugin`/`DocumentModel`
//! (`PptxPlugin id "pptx"`, `PptxDocument`: `page_count` — число слайдов,
//! `build_display_list` — первый слайд от начала координат).
//!
//! PDF здесь нет: экспорт живёт в `doc-converter-pptx-pdf` (просмотрщик и
//! экспортёр разнесены по модулям, как у DOCX).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod plugin;
pub mod render;

// Раскладка переэкспортируется для wasm-клея: `pptx-wasm` держит готовые
// `SlideLayout` и не ходит за ними в крейт раскладки напрямую.
pub use doc_converter_pptx_layout::{layout_slide, SlideLayout};
