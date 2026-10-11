//! Таблицы DOCX в PDF.
//!
//! Границы до раскладки не доезжают: [`doc_converter_docx::layout::LayoutItem::Table`]
//! несёт только прямоугольники ячеек, поэтому PDF повторяет решение canvas-пути
//! ([`doc_converter_docx::render::draw_table`]) — одинарная чёрная рамка в один
//! пиксель раскладки на каждую ячейку. Настоящие границы (`w:tcBorders`) —
//! правка `crates/docx`, а не экспорта.

use doc_converter_docx::layout::{Rect, TableCellLayout};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::FontRegistry;
use printpdf::{Line, LinePoint, Op, Point, Pt};

use crate::fonts::EmbeddedFonts;
use crate::layout::{PageGeometry, RectPx};
use crate::styles::to_pdf_color;

/// Толщина рамки ячейки в пикселях раскладки — как у canvas-пути
/// (`doc_converter_docx::render::CELL_BORDER_W`): границы ячеек в раскладку не
/// приходят, и обе стороны рисуют одинаковую одинарную рамку.
const CELL_BORDER_W_PX: f32 = 1.0;

/// Нарисовать таблицу: рамки ячеек и их содержимое.
///
/// Ячейки приходят в координатах страницы (`TableCellLayout::rect`), а их
/// содержимое — такими же элементами раскладки, что и тело документа, уже в
/// абсолютных координатах ([`doc_converter_docx::layout::LayoutItem`]): сдвигать
/// его не нужно. Рамки рисуются первыми и все сразу — иначе границы следующей
/// ячейки легли бы поверх текста предыдущей (так же устроен canvas-путь,
/// `doc_converter_docx::render::draw_table`).
pub(crate) fn draw(
    ops: &mut Vec<Op>,
    cells: &[TableCellLayout],
    page: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
) {
    for cell in cells {
        draw_cell_border(ops, &cell.rect, page);
    }
    for cell in cells {
        super::draw_items(&cell.content, ops, page, embedded, registry);
    }
}

/// Контур ячейки: четыре сплошные линии по её прямоугольнику.
///
/// Общее ребро соседних ячеек рисуется дважды — как и у canvas-пути. Цвет и
/// толщина берутся у canvas-пути (`CELL_BORDER_W` в пикселях раскладки), а не
/// из `w:tcBorders`: до экспорта они не доходят.
fn draw_cell_border(ops: &mut Vec<Op>, rect: &Rect, page: &PageGeometry) {
    // Начало координат printpdf — левый нижний угол, у раскладки — левый
    // верхний; `RectPt::to_pdf` уже вернул `y` нижней кромки.
    let pdf = page
        .rect_to_pt(RectPx::new(rect.x, rect.y, rect.width, rect.height))
        .to_pdf(page.height_pt());
    let (x0, y0) = (pdf.x.0, pdf.y.0);
    let (x1, y1) = (x0 + pdf.width.0, y0 + pdf.height.0);

    ops.push(Op::SetOutlineThickness {
        pt: Pt(page.px_to_pt(CELL_BORDER_W_PX)),
    });
    ops.push(Op::SetOutlineColor {
        col: to_pdf_color(Color::BLACK),
    });
    for (from, to) in [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x1, y1), (x0, y1)),
        ((x0, y1), (x0, y0)),
    ] {
        ops.push(Op::DrawLine {
            line: segment(from, to),
        });
    }
}

/// Отрезок по двум точкам.
fn segment(from: (f32, f32), to: (f32, f32)) -> Line {
    Line {
        points: vec![
            LinePoint {
                p: Point {
                    x: Pt(from.0),
                    y: Pt(from.1),
                },
                bezier: false,
            },
            LinePoint {
                p: Point {
                    x: Pt(to.0),
                    y: Pt(to.1),
                },
                bezier: false,
            },
        ],
        is_closed: false,
    }
}
