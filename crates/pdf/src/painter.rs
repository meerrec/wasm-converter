//! Сборка страницы PDF: обход ячеек листа и запись операций printpdf.
//!
//! Порядок слоёв повторяет оба «экранных» пути (Excel и `xlsx::paint`):
//! сначала заливки, поверх них рамки, поверх всего текст. Поэтому обход листа
//! идёт тремя проходами по заранее собранному списку ячеек, а не одним.
//!
//! Страница пока одна. Место пагинации — [`crate::layout`] и этот модуль:
//! список [`PaintedCell`] уже отделён от записи операций, разбить его на
//! страницы можно будет без переписывания обхода.
//!
//! Ещё не перенесены настройки [`PdfOptions`], за которыми стоит заметная
//! работа: сетка (`print_grid_lines`), `fit_to_width`, повтор заголовков,
//! закреплённые области, картинки и диаграммы — следующие срезы спринта.

use doc_converter_render::font::FontRegistry;
use doc_converter_xlsx::layout::{SheetLayout, PX_PER_POINT};
use doc_converter_xlsx::paint::display_text;
use doc_converter_xlsx::{Cell, CellRef, CellValue, Sheet, Workbook};
use printpdf::{Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg, Pt};

use crate::layout::{PageGeometry, RectPx};
use crate::options::PdfOptions;
use crate::styles::{self, CellStyle};
use crate::{background, border, fonts, text, PdfError};

/// Размер LRU-кэша метрик: столько же, сколько у canvas-пути.
const FONT_CACHE: usize = 4096;

/// Ячейка, готовая к отрисовке: ссылка на модель, её прямоугольник и стиль.
struct PaintedCell<'a> {
    cell: &'a Cell,
    rect: RectPx,
    style: CellStyle,
}

/// Открыть ячейку листа и собрать PDF одной страницей.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если индекса нет в книге; [`PdfError::Font`],
/// если printpdf не принял байты шрифта.
pub fn export(
    book: &Workbook,
    sheet_index: usize,
    options: &PdfOptions,
) -> Result<Vec<u8>, PdfError> {
    let sheet = book
        .sheets()
        .get(sheet_index)
        .ok_or(PdfError::NoSuchSheet(sheet_index))?;
    let layout = SheetLayout::new(sheet);
    let page = PageGeometry::new(&options.page);

    let title = if options.title.is_empty() {
        sheet.meta.name.clone()
    } else {
        options.title.clone()
    };
    let mut doc = PdfDocument::new(&title);
    doc.metadata.info.author.clone_from(&options.author);
    doc.metadata.info.subject.clone_from(&options.subject);
    doc.metadata.info.keywords.clone_from(&options.keywords);

    let mut warnings: Vec<PdfWarnMsg> = Vec::new();
    let font = fonts::embed_default(&mut doc, &mut warnings)?;
    let mut registry = FontRegistry::new(FONT_CACHE);

    let mut ops: Vec<Op> = Vec::new();
    draw_sheet(
        &mut ops,
        &mut registry,
        font.id(),
        &page,
        book,
        sheet,
        &layout,
    );

    doc.with_pages(vec![PdfPage::new(
        Mm::from(Pt(page.width_pt())),
        Mm::from(Pt(page.height_pt())),
        ops,
    )]);
    let save = PdfSaveOptions {
        // `optimize` у printpdf сжимает потоки — это и есть `compress`.
        optimize: options.compress,
        ..PdfSaveOptions::default()
    };
    Ok(doc.save(&save, &mut warnings))
}

/// Записать лист в операции страницы.
fn draw_sheet(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    pdf_font: &printpdf::FontId,
    page: &PageGeometry,
    book: &Workbook,
    sheet: &Sheet,
    layout: &SheetLayout,
) {
    let cells = collect_cells(book, sheet, layout, page);
    for painted in &cells {
        if let Some(color) = painted.style.fill {
            background::fill_rect(ops, page.rect_to_pt(painted.rect), page.height_pt(), color);
        }
    }
    for painted in &cells {
        if border::is_visible(&painted.style.border) {
            border::draw_border(
                ops,
                page.rect_to_pt(painted.rect),
                page.height_pt(),
                &painted.style.border,
                book.theme(),
            );
        }
    }
    for painted in &cells {
        let cell = painted.cell;
        let Some(mut value) = display_text(book, cell, painted.style.number_format) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if matches!(cell.value, CellValue::Number(_)) {
            let size_px = painted.style.font_size_pt * PX_PER_POINT * page.scale();
            value = text::clip_number(
                value,
                size_px,
                text::inner_width_px(page, painted.rect),
                registry,
            );
        }
        text::draw_cell_text(
            ops,
            registry,
            pdf_font,
            page,
            &value,
            &painted.style,
            painted.rect,
        );
    }
}

/// Собрать ячейки листа, попадающие на страницу.
///
/// Ячейка объединения рисуется один раз — на своей левой верхней: остальные
/// пусты и отдельного текста не несут.
fn collect_cells<'a>(
    book: &Workbook,
    sheet: &'a Sheet,
    layout: &SheetLayout,
    page: &PageGeometry,
) -> Vec<PaintedCell<'a>> {
    let (max_x, max_y) = page.content_px();
    let mut cells = Vec::new();
    for (row, row_cells) in sheet.cells.rows() {
        for cell in row_cells {
            let Some(rect) = cell_rect(sheet, layout, cell.at(row)) else {
                continue;
            };
            if rect.w <= 0.0 || rect.h <= 0.0 {
                continue;
            }
            // За границей первой страницы: разбивка на страницы — следующий срез.
            if rect.x >= max_x || rect.y >= max_y {
                continue;
            }
            cells.push(PaintedCell {
                cell,
                rect,
                style: styles::resolve(book, cell),
            });
        }
    }
    cells
}

/// Прямоугольник ячейки в пикселях раскладки; `None` — ячейку закрывает
/// объединение, её левая верхняя рисуется отдельно.
fn cell_rect(sheet: &Sheet, layout: &SheetLayout, at: CellRef) -> Option<RectPx> {
    let (first, last) = match sheet.merges.covering(at) {
        Some(range) if range.first != at => return None,
        Some(range) => (range.first, range.last),
        None => (at, at),
    };
    let x = layout.column_x(first.col);
    let y = layout.row_y(first.row);
    Some(RectPx::new(
        x,
        y,
        layout.column_x(last.col + 1) - x,
        layout.row_y(last.row + 1) - y,
    ))
}
