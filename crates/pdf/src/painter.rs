//! Сборка страниц PDF: обход ячеек листа и запись операций printpdf.
//!
//! Порядок слоёв повторяет оба «экранных» пути (Excel и `xlsx::paint`):
//! сначала заливки, поверх них рамки, поверх всего текст. Поэтому обход листа
//! идёт тремя проходами по заранее собранному списку ячеек, а не одним.
//!
//! Лист длиннее страницы печатается несколькими страницами: строки делятся на
//! страницы до отрисовки ([`paginate`]), каждая страница получает свою
//! геометрию со сдвигом по вертикали. Разбивка по столбцам — Спринт 7.
//!
//! Ещё не перенесены настройки [`PdfOptions`], за которыми стоит заметная
//! работа: сетка (`print_grid_lines`), `fit_to_width`, повтор заголовков,
//! закреплённые области, картинки и диаграммы — следующие срезы спринта.

use doc_converter_render::font::FontRegistry;
use doc_converter_xlsx::layout::{SheetLayout, PX_PER_POINT};
use doc_converter_xlsx::paint::display_text;
use doc_converter_xlsx::{Cell, CellRef, CellValue, Sheet, Workbook};
use printpdf::{Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg, Pt};

use crate::fonts::{EmbeddedFonts, Face, FaceSet};
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

/// Ячейки одной страницы и её верх в пикселях раскладки.
struct SheetPage<'a> {
    /// Верх страницы в координатах листа; вычитается при записи операций.
    top_px: f32,
    cells: Vec<PaintedCell<'a>>,
}

/// Лист, разбитый на страницы, и начертания, которые на них встретились.
struct SheetPagination<'a> {
    faces: FaceSet,
    pages: Vec<SheetPage<'a>>,
}

/// Открыть лист книги и собрать PDF всеми его страницами.
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
    let pagination = paginate(book, sheet, &layout, &page);
    let fonts = fonts::embed(&mut doc, &mut warnings, pagination.faces)?;
    let mut registry = FontRegistry::new(FONT_CACHE);

    for sheet_page in &pagination.pages {
        let page_geom = page.with_page_top_px(sheet_page.top_px);
        let mut ops: Vec<Op> = Vec::new();
        draw_page(
            &mut ops,
            &mut registry,
            &fonts,
            &page_geom,
            book,
            &sheet_page.cells,
        );
        doc.with_pages(vec![PdfPage::new(
            Mm::from(Pt(page_geom.width_pt())),
            Mm::from(Pt(page_geom.height_pt())),
            ops,
        )]);
    }

    let save = PdfSaveOptions {
        // `optimize` у printpdf сжимает потоки — это и есть `compress`.
        optimize: options.compress,
        ..PdfSaveOptions::default()
    };
    Ok(doc.save(&save, &mut warnings))
}

/// Записать ячейки страницы в её операции.
///
/// Начертание выбирается по стилю ячейки, но лишь для записи текста: кегль и
/// перенос считает regular, как canvas-путь.
fn draw_page(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    fonts: &EmbeddedFonts,
    page: &PageGeometry,
    book: &Workbook,
    cells: &[PaintedCell<'_>],
) {
    for painted in cells {
        if let Some(color) = painted.style.fill {
            background::fill_rect(ops, page.rect_to_pt(painted.rect), page.height_pt(), color);
        }
    }
    for painted in cells {
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
    for painted in cells {
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
            fonts.id(Face::of(painted.style.bold, painted.style.italic)),
            page,
            &value,
            &painted.style,
            painted.rect,
        );
    }
}

/// Разбить ячейки листа на страницы по строкам и собрать нужные начертания.
///
/// Строка не разрывается между страницами: как только её низ выходит за
/// границу области содержимого, начинается новая страница — с этой строки.
/// Высоты строк берутся из раскладки абсолютными, поэтому пустые строки
/// занимают на странице своё место. Страницы без ячеек не создаются: первой
/// странице задаёт верх первая строка с ячейкой, остальным — строка, на
/// которой случился разрыв. Пустой лист — одна пустая страница.
///
/// Ячейка объединения рисуется один раз — на своей левой верхней: остальные
/// пусты и отдельного текста не несут.
fn paginate<'a>(
    book: &Workbook,
    sheet: &'a Sheet,
    layout: &SheetLayout,
    page: &PageGeometry,
) -> SheetPagination<'a> {
    let (max_x, max_h_px) = page.content_px();
    let mut faces = FaceSet::default();
    let mut pages: Vec<SheetPage<'a>> = Vec::new();
    for (row, row_cells) in sheet.cells.rows() {
        let top_px = layout.row_y(row);
        let bottom_px = layout.row_y(row + 1);
        let breaks = match pages.last() {
            None => true,
            Some(current) => bottom_px > current.top_px + max_h_px,
        };
        if breaks {
            pages.push(SheetPage {
                top_px,
                cells: Vec::new(),
            });
        }
        let Some(current) = pages.last_mut() else {
            continue;
        };
        for cell in row_cells {
            let Some(rect) = cell_rect(sheet, layout, cell.at(row)) else {
                continue;
            };
            if rect.w <= 0.0 || rect.h <= 0.0 {
                continue;
            }
            // Правее области содержимого: разбивка по столбцам — Спринт 7.
            if rect.x >= max_x {
                continue;
            }
            let style = styles::resolve(book, cell);
            faces.insert(Face::of(style.bold, style.italic));
            current.cells.push(PaintedCell { cell, rect, style });
        }
    }
    if pages.is_empty() {
        pages.push(SheetPage {
            top_px: 0.0,
            cells: Vec::new(),
        });
    }
    SheetPagination { faces, pages }
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
