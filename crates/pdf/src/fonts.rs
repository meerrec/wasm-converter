//! Встраивание шрифта в PDF.
//!
//! Байты берутся из `doc-converter-render` — тот же Carlito, которым canvas
//! измеряет текст (`render::font::default_font_bytes`). Один шрифт на экран и
//! на PDF — залог совпадения переносов (ADR-0007); своей копии TTF в pdf-крейте
//! нет.
//!
//! Подрезанные Bold/Italic/BoldItalic и подстановка начертания по флагу
//! `Font::bold` — задача C4 спринта.

use doc_converter_render::font::default_font_bytes;
use printpdf::{FontId as PdfFontId, ParsedFont, PdfDocument, PdfWarnMsg};

use crate::PdfError;

/// Шрифт, встроенный в документ printpdf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedFont {
    id: PdfFontId,
}

impl EmbeddedFont {
    /// Идентификатор в ресурсах документа: им помечаются текстовые операции.
    #[must_use]
    pub const fn id(&self) -> &PdfFontId {
        &self.id
    }
}

/// Встроить шрифт по умолчанию (Carlito regular) в документ.
///
/// Разбор TTF и подстановка (subsetting) — на стороне printpdf: он же
/// собирает `ToUnicode`, без которого кириллицу не извлечь из готового PDF.
///
/// # Errors
/// [`PdfError::Font`], если printpdf не принял байты шрифта.
pub fn embed_default(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
) -> Result<EmbeddedFont, PdfError> {
    let parsed = ParsedFont::from_bytes(default_font_bytes(), 0, warnings)
        .ok_or_else(|| PdfError::Font("printpdf rejected the default font".to_owned()))?;
    Ok(EmbeddedFont {
        id: doc.add_font(&parsed),
    })
}
