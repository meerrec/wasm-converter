//! Плагин формата DOCX по общему контракту `render::plugin` (ADR-0030).
//!
//! `DocxPlugin` разбирает документ и сразу считает раскладку — как
//! `docx-wasm::docx_open`: единица рендера (страница) появляется при открытии,
//! и `page_count` не пересчитывает документ. wasm-клей (`crates/docx-wasm`)
//! через trait-слой не ходит: он держит `OpenDoc` в `thread_local!` и собирает
//! конкретные кадры `docx::render` напрямую — контракт нужен новым форматам и
//! тестам, а не горячему пути.

use doc_converter_core::ZipLimits;
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::font::FontRegistry;
use doc_converter_render::plugin::{DocumentModel, EngineError, FormatPlugin, RenderContext};
use doc_converter_render::viewport::Viewport;

use crate::layout::engine::{layout_document, LayoutOptions, PageLayout};
use crate::model::Document;

/// Размер LRU-кэша метрик шрифта — как у `docx-wasm` (`FONT_CACHE`): больше,
/// чем символов в документе, поэтому кэш не промахивается на повторах и не
/// растёт вместе с документом.
const FONT_CACHE: usize = 4096;

/// Плагин DOCX: `id()` → `"docx"`, как `DocFormat` протокола TS.
#[derive(Debug, Default)]
pub struct DocxPlugin;

impl FormatPlugin for DocxPlugin {
    fn id(&self) -> &'static str {
        "docx"
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не OOXML-пакет DOCX или
    /// `word/document.xml` не разбирается; [`EngineError::Layout`], если
    /// раскладка упирается в [`crate::LayoutError`].
    fn parse(
        &self,
        bytes: &[u8],
        limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        let document = crate::parse::parse_docx(bytes, limits.clone())
            .map_err(|e| EngineError::Parse(e.to_string()))?;
        // Раскладка считается здесь и один раз: ширина полосы набора задана
        // страницей, а не окном, поэтому окно и зум приходят только с кадром.
        // Шрифты — свежий реестр без зарегистрированных гарнитур: раскладка
        // использует fallback-метрики, как wasm-клей до `registerPaintFonts`.
        let mut fonts = FontRegistry::new(FONT_CACHE);
        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .map_err(|e| EngineError::Layout(e.to_string()))?;
        Ok(Box::new(DocxDocument { document, layout }))
    }
}

/// Документ как `DocumentModel`: число единиц рендера — число страниц
/// раскладки, первый кадр — первая страница от начала координат.
pub struct DocxDocument {
    document: Document,
    layout: PageLayout,
}

impl DocxDocument {
    /// Модель документа — абзацы, таблицы, медиачасти — в обход trait-слоя.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }
}

impl DocumentModel for DocxDocument {
    // Ширина/высота canvas — тысячи пикселей, f32 держит их точно; число
    // страниц в u32 влезает всегда (предел — пакеты по лимитам ADR-0015).
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn page_count(&self) -> u32 {
        self.layout.pages.len() as u32
    }

    /// # Errors
    /// Пока — никогда: раскладка уже посчитана при [`FormatPlugin::parse`],
    /// вариант зарезервирован на будущие пути пересборки.
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
        crate::render::build(&self.layout, 0, viewport, &mut out);
        Ok(out.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Документы из общего корпуса фикстур (`scripts/generate_docx_fixtures.ts`):
    /// тот же каталог используют интеграционные тесты крейта.
    fn fixture(name: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx");
        std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("фикстура {name}: {e}"))
    }

    #[test]
    fn segmented_parse_matches_bytes() {
        // Путь стрима (ADR-0031) обязан давать ту же модель и ту же раскладку,
        // что путь байтов: фикстура режется на чанки по 7 байт.
        let bytes = fixture("simple/one_paragraph.docx");
        let limits = ZipLimits::default();
        let via_bytes = crate::parse::parse_docx(&bytes, limits.clone()).unwrap();

        let mut buf = doc_converter_core::SegmentedBuffer::new();
        for chunk in bytes.chunks(7) {
            buf.append(chunk);
        }
        let via_stream = crate::parse::parse_docx_segmented(buf, limits).unwrap();

        assert_eq!(
            via_bytes.warnings().len(),
            via_stream.warnings().len(),
            "тот же набор предупреждений"
        );
        let pages_of = |document: &crate::Document| {
            let mut fonts = FontRegistry::new(FONT_CACHE);
            layout_document(document, &LayoutOptions::default(), &mut fonts)
                .unwrap()
                .pages
                .len()
        };
        assert_eq!(
            pages_of(&via_bytes),
            pages_of(&via_stream),
            "та же раскладка"
        );
    }

    #[test]
    fn plugin_id_is_docx() {
        assert_eq!(DocxPlugin.id(), "docx");
    }

    #[test]
    fn parse_gives_model_with_pages_and_frame() {
        let model = DocxPlugin
            .parse(&fixture("simple/one_paragraph.docx"), &ZipLimits::default())
            .unwrap();
        assert!(
            model.page_count() >= 1,
            "у документа есть хотя бы одна страница"
        );

        let ctx = RenderContext {
            scale: 1.0,
            viewport_width: 800,
            viewport_height: 600,
        };
        let bytes = model.build_display_list(&ctx).unwrap();
        // DisplayList — единственный контракт кадра: кадр обязан нести магию
        // заголовка (на диске — little-endian, поэтому сравнение через u32,
        // а не по строке) и разбираться читателем кадра.
        let magic = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        assert_eq!(
            magic,
            doc_converter_render::display_list::DL_MAGIC,
            "кадр несёт магию DLST"
        );
        assert!(
            doc_converter_render::DisplayList::from_bytes(&bytes).is_ok(),
            "кадр разбирается читателем DisplayList"
        );
    }
}
