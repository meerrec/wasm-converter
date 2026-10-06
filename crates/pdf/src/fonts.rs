//! Встраивание шрифтов в PDF.
//!
//! Байты берутся из `doc-converter-render` — тот же Carlito, которым canvas
//! измеряет текст (`render::font`). Один шрифт на экран и на PDF — залог
//! совпадения переносов (ADR-0007); своей копии TTF в pdf-крейте нет.
//!
//! Встраиваются только те начертания, что встретились у ячеек страницы:
//! каждое подмножество весит около 90 КБ, и возить неиспользованные незачем.
//! Метрики при этом общие — regular (`DEFAULT_FONT_ID`), как у canvas-пути:
//! начертание выбирает встраиваемый шрифт записи текста, а не точки переноса.

use doc_converter_render::font::{
    bold_font_bytes, bold_italic_font_bytes, default_font_bytes, italic_font_bytes,
};
use printpdf::{FontId as PdfFontId, ParsedFont, PdfDocument, PdfWarnMsg};

use crate::PdfError;

/// Начертание шрифта — то, что записано в `styles.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Face {
    /// Начертание по флагам стиля ячейки.
    #[must_use]
    pub const fn of(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Self::Regular,
            (true, false) => Self::Bold,
            (false, true) => Self::Italic,
            (true, true) => Self::BoldItalic,
        }
    }

    /// Место начертания в [`FaceSet`].
    const fn index(self) -> usize {
        match self {
            Self::Regular => 0,
            Self::Bold => 1,
            Self::Italic => 2,
            Self::BoldItalic => 3,
        }
    }

    /// Байты подрезанного Carlito этого начертания.
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Regular => default_font_bytes(),
            Self::Bold => bold_font_bytes(),
            Self::Italic => italic_font_bytes(),
            Self::BoldItalic => bold_italic_font_bytes(),
        }
    }
}

/// Набор начертаний, встретившихся на страницах листа.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaceSet([bool; 4]);

impl FaceSet {
    /// Добавить начертание в набор.
    pub fn insert(&mut self, face: Face) {
        self.0[face.index()] = true;
    }

    /// Есть ли начертание в наборе.
    #[must_use]
    pub fn contains(self, face: Face) -> bool {
        self.0[face.index()]
    }

    /// Добавить все начертания другого набора.
    ///
    /// Наборы собираются по листам, а шрифты встраиваются один раз на
    /// документ: книге нужен общий набор, а не по листу.
    pub fn merge(&mut self, other: Self) {
        for (slot, present) in self.0.iter_mut().zip(other.0) {
            *slot |= present;
        }
    }
}

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

/// Шрифты страниц: regular всегда, выделенные начертания — как встретились.
#[derive(Debug)]
pub struct EmbeddedFonts {
    regular: EmbeddedFont,
    bold: Option<EmbeddedFont>,
    italic: Option<EmbeddedFont>,
    bold_italic: Option<EmbeddedFont>,
}

impl EmbeddedFonts {
    /// Шрифт начертания; невстроенное начертание заменяется regular.
    ///
    /// Замена — страховка на случай, если спрашивают начертание, которого в
    /// наборе не было: пустой записи текста printpdf не примет.
    #[must_use]
    pub fn id(&self, face: Face) -> &PdfFontId {
        let embedded = match face {
            Face::Regular => None,
            Face::Bold => self.bold.as_ref(),
            Face::Italic => self.italic.as_ref(),
            Face::BoldItalic => self.bold_italic.as_ref(),
        };
        embedded.unwrap_or(&self.regular).id()
    }
}

/// Встроить начертания из набора (и всегда — regular).
///
/// Разбор TTF и подстановка (subsetting) — на стороне printpdf: он же
/// собирает `ToUnicode`, без которого кириллицу не извлечь из готового PDF.
///
/// # Errors
/// [`PdfError::Font`], если printpdf не принял байты шрифта.
pub fn embed(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
    faces: FaceSet,
) -> Result<EmbeddedFonts, PdfError> {
    Ok(EmbeddedFonts {
        regular: embed_face(doc, warnings, Face::Regular)?,
        bold: embed_optional(doc, warnings, faces, Face::Bold)?,
        italic: embed_optional(doc, warnings, faces, Face::Italic)?,
        bold_italic: embed_optional(doc, warnings, faces, Face::BoldItalic)?,
    })
}

/// Встроить начертание, только если оно есть в наборе.
fn embed_optional(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
    faces: FaceSet,
    face: Face,
) -> Result<Option<EmbeddedFont>, PdfError> {
    if !faces.contains(face) {
        return Ok(None);
    }
    embed_face(doc, warnings, face).map(Some)
}

/// Встроить одно начертание.
fn embed_face(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
    face: Face,
) -> Result<EmbeddedFont, PdfError> {
    let parsed = ParsedFont::from_bytes(face.bytes(), 0, warnings)
        .ok_or_else(|| PdfError::Font(format!("printpdf rejected the {face:?} font")))?;
    Ok(EmbeddedFont {
        id: doc.add_font(&parsed),
    })
}
