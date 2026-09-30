//! OOXML core: ZIP-архив, streaming XML, relationships, метаданные.
//!
//! Крейт не знает ничего про конкретные форматы (DOCX/XLSX) — он даёт
//! низкоуровневые примитивы: чтение `[Content_Types].xml`, `_rels/*.rels`,
//! распаковку part'ов и потоковый XML-парсер. Таблица общих строк — понятие
//! формата XLSX, она живёт в `doc-converter-xlsx`.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod archive;
pub mod error;
pub mod rels;
pub mod xml;

pub use archive::Archive;
pub use error::{Error, Result};

/// Версия OOXML (ECMA-376 5-е издание), которую поддерживает движок.
pub const OOXML_ECMA_376: &str = "ECMA-376:2021";

/// Части (parts), обязательные в любом OOXML-пакете.
pub const CONTENT_TYPES: &str = "[Content_Types].xml";
pub const ROOT_RELS: &str = "_rels/.rels";
