//! Плагин формата XLSX по общему контракту `render::plugin` (ADR-0030).
//!
//! `XlsxPlugin` разбирает книгу в `XlsxDocument` — обёртку [`Workbook`] над
//! trait-объектом `DocumentModel`. Единица рендера — лист. wasm-клей
//! (`crates/wasm::xlsx_api`) через trait-слой не ходит: он держит конкретную
//! книгу в `thread_local!` и собирает конкретные кадры `paint::build`
//! напрямую — контракт нужен новым форматам и тестам, а не горячему пути.

use std::cell::Cell;

use doc_converter_core::ZipLimits;
use doc_converter_render::display_list::DisplayList;
use doc_converter_render::plugin::{DocumentModel, EngineError, FormatPlugin, RenderContext};
use doc_converter_render::viewport::Viewport;

use crate::model::Workbook;
use crate::paint::{self, PaintOptions};

/// Плагин XLSX: `id()` → `"xlsx"`, как `DocFormat` протокола TS.
#[derive(Debug, Default)]
pub struct XlsxPlugin;

impl FormatPlugin for XlsxPlugin {
    fn id(&self) -> &'static str {
        "xlsx"
    }

    /// # Errors
    /// [`EngineError::Parse`], если байты не OOXML-пакет XLSX или какая-то из
    /// частей не разбирается.
    fn parse(
        &self,
        bytes: &[u8],
        _limits: &ZipLimits,
    ) -> Result<Box<dyn DocumentModel>, EngineError> {
        // `crate::open` забирает буфер во владение (архив живёт до конца
        // разбора), поэтому на границе трейта байты копируются. Лимиты не
        // принимаются: `xlsx::open` применяет дефолтные (ADR-0015).
        let book = crate::open(bytes.to_vec()).map_err(|e| EngineError::Parse(e.to_string()))?;
        Ok(Box::new(XlsxDocument { book }))
    }
}

/// Книга как `DocumentModel`: число единиц рендера — число листов, первый
/// кадр — первый лист от начала координат.
pub struct XlsxDocument {
    book: Workbook,
}

impl XlsxDocument {
    /// Модель книги — доступ к листам, ячейкам и стилям в обход trait-слоя.
    #[must_use]
    pub fn book(&self) -> &Workbook {
        &self.book
    }
}

impl DocumentModel for XlsxDocument {
    // Число листов в u32 влезает всегда (предел — пакеты по лимитам ADR-0015).
    #[allow(clippy::cast_possible_truncation)]
    fn page_count(&self) -> u32 {
        self.book.sheets().len() as u32
    }

    /// # Errors
    /// [`EngineError::Parse`], если в книге нет ни одного листа (для OOXML это
    /// битый пакет: `workbook.xml` обязан нести хотя бы один лист).
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
        .map_err(|e| EngineError::Layout(e.to_string()))?;
        Ok(out.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Книги из общего корпуса фикстур (`scripts/gen-fixtures.ts`): детерминированные
    /// пакеты, тот же каталог используют интеграционные тесты крейта.
    fn fixture(name: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/xlsx");
        std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("фикстура {name}: {e}"))
    }

    #[test]
    fn segmented_open_matches_bytes() {
        // Путь стрима (ADR-0031) обязан разбирать ту же книгу, что путь байтов:
        // фикстура режется на чанки по 7 байт и отдаётся `open_segmented`.
        let bytes = fixture("cf-cell-is.xlsx");
        let via_bytes = crate::open(bytes.clone()).unwrap();
        let mut buf = doc_converter_core::SegmentedBuffer::new();
        for chunk in bytes.chunks(7) {
            buf.append(chunk);
        }
        let via_stream = crate::open_segmented(buf).unwrap();
        assert_eq!(via_bytes, via_stream, "та же книга из чанков");
    }

    #[test]
    fn plugin_id_is_xlsx() {
        assert_eq!(XlsxPlugin.id(), "xlsx");
    }

    #[test]
    fn parse_gives_model_with_sheets_and_frame() {
        let model = XlsxPlugin
            .parse(&fixture("cf-cell-is.xlsx"), &ZipLimits::default())
            .unwrap();
        assert!(model.page_count() >= 1, "у книги есть хотя бы один лист");

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
