//! Плагины ODF по общему контракту `render::plugin` (ADR-0030/0046).
//!
//! `OdsPlugin` разбирает ODS в `Workbook` модели `doc-converter-xlsx`,
//! `OdpPlugin` — ODP в `Presentation` модели `doc-converter-pptx`. Оба плагина
//! живут в одном файле под раздельными `#[cfg]`: раскладка, painter и PDF не
//! знают, из какого формата пришла модель, — это и есть «новый формат без
//! второго движка».
//!
//! Горячий путь wasm в будущем держит модель в `thread_local!` и собирает кадры
//! напрямую (`xlsx::paint`/`pptx::render`); контракт нужен тестам и общему
//! `FormatPlugin`-пути.

use doc_converter_core::ZipLimits;
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::plugin::{
    DocumentModel, EngineError, FormatKind, FormatPlugin, PluginInfo, RenderContext,
};
use doc_converter_render::viewport::Viewport;

use crate::error::OdfError;

#[cfg(feature = "odp")]
use doc_converter_pptx_layout::{layout_slide, SlideLayout};
#[cfg(feature = "ods")]
use doc_converter_xlsx::paint::{self, PaintOptions};
#[cfg(feature = "ods")]
use doc_converter_xlsx::Workbook;
// Флаг отмены раскладки нужен ODS (paint) и ODT (layout_document_with_cancel).
#[cfg(any(feature = "ods", feature = "odt"))]
use std::cell::Cell;

#[cfg(feature = "odt")]
use doc_converter_docx::layout::{layout_document_with_cancel, LayoutOptions, PageLayout};
#[cfg(feature = "odt")]
use doc_converter_docx::Document;
#[cfg(feature = "odt")]
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};

/// Плагин ODS: `info().id` → `"ods"`, как `DocFormat` протокола TS.
#[cfg(feature = "ods")]
#[derive(Debug, Default)]
pub struct OdsPlugin;

#[cfg(feature = "ods")]
impl FormatPlugin for OdsPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "ods",
            kind: FormatKind::Sheets,
            extensions: &["ods"],
        }
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не ODF-пакет ODS или его часть не
    /// разбирается; [`EngineError::Unsupported`] — пакет ODT/ODP или чужой
    /// mimetype; [`EngineError::Limit`] — повторы вышли за лимиты модели.
    fn parse(
        &self,
        bytes: &[u8],
        _limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        // `open` забирает буфер во владение: архив живёт до конца разбора.
        // Лимиты — дефолтные (ADR-0015), как у `XlsxPlugin`.
        let book = crate::open(bytes.to_vec()).map_err(engine_error)?;
        Ok(Box::new(OdsDocument { workbook: book }))
    }
}

/// Книга ODS как `DocumentModel`: число единиц рендера — число листов,
/// первый кадр — первый лист от начала координат.
#[cfg(feature = "ods")]
pub struct OdsDocument {
    workbook: Workbook,
}

#[cfg(feature = "ods")]
impl OdsDocument {
    /// Модель книги — доступ к листам, ячейкам и стилям в обход trait-слоя.
    #[must_use]
    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }
}

#[cfg(feature = "ods")]
impl DocumentModel for OdsDocument {
    fn page_count(&self) -> u32 {
        u32::try_from(self.workbook.sheets().len()).unwrap_or(u32::MAX)
    }

    /// # Errors
    /// [`EngineError::Parse`], если в книге нет ни одного листа (для ODF это
    /// битый пакет: `office:spreadsheet` обязан нести хотя бы одну таблицу).
    #[allow(clippy::cast_precision_loss)]
    fn build_display_list(&self, ctx: &RenderContext) -> Result<Vec<u8>, EngineError> {
        let sheet = self
            .workbook
            .sheets()
            .first()
            .ok_or_else(|| EngineError::Parse("workbook has no sheets".to_owned()))?;
        let viewport = Viewport::new(
            0.0,
            0.0,
            ctx.viewport_width as f32,
            ctx.viewport_height as f32,
            ctx.scale,
        );
        let mut out = DisplayList::new();
        paint::build(
            &self.workbook,
            sheet,
            viewport,
            &PaintOptions::default(),
            &mut out,
            &Cell::new(false),
        )
        .map_err(|error| EngineError::Layout(error.to_string()))?;
        Ok(out.to_bytes())
    }
}

