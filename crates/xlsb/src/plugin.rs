//! Плагин XLSB по общему контракту `render::plugin` (ADR-0030/0046).
//!
//! `XlsbPlugin` разбирает BIFF12-пакет в [`Workbook`] модели `xlsx` — той же,
//! что у XLSX. Поэтому раскладка, painter и PDF не знают, из какого формата
//! пришла книга: это и есть «новый формат без второго движка» (ADR-0046).
//!
//! Горячий путь wasm (`crates/xlsb-wasm`) в будущем держит книгу в
//! `thread_local!` и собирает кадры `xlsx::paint` напрямую; контракт нужен
//! тестам и общему `FormatPlugin`-пути.

use std::cell::Cell;

use doc_converter_core::ZipLimits;
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::plugin::{
    DocumentModel, EngineError, FormatKind, FormatPlugin, PluginInfo, RenderContext,
};
use doc_converter_render::viewport::Viewport;
use doc_converter_xlsx::paint::{self, PaintOptions};
use doc_converter_xlsx::Workbook;

/// Плагин XLSB: `info().id` → `"xlsb"`, как `DocFormat` протокола TS.
#[derive(Debug, Default)]
pub struct XlsbPlugin;

impl FormatPlugin for XlsbPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "xlsb",
            kind: FormatKind::Sheets,
            extensions: &["xlsb"],
        }
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не XLSB-пакет или запись BIFF12
    /// оборвана; [`EngineError::Unsupported`] — лист не-рабочего типа;
    /// [`EngineError::Limit`] — запись превысила лимит.
    fn parse(
        &self,
        bytes: &[u8],
        _limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        // `open` забирает буфер во владение: архив живёт до конца разбора.
        let book = crate::open(bytes.to_vec()).map_err(|error| match error {
            crate::error::XlsbError::Unsupported(message) => EngineError::Unsupported(message),
            crate::error::XlsbError::Limit(message) => EngineError::Limit(message),
            other => EngineError::Parse(other.to_string()),
        })?;
        Ok(Box::new(XlsbDocument { book }))
    }
}

/// Книга XLSB как `DocumentModel`: число единиц рендера — число листов,
/// первый кадр — первый лист от начала координат.
pub struct XlsbDocument {
    book: Workbook,
}

impl XlsbDocument {
    /// Модель книги — для тестов и будущего wasm-слоя.
    #[must_use]
    pub fn book(&self) -> &Workbook {
        &self.book
    }
}

impl DocumentModel for XlsbDocument {
    // Число листов в u32 влезает всегда (предел — пакеты по лимитам ADR-0015).
    #[allow(clippy::cast_possible_truncation)]
    fn page_count(&self) -> u32 {
        self.book.sheets().len() as u32
    }

    /// # Errors
    /// [`EngineError::Parse`], если в книге нет ни одного листа; кадр первого
    /// листа собирается тем же `xlsx::paint`, что у XLSX.
    #[allow(clippy::cast_precision_loss)]
    fn build_display_list(&self, ctx: &RenderContext) -> Result<Vec<u8>, EngineError> {
        let sheet = self
            .book
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
            &self.book,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_is_xlsb() {
        assert_eq!(XlsbPlugin.id(), "xlsb");
        assert_eq!(XlsbPlugin.info().kind, FormatKind::Sheets);
        assert_eq!(XlsbPlugin.info().extensions, &["xlsb"]);
    }
}
