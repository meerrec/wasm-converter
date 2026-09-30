//! Ручной PDF-экспорт (printpdf + ttf-parser + subsetting).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

mod options;

pub use options::*;

use doc_converter_core::Result;

pub struct PdfExporter { opts: PdfOptions }

impl PdfExporter {
    pub fn new(opts: PdfOptions) -> Self { Self { opts } }

    pub fn export_docx(&mut self, _doc: &doc_converter_docx::Document) -> Result<Vec<u8>> {
        todo!("PDF export for DOCX (Фаза 4)")
    }

    pub fn export_xlsx_sheet(
        &mut self,
        _wb: &doc_converter_xlsx::Workbook,
        _sheet: usize,
    ) -> Result<Vec<u8>> {
        todo!("PDF export for XLSX sheet (Фаза 4)")
    }
}