/// Плагин ODP: `info().id` → `"odp"`, как `DocFormat` протокола TS.
#[cfg(feature = "odp")]
#[derive(Debug, Default)]
pub struct OdpPlugin;

#[cfg(feature = "odp")]
impl FormatPlugin for OdpPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "odp",
            kind: FormatKind::Slides,
            extensions: &["odp"],
        }
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не ODF-пакет ODP или его часть не
    /// разбирается; [`EngineError::Unsupported`] — пакет ODT/ODS или чужой
    /// mimetype; [`EngineError::Layout`], если раскладка слайда не собралась.
    fn parse(
        &self,
        bytes: &[u8],
        _limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        let presentation = crate::odp::open(bytes.to_vec()).map_err(engine_error)?;
        // Раскладка считается здесь и один раз: размер слайда задан
        // презентацией, а не окном (как у `PptxPlugin`).
        let layouts = (0..presentation.slide_count())
            .map(|index| layout_slide(&presentation, index))
            .collect::<doc_converter_pptx::Result<Vec<_>>>()
            .map_err(|error| EngineError::Layout(error.to_string()))?;
        Ok(Box::new(OdpDocument { layouts }))
    }
}

/// Презентация ODP как `DocumentModel`: число единиц рендера — число слайдов,
/// первый кадр — первый слайд от начала координат.
#[cfg(feature = "odp")]
pub struct OdpDocument {
    layouts: Vec<SlideLayout>,
}

#[cfg(feature = "odp")]
impl DocumentModel for OdpDocument {
    // Число слайдов в u32 влезает всегда (предел — пакеты по лимитам ADR-0015).
    #[allow(clippy::cast_possible_truncation)]
    fn page_count(&self) -> u32 {
        self.layouts.len() as u32
    }

    /// # Errors
    /// [`EngineError::Parse`], если в презентации нет ни одного слайда (для ODF
    /// это битый пакет).
    #[allow(clippy::cast_precision_loss)]
    fn build_display_list(&self, ctx: &RenderContext) -> Result<Vec<u8>, EngineError> {
        let layout = self
            .layouts
            .first()
            .ok_or_else(|| EngineError::Parse("presentation has no slides".to_owned()))?;
        let viewport = Viewport::new(
            0.0,
            0.0,
            ctx.viewport_width as f32,
            ctx.viewport_height as f32,
            ctx.scale,
        );
        let mut out = DisplayList::new();
        doc_converter_pptx_render::render::build(layout, &viewport, &mut out);
        Ok(out.to_bytes())
    }
}

/// Плагин ODT: `info().id` → `"odt"`, как `DocFormat` протокола TS.
#[cfg(feature = "odt")]
#[derive(Debug, Default)]
pub struct OdtPlugin;

#[cfg(feature = "odt")]
impl FormatPlugin for OdtPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "odt",
            kind: FormatKind::Pages,
            extensions: &["odt"],
        }
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не ODF-пакет ODT или его часть не
    /// разбирается; [`EngineError::Unsupported`] — пакет ODS/ODP или чужой
    /// mimetype; [`EngineError::Layout`], если раскладка не собралась.
    fn parse(
        &self,
        bytes: &[u8],
        _limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        let document = crate::odt::open(bytes.to_vec()).map_err(engine_error)?;
        // Раскладка считается здесь и один раз: ширина полосы набора задана
        // страницей, а не окном (как у `DocxPlugin`). Шрифты — свежий реестр без
        // зарегистрированных гарнитур: раскладка берёт fallback-метрики.
        let mut fonts = FontRegistry::new(DEFAULT_FONT_CACHE);
        let layout = layout_document_with_cancel(
            &document,
            &LayoutOptions::default(),
            &mut fonts,
            &Cell::new(false),
        )
        .map_err(|error| EngineError::Layout(error.to_string()))?;
        Ok(Box::new(OdtDocument { document, layout }))
    }
}

/// Документ ODT как `DocumentModel`: число единиц рендера — число страниц
/// раскладки, первый кадр — первая страница от начала координат.
#[cfg(feature = "odt")]
pub struct OdtDocument {
    document: Document,
    layout: PageLayout,
}

#[cfg(feature = "odt")]
impl OdtDocument {
    /// Модель документа — абзацы, таблицы, медиачасти — в обход trait-слоя.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }
}

