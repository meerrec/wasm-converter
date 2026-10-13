//! Плавающие элементы DOCX (`wp:anchor`) в PDF.
//!
//! Плавающий рисунок рисуется тем же путём, что встроенный (`super::images`):
//! прямоугольник берётся у анкера, байты — у медиачасти (`FloatKind::Image`).
//! Если байтов нет (`image_id: None` — внешняя цель, битая часть, часть выше
//! предела), на месте рисунка остаётся плейсхолдер — контур прямоугольника
//! анкера, ровно как у canvas-пути (`doc_converter_docx::render::draw_image`):
//! видны габариты, и раскладка с обтеканием проверяема на глаз.
//!
//! `behind_text` (`behindDoc`) учитывается порядком операций страницы: элемент
//! под текстом встаёт в начало, остальные дописываются в конец. Раскладка
//! кладёт `LayoutItem::Float` после строк абзаца-анкера, поэтому без переноса
//! вперёд рисунок «под текстом» лёг бы поверх него.

use doc_converter_docx::layout::{FloatElement, FloatKind, Rect};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::FontRegistry;

use crate::fonts::EmbeddedFonts;
use crate::layout::PageGeometry;
use crate::pdf::ops::{LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, WindingOrder};
use crate::styles::to_pdf_color;

use super::images::Images;

/// Цвет контура плейсхолдера — тот же серый, что у canvas-пути
/// (`render::PLACEHOLDER_STROKE` в `doc-converter-docx`).
const PLACEHOLDER_STROKE: Color = Color::rgba(0x80, 0x80, 0x80, 0xFF);

/// Толщина контура плейсхолдера в пикселях раскладки — как у canvas-пути
/// (`render::PLACEHOLDER_STROKE_W`).
const PLACEHOLDER_STROKE_PX: f32 = 1.0;

/// Нарисовать плавающий элемент в его прямоугольнике (`FloatElement::rect`).
///
/// Прямоугольник анкера лежит в координатах страницы — той же, что у остальных
/// элементов раскладки; сдвигать его не нужно. Рисунок занимает этот
/// прямоугольник целиком: `wp:extent` раскладка уже учла, а `a:stretch` тянет
/// картинку на якорь, не сохраняя пропорций, — так же её кладёт canvas-путь.
/// `FloatKind::TextBox` раскладка пока не порождает, и видно только габариты.
///
/// `offset_y` — сдвиг в пикселях раскладки: он не ноль, когда элемент — часть
/// содержимого ячейки, выровненного по `w:vAlign`.
pub(crate) fn draw(
    ops: &mut Vec<Op>,
    float: &FloatElement,
    page: &PageGeometry,
    _embedded: &EmbeddedFonts,
    _registry: &mut FontRegistry,
    images: &Images,
    offset_y: f32,
) {
    let drawn = match &float.kind {
        FloatKind::Image { image_id, .. } => images.ops(*image_id, float.rect, offset_y, page),
        FloatKind::TextBox { .. } => Vec::new(),
    };
    // Пустой список операций означает «рисовать нечем»: нет байтов медиачасти,
    // они не декодировались или прямоугольник вырожден.
    let drawn = if drawn.is_empty() {
        placeholder_ops(shifted(float.rect, offset_y), page)
    } else {
        drawn
    };
    if float.behind_text {
        // Два элемента под текстом на одной странице не перекрываются —
        // раскладка разводит их по вертикали (`float::blocking_bottom`), —
        // поэтому порядок между ними не наблюдаем и обратный порядок вставки
        // безопасен.
        ops.splice(0..0, drawn);
    } else {
        ops.extend(drawn);
    }
}

/// Прямоугольник анкера, сдвинутый содержимым ячейки по вертикали.
fn shifted(rect: Rect, offset_y: f32) -> Rect {
    Rect::new(rect.x, rect.y + offset_y, rect.width, rect.height)
}

/// Операции контура плейсхолдера по прямоугольнику анкера.
///
/// Начало координат PDF — левый нижний угол страницы, а `rect` отсчитан
/// от левого верхнего, поэтому вертикаль переворачивается. Вершины собираются
/// вручную, как в [`crate::background::fill_rect`]: PDF рисует
/// `PaintMode::Stroke` по замкнутому пути, и заливки (в canvas-пути она
/// прозрачная) в PDF не нужно вовсе.
fn placeholder_ops(rect: Rect, page: &PageGeometry) -> Vec<Op> {
    let (left, right) = (page.px_to_pt(rect.x), page.px_to_pt(rect.x + rect.width));
    let (top, bottom) = (
        page.height_pt() - page.px_to_pt(rect.y),
        page.height_pt() - page.px_to_pt(rect.y + rect.height),
    );
    let points = [(left, bottom), (right, bottom), (right, top), (left, top)]
        .into_iter()
        .map(|(x, y)| LinePoint {
            p: Point { x: Pt(x), y: Pt(y) },
            bezier: false,
        })
        .collect();

    vec![
        Op::SetOutlineThickness {
            pt: Pt(page.px_to_pt(PLACEHOLDER_STROKE_PX)),
        },
        Op::SetOutlineColor {
            col: to_pdf_color(PLACEHOLDER_STROKE),
        },
        Op::DrawPolygon {
            polygon: Polygon {
                rings: vec![PolygonRing { points }],
                mode: PaintMode::Stroke,
                winding_order: WindingOrder::NonZero,
            },
        },
    ]
}
