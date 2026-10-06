//! Сборка страниц PDF: обход ячеек листа и запись операций printpdf.
//!
//! Порядок слоёв повторяет оба «экранных» пути (Excel и `xlsx::paint`):
//! сначала заливки, поверх них рамки, поверх всего текст. Поэтому обход листа
//! идёт тремя проходами по заранее собранному списку ячеек, а не одним.
//!
//! Лист режется на страницы заранее ([`crate::pagination`]): сюда приходит
//! срез страницы и её ячейки, отсюда — только геометрия и операции.
//!
//! Ещё не перенесены настройки [`PdfOptions`], за которыми стоит заметная
//! работа: сетка (`print_grid_lines`), закреплённые области, картинки и
//! диаграммы — следующие срезы спринта.

use doc_converter_render::font::FontRegistry;
use doc_converter_xlsx::layout::{SheetLayout, PX_PER_POINT};
use doc_converter_xlsx::paint::display_text;
use doc_converter_xlsx::{CellValue, Workbook};
use printpdf::{Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg, Pt};

use crate::fonts::{EmbeddedFonts, Face};
use crate::layout::{PageGeometry, RectPx};
use crate::options::PdfOptions;
use crate::pagination::{paginate, print_scale, PageSlice, PaintedCell, SheetPage};
use crate::{background, border, fonts, text, PdfError};

/// Размер LRU-кэша метрик: столько же, сколько у canvas-пути.
const FONT_CACHE: usize = 4096;

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
    // `fit_to_width` пересчитывает масштаб до сборки геометрии: от него зависят
    // и полосы, и координаты ячеек.
    let mut page_cfg = options.page.clone();
    page_cfg.scale = print_scale(&page_cfg, sheet, &layout);
    let page = PageGeometry::new(&page_cfg);

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
    // `page_cfg` несёт и масштаб, и повтор шапки/первых столбцов: пагинация
    // читает их одним конфигом, а геометрия — уже с итоговым масштабом.
    let pagination = paginate(book, sheet, &layout, &page, &page_cfg);
    let fonts = fonts::embed(&mut doc, &mut warnings, pagination.faces)?;
    let mut registry = FontRegistry::new(FONT_CACHE);

    for sheet_page in &pagination.pages {
        let page_geom = page.with_page_top_px(sheet_page.slice.offset_y);
        let mut ops: Vec<Op> = Vec::new();
        draw_page(
            &mut ops,
            &mut registry,
            &fonts,
            &page_geom,
            book,
            sheet_page,
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

/// Прямоугольник ячейки в координатах страницы: из координат листа вычитается
/// сдвиг среза ([`PageSlice::offset_x`] — по горизонтали, `offset_y` делает
/// геометрия) и прибавляется сдвиг центрирования, посчитанный пагинацией.
///
/// Сдвиг центрирования получают все ячейки страницы, и потока, и повторяемые
/// части: их прямоугольники уже приведены к началу координат страницы, поэтому
/// общий сдвиг двигает полосу набора целиком.
fn page_rect(rect: RectPx, slice: &PageSlice) -> RectPx {
    RectPx::new(
        rect.x - slice.offset_x + slice.center_x_px,
        rect.y + slice.center_y_px,
        rect.w,
        rect.h,
    )
}

/// Записать ячейки страницы в её операции.
///
/// Начертание выбирается по стилю ячейки, но лишь для записи текста: кегль и
/// перенос считает regular, как canvas-путь.
///
/// Прямоугольники ячеек приходят в координатах листа, поэтому сдвиг полосы
/// столбцов вычитается здесь: у [`PageGeometry`] сдвига по горизонтали нет, а по
/// вертикали его делает `with_page_top_px`.
fn draw_page(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    fonts: &EmbeddedFonts,
    page: &PageGeometry,
    book: &Workbook,
    sheet_page: &SheetPage<'_>,
) {
    let on_page = |painted: &PaintedCell<'_>| page_rect(painted.rect, &sheet_page.slice);
    for painted in &sheet_page.cells {
        if let Some(color) = painted.style.fill {
            background::fill_rect(
                ops,
                page.rect_to_pt(on_page(painted)),
                page.height_pt(),
                color,
            );
        }
    }
    for painted in &sheet_page.cells {
        if border::is_visible(&painted.style.border) {
            border::draw_border(
                ops,
                page.rect_to_pt(on_page(painted)),
                page.height_pt(),
                &painted.style.border,
                book.theme(),
            );
        }
    }
    for painted in &sheet_page.cells {
        let cell = painted.cell;
        let Some(mut value) = display_text(book, cell, painted.style.number_format) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let rect = on_page(painted);
        if matches!(cell.value, CellValue::Number(_)) {
            let size_px = painted.style.font_size_pt * PX_PER_POINT * page.scale();
            value = text::clip_number(value, size_px, text::inner_width_px(page, rect), registry);
        }
        text::draw_cell_text(
            ops,
            registry,
            fonts.id(Face::of(painted.style.bold, painted.style.italic)),
            page,
            &value,
            &painted.style,
            rect,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PageConfig;

    /// Срез страницы без полос и повторов: важны только сдвиги.
    fn plain_slice() -> PageSlice {
        PageSlice {
            rows: 0..0,
            cols: 0..0,
            header_rows: 0..0,
            repeat_cols: 0..0,
            offset_y: 0.0,
            offset_x: 0.0,
            center_x_px: 0.0,
            center_y_px: 0.0,
        }
    }

    /// Центрирование сдвигает прямоугольник в точках: 96 px раскладки при 100%
    /// — это 72 pt, значит половинный сдвиг в 48 px — 36 pt.
    #[test]
    fn centering_shifts_cell_rect_in_points() {
        let page = PageGeometry::new(&PageConfig::default());
        let rect = RectPx::new(0.0, 0.0, 96.0, 96.0);
        let plain = page.rect_to_pt(page_rect(rect, &plain_slice()));

        let mut slice = plain_slice();
        slice.center_x_px = 48.0;
        slice.center_y_px = 48.0;
        let centered = page.rect_to_pt(page_rect(rect, &slice));

        let dx = centered.x - plain.x;
        let dy = centered.y - plain.y;
        assert!((dx - 36.0).abs() < 1e-3, "сдвиг по x: {dx} pt");
        assert!((dy - 36.0).abs() < 1e-3, "сдвиг по y: {dy} pt");
    }

    /// Срез вычитается до центрирования: сдвиг возвращает ячейку к левому краю
    /// полосы, и лишь потом она уезжает к середине страницы.
    #[test]
    fn slice_offset_is_subtracted_before_centering() {
        let mut slice = plain_slice();
        slice.offset_x = 40.0;
        slice.center_x_px = 25.0;
        slice.center_y_px = 5.0;

        let shifted = page_rect(RectPx::new(100.0, 200.0, 30.0, 10.0), &slice);
        assert_eq!(shifted, RectPx::new(85.0, 205.0, 30.0, 10.0));
    }
}
