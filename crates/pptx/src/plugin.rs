//! Плагин формата PPTX по общему контракту `render::plugin` (ADR-0030/0033).
//!
//! `PptxPlugin` разбирает презентацию и сразу считает раскладку слайдов —
//! как `pptx-wasm::pptx_open`: единица рендера (слайд) появляется при
//! открытии, и `page_count` не пересчитывает документ. wasm-клей
//! (`crates/pptx-wasm`) через trait-слой не ходит: он держит презентацию в
//! `thread_local!` и собирает конкретные кадры `pptx::render` напрямую —
//! контракт нужен тестам и будущим форматам, а не горячему пути.

use doc_converter_core::zip_limits::ZipLimits;
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::plugin::{DocumentModel, EngineError, FormatPlugin, RenderContext};
use doc_converter_render::viewport::Viewport;

use crate::layout::{layout_slide, SlideLayout};
use crate::model::Presentation;

/// Плагин PPTX: `id()` → `"pptx"`, как `DocFormat` протокола TS.
#[derive(Debug, Default)]
pub struct PptxPlugin;

impl FormatPlugin for PptxPlugin {
    fn id(&self) -> &'static str {
        "pptx"
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не OOXML-пакет PPTX, в нём нет
    /// слайдов или какая-то из частей не разбирается; [`EngineError::Layout`],
    /// если раскладка слайда не собралась.
    fn parse(
        &self,
        bytes: &[u8],
        limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        let presentation =
            crate::parse::parse(bytes, limits).map_err(|e| EngineError::Parse(e.to_string()))?;
        // Раскладка считается здесь и один раз: размер слайда задан
        // презентацией, а не окном, поэтому окно и зум приходят только с кадром.
        let layouts = (0..presentation.slide_count())
            .map(|index| layout_slide(&presentation, index))
            .collect::<crate::Result<Vec<_>>>()
            .map_err(|e| EngineError::Layout(e.to_string()))?;
        Ok(Box::new(PptxDocument {
            presentation,
            layouts,
        }))
    }
}

/// Презентация как `DocumentModel`: число единиц рендера — число слайдов,
/// первый кадр — первый слайд от начала координат.
pub struct PptxDocument {
    presentation: Presentation,
    layouts: Vec<SlideLayout>,
}

impl PptxDocument {
    /// Модель презентации — слайды, шейпы, media-части — в обход trait-слоя.
    #[must_use]
    pub fn presentation(&self) -> &Presentation {
        &self.presentation
    }

    /// Готовая раскладка слайда.
    #[must_use]
    pub fn layout(&self, index: usize) -> Option<&SlideLayout> {
        self.layouts.get(index)
    }
}

impl DocumentModel for PptxDocument {
    // Число слайдов в u32 влезает всегда (предел — пакеты по лимитам ADR-0015).
    #[allow(clippy::cast_possible_truncation)]
    fn page_count(&self) -> u32 {
        self.layouts.len() as u32
    }

    /// # Errors
    /// [`EngineError::Parse`], если в презентации нет ни одного слайда (для
    /// PPTX это битый пакет).
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
        crate::render::build(layout, &viewport, &mut out);
        Ok(out.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_is_pptx() {
        assert_eq!(PptxPlugin.id(), "pptx");
    }

    #[test]
    fn empty_bytes_are_a_parse_error() {
        let plugin = PptxPlugin;
        let result = plugin.parse(b"", &ZipLimits::default());
        assert!(matches!(result, Err(EngineError::Parse(_))));
    }
}
