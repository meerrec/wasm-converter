//! PDF-экспорт: XLSX-лист в страницы PDF на printpdf.
//!
//! Раскладка берётся у `doc-converter-xlsx` (`SheetLayout` в пикселях при
//! 96 dpi), метрики и перенос текста — у `doc-converter-render` (`FontRegistry`,
//! `text_measure`). Обе стороны общие с canvas-путём, поэтому точки разрыва
//! строк у экрана и в PDF совпадают по построению; граница зафиксирована в
//! ADR-0007, отдельного крейта `layout-core` нет.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

mod annot;
mod background;
mod border;
pub mod chart;
mod fonts;
pub mod image;
mod layout;
mod options;
pub mod overlay;
mod pagination;
mod painter;
mod styles;
mod text;

pub use options::*;

use std::io::Write;

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
    /// Приёмник PDF вернул ошибку записи.
    #[error("write failed: {0}")]
    Io(#[from] std::io::Error),
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
    /// Лист печатается столько́м страниц, на сколько делят его строки: строка
    /// целиком, страницы без содержимого не создаются. Разбивка по столбцам —
    /// Спринт 7.
    ///
    /// # Errors
    /// [`doc_converter_core::Error::Export`], если листа с таким индексом нет
    /// или printpdf не смог собрать документ.
    pub fn export_xlsx_sheet(
        &mut self,
        wb: &doc_converter_xlsx::Workbook,
        sheet: usize,
    ) -> Result<Vec<u8>> {
        painter::export(wb, sheet, &self.opts)
            .map_err(|err| doc_converter_core::Error::Export(format!("pdf: {err}")))
    }

    /// Экспортировать лист книги в приёмник байтов: файл, `Vec<u8>`, чанковый
    /// поток.
    ///
    /// Форма — writer ([`Write`]), а не колбэк или чанки: стриминговый
    /// сериализатор форка (`printpdf::StreamSession`) уже обобщён по `Write`,
    /// приёмнику не нужен seek (xref пишется одной секцией в конце), а колбэк
    /// и чанки выражаются адаптером над `write`, не наоборот.
    ///
    /// Страницы уходят в приёмник по мере сборки, и содержимое страницы
    /// освобождается сразу после записи: сам писатель держит на страницу
    /// только идентификаторы и xref. Это не значит, что память экспорта
    /// постоянна, — замер (`docs/sprint-7/streaming-report.md`) показывает
    /// рабочий набор `≈15,6 КиБ` на страницу сверх разобранной книги, и растёт
    /// он не в писателе, а в модуле `pagination`: `paginate` строит срезы
    /// всех страниц заранее. Пик приёмника — его собственное дело (файл на
    /// диске не буферизуется, `Vec<u8>` — буферизуется целиком). Отличие от
    /// [`PdfExporter::export_xlsx_sheet`] — только в этом.
    ///
    /// # Errors
    /// [`doc_converter_core::Error::Export`], если листа с таким индексом нет,
    /// printpdf не смог собрать документ или приёмник вернул ошибку записи.
    pub fn export_xlsx_sheet_to<W: Write>(
        &mut self,
        wb: &doc_converter_xlsx::Workbook,
        sheet: usize,
        out: &mut W,
    ) -> Result<()> {
        painter::export_to(wb, sheet, &self.opts, out)
            .map_err(|err| doc_converter_core::Error::Export(format!("pdf: {err}")))
    }
}
