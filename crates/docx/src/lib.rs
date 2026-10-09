//! DOCX: разбор OOXML-пакета в нормализованную модель документа.
//!
//! Модель (`model`) повторяет `WordprocessingML` как есть: каскад стилей на
//! разборе не разрешается, и сырые свойства абзаца и знака достаются
//! потребителю такими, какими были в XML, — сводить их будет Спринт 9
//! (ADR-0013). Сборка документа — [`open`]: части пакета разбираются по
//! отдельности, а нарушения, не мешающие читать документ, копятся в
//! [`Document::warnings()`].
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]
// Имена типов повторяют имена модулей модели (`model::table::Table`, `model::style::StyleTable`).
#![allow(clippy::module_name_repetitions)]
// Свойства OOXML — это буквально наборы bool-атрибутов (`TableLook`, `CompatSettings`).
#![allow(clippy::struct_excessive_bools)]

pub mod error;
pub mod model;

// Парсеры частей пакета и общий контекст разбора.
mod comments;
mod context;
mod document;
mod footnotes;
mod metadata;
mod numbering;
mod parse;
mod rels;
mod settings;
mod styles;
mod xml;

pub use error::{Error, Result};
// Поимённо, а не `pub use model::*`: `model` реэкспортирует свои модули, и
// глоб затенял бы `model::numbering`/`model::settings` приватными модулями
// парсеров с теми же именами (`hidden_glob_reexports`). Типы наружу при этом
// попадают все — как в `doc-converter-xlsx`.
pub use model::annotation::{Comment, Footnote, NoteKind};
pub use model::drawing::{
    AlignH, AlignV, Anchor, Extent, InlineImage, InlineOrAnchor, PositionH, PositionV, RelFromH,
    RelFromV, WrapKind,
};
pub use model::inline::{
    Bookmark, BreakKind, Field, FieldKind, Hyperlink, Inline, Paragraph, Run, RunContent,
};
pub use model::numbering::{
    AbstractNum, AbstractNumId, LevelSuffix, Lvl, LvlOverride, MultiLevelType, Num, NumFmt, NumId,
    NumberingTable,
};
pub use model::raw::{
    Border, BorderStyle, CellVAlign, CharacterSpacing, Color, FontHint, HalfPoint, Highlight, Ind,
    Justification, LineSpacing, LineSpacingRule, NumPr, ParagraphBorders, ParagraphSpacing, RFonts,
    RawPPr, RawRPr, Shading, ShadingPattern, StyleId, TabLeader, TabStop, TabStopKind, Toggle,
    Twips, Underline, VertAlign,
};
pub use model::section::{
    ColumnDef, Columns, HeaderFooter, Margins, Orientation, PageSize, PartRef, Section,
    SectionProperties, SectionType,
};
pub use model::settings::{
    CharacterSpacingControl, CompatSettings, EndnotePos, EndnotePr, FootnotePos, FootnotePr,
    NumRestart, Settings,
};
pub use model::style::{
    CharacterStyle, ConditionalFormat, DefaultStyleIds, DocDefaults, NumberingStyle,
    ParagraphStyle, StyleTable, TableStyle, TableStyleCondition,
};
pub use model::table::{
    Cell, CellBorders, CellMargins, CellWidth, GridCol, HeightRule, Row, RowHeight, Table,
    TableBorders, TableLayout, TableLook, TableWidth, VMerge,
};
pub use model::{BlockItem, Body, Document, Metadata, Relationships};
pub use parse::parse_docx;

use doc_converter_core::zip_limits::ZipLimits;

/// Открыть DOCX из сырых байт.
///
/// Владение буфером берётся, хотя разбору пока хватает среза: так подпись
/// совпадает с прежним публичным API и с тем, что отдаёт вызывающий (файл или
/// `fetch`), а `parse_docx` по контракту слайса S12 принимает срез.
///
/// # Errors
/// Если байты не OOXML-пакет, в нём нет `word/document.xml` или разбор упёрся в фатальное нарушение
/// (см. [`Error`]); восстановимые нарушения попадают в [`Document::warnings()`].
#[allow(clippy::needless_pass_by_value)]
pub fn open(bytes: Vec<u8>) -> Result<Document> {
    parse::parse_docx(&bytes, ZipLimits::default())
}