#[cfg(feature = "odt")]
impl DocumentModel for OdtDocument {
    // Число страниц в u32 влезает всегда (предел — пакеты по лимитам ADR-0015).
    #[allow(clippy::cast_possible_truncation)]
    fn page_count(&self) -> u32 {
        self.layout.pages.len() as u32
    }

    /// # Errors
    /// Пока — никогда: раскладка уже посчитана при [`FormatPlugin::parse`].
    #[allow(clippy::cast_precision_loss)]
    fn build_display_list(&self, ctx: &RenderContext) -> Result<Vec<u8>, EngineError> {
        let viewport = Viewport::new(
            0.0,
            0.0,
            ctx.viewport_width as f32,
            ctx.viewport_height as f32,
            ctx.scale,
        );
        let mut out = DisplayList::new();
        doc_converter_docx::render::build(&self.layout, 0, viewport, &mut out);
        Ok(out.to_bytes())
    }
}

/// Ошибку ODF — в ошибку trait-слоя: UNSUPPORTED и LIMIT сохраняют вид,
/// остальные считаются ошибкой разбора.
fn engine_error(error: OdfError) -> EngineError {
    match error {
        OdfError::Unsupported(message) => EngineError::Unsupported(message),
        OdfError::Limit(message) => EngineError::Limit(message),
        other => EngineError::Parse(other.to_string()),
    }
}

#[cfg(all(test, feature = "ods"))]
mod ods_tests {
    use std::path::Path;

    use doc_converter_render::display_list::DL_MAGIC;

    use super::*;

    /// Фикстуры ODF из общего корпуса (`scripts/generate_odf_fixtures.ts`).
    fn fixture(name: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/odf/ods");
        std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("фикстура {name}: {error}"))
    }

    #[test]
    fn plugin_id_is_ods() {
        let info = OdsPlugin.info();
        assert_eq!(info.id, "ods");
        assert_eq!(info.kind, FormatKind::Sheets);
        assert_eq!(info.extensions, &["ods"]);
        assert_eq!(OdsPlugin.id(), info.id);
    }

    #[test]
    fn parse_gives_sheets_and_a_frame() {
        let model = OdsPlugin
            .parse(&fixture("simple.ods"), &ZipLimits::default())
            .expect("ODS разбирается");
        assert_eq!(model.page_count(), 2);

        let ctx = RenderContext {
            scale: 1.0,
            viewport_width: 800,
            viewport_height: 600,
        };
        let bytes = model.build_display_list(&ctx).expect("кадр собран");
        // DisplayList — единственный контракт кадра: магия заголовка на диске
        // little-endian, поэтому сравнивается u32, а не строка.
        let magic = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        assert_eq!(magic, DL_MAGIC, "кадр несёт магию DLST");
        assert!(
            DisplayList::from_bytes(&bytes).is_ok(),
            "кадр разбирается читателем DisplayList"
        );
    }
}

#[cfg(all(test, feature = "odp"))]
mod odp_tests {
    use std::path::Path;

    use doc_converter_render::display_list::DL_MAGIC;

    use super::*;

    /// Фикстуры ODF из общего корпуса (`scripts/generate_odf_fixtures.ts`).
    fn fixture(name: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/odf/odp");
        std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("фикстура {name}: {error}"))
    }

    #[test]
    fn plugin_id_is_odp() {
        let info = OdpPlugin.info();
        assert_eq!(info.id, "odp");
        assert_eq!(info.kind, FormatKind::Slides);
        assert_eq!(info.extensions, &["odp"]);
        assert_eq!(OdpPlugin.id(), info.id);
    }

    #[test]
    fn parse_gives_slides_and_a_frame() {
        let model = OdpPlugin
            .parse(&fixture("simple.odp"), &ZipLimits::default())
            .expect("ODP разбирается");
        assert_eq!(model.page_count(), 2);

        let ctx = RenderContext {
            scale: 1.0,
            viewport_width: 800,
            viewport_height: 600,
        };
        let bytes = model.build_display_list(&ctx).expect("кадр собран");
        let magic = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        assert_eq!(magic, DL_MAGIC, "кадр несёт магию DLST");
        assert!(
            DisplayList::from_bytes(&bytes).is_ok(),
            "кадр разбирается читателем DisplayList"
        );
    }
}
