//! Плавающие рисунки DOCX (`wp:anchor`) в PDF.
//!
//! Слайс фундамента: страницы, единицы, шрифт и абзацы. Плавающие элементы
//! рисует следующий слайс Спринта 10 — здесь только шов, чтобы он правил один
//! этот файл, а не диспетчер (`docx::draw_items`). Пока элемент пропускается:
//! абзацы на месте, а место рисунка в PDF пустое.

use doc_converter_docx::layout::FloatElement;
use doc_converter_render::font::FontRegistry;
use printpdf::Op;

use crate::fonts::EmbeddedFonts;
use crate::layout::PageGeometry;

/// Нарисовать плавающий элемент в его прямоугольнике (`FloatElement::rect`).
///
/// Прямоугольник анкера лежит в координатах страницы — той же, что у остальных
/// элементов раскладки; сдвигать его не нужно.
pub(crate) fn draw(
    _ops: &mut Vec<Op>,
    _float: &FloatElement,
    _page: &PageGeometry,
    _embedded: &EmbeddedFonts,
    _registry: &mut FontRegistry,
) {
    // TODO (Спринт 10): рисунок, обтекание и выемка медиа из архива — отдельный
    // слайс плавающих элементов; canvas-путь пока тоже рисует только габариты
    // (`doc_converter_docx::render::draw_float_placeholder`).
}
