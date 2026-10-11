//! Таблицы DOCX в PDF.
//!
//! Слайс фундамента: страницы, единицы, шрифт и абзацы. Таблицы рисует
//! следующий слайс Спринта 10 — здесь только шов, чтобы он правил один этот
//! файл, а не диспетчер (`docx::draw_items`). Пока таблица пропускается: текст
//! абзацев до и после неё на месте, а содержимое ячеек в PDF не попадает.

use doc_converter_docx::layout::TableCellLayout;
use doc_converter_render::font::FontRegistry;
use printpdf::Op;

use crate::fonts::EmbeddedFonts;
use crate::layout::PageGeometry;

/// Нарисовать таблицу: рамки ячеек и их содержимое.
///
/// Ячейки приходят в координатах страницы (`TableCellLayout::rect`), а их
/// содержимое — такими же элементами раскладки, что и тело документа, уже в
/// абсолютных координатах ([`doc_converter_docx::layout::LayoutItem`]): сдвигать
/// его не нужно. Рамки рисуются первыми и все сразу — иначе границы следующей
/// ячейки легли бы поверх текста предыдущей (так же устроен canvas-путь,
/// `doc_converter_docx::render::draw_table`).
pub(crate) fn draw(
    _ops: &mut Vec<Op>,
    _cells: &[TableCellLayout],
    _page: &PageGeometry,
    _embedded: &EmbeddedFonts,
    _registry: &mut FontRegistry,
) {
    // TODO (Спринт 10): рамки ячеек и их содержимое — отдельный слайс таблиц.
    // Начертания из ячеек уже собираются в общий набор (`docx::collect_faces`),
    // поэтому встраивать шрифты заново не придётся.
}
