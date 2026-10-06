//! PDF-экспорт: XLSX-лист в страницы PDF на printpdf.
//!
//! Раскладка берётся у `doc-converter-xlsx` (`SheetLayout` в пикселях при
//! 96 dpi), метрики и перенос текста — у `doc-converter-render` (`FontRegistry`,
//! `text_measure`). Обе стороны общие с canvas-путём, поэтому точки разрыва
//! строк у экрана и в PDF совпадают по построению; граница зафиксирована в
//! ADR-0007, отдельного крейта `layout-core` нет.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

mod background;
mod border;
mod fonts;
mod layout;
mod options;
mod painter;
mod styles;
mod text;

pub use options::*;

use doc_converter_core::Result;

/// Ошибка PDF-экспорта.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PdfError {
    /// Лист с таким индексом в книге не найден.
    #[error("sheet index {0} is out of range")]
    NoSuchSheet(usize),
    /// Шрифт не разобран `printpdf`.
    #[error("invalid font data: {0}")]
    Font(String),
}

pub struct PdfExporter {
    opts: PdfOptions,
}

impl PdfExporter {
    #[must_use]
    pub fn new(opts: PdfOptions) -> Self {
        Self { opts }
    }

    /// # Errors
    /// TODO (Спринт 10): ошибки раскладки и записи PDF.
    pub fn export_docx(&mut self, _doc: &doc_converter_docx::Document) -> Result<Vec<u8>> {
        todo!("PDF export for DOCX (Фаза 4)")
    }

    /// Экспортировать лист книги (индекс в `wb.sheets()`) в PDF.
    ///
    /// Пока страница одна: содержимое за её границей отбрасывается, пагинация —
    /// следующий срез.
    ///
    /// # Errors
    /// [`doc_converter_core::Error::Malformed`], если листа с таким индексом нет
    /// или printpdf не смог собрать документ.
    pub fn export_xlsx_sheet(
        &mut self,
        wb: &doc_converter_xlsx::Workbook,
        sheet: usize,
    ) -> Result<Vec<u8>> {
        painter::export(wb, sheet, &self.opts)
            .map_err(|err| doc_converter_core::Error::Malformed(format!("pdf export: {err}")))
    }
}
