//! Плавающие элементы DOCX (`wp:anchor`) в PDF.
//!
//! Рисуется плейсхолдер — контур прямоугольника анкера, ровно как у
//! canvas-пути (`doc_converter_docx::render::draw_float_placeholder`):
//! настоящей картинки нет, потому что `FloatKind::Image` несёт только ссылку
//! на часть архива (`rel_id`/`part`), а байтов медиачасти публичное API
//! `doc-converter-docx` не отдаёт — `Document` не держит архив. Пока на месте
//! рисунка видны его габариты, и раскладка с обтеканием проверяемы на глаз.
//!
//! `behind_text` (`behindDoc`) учитывается порядком операций страницы: элемент
//! под текстом встаёт в начало, остальные дописываются в конец. Раскладка
//! кладёт `LayoutItem::Float` после строк абзаца-анкера, поэтому без переноса
//! вперёд рисунок «под текстом» лёг бы поверх него.

use doc_converter_docx::layout::{FloatElement, Rect};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::FontRegistry;
use printpdf::{LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, WindingOrder};

use crate::fonts::EmbeddedFonts;
use crate::layout::PageGeometry;
use crate::styles::to_pdf_color;

/// Цвет контура плейсхолдера — тот же серый, что у canvas-пути
/// (`render::PLACEHOLDER_STROKE` в `doc-converter-docx`).
const PLACEHOLDER_STROKE: Color = Color::rgba(0x80, 0x80, 0x80, 0xFF);

/// Толщина контура плейсхолдера в пикселях раскладки — как у canvas-пути
/// (`render::PLACEHOLDER_STROKE_W`).
const PLACEHOLDER_STROKE_PX: f32 = 1.0;

/// Нарисовать плавающий элемент в его прямоугольнике (`FloatElement::rect`).
///
/// Прямоугольник анкера лежит в координатах страницы — той же, что у остальных
/// элементов раскладки; сдвигать его не нужно. Вид элемента не различается:
/// canvas-путь тоже рисует одни габариты, а `FloatKind::TextBox` раскладка
/// пока не порождает.
///
/// `offset_y` — сдвиг в пикселях раскладки: он не ноль, когда элемент — часть
/// содержимого ячейки, выровненного по `w:vAlign`.
pub(crate) fn draw(
    ops: &mut Vec<Op>,
    float: &FloatElement,
    page: &PageGeometry,
    _embedded: &EmbeddedFonts,
    _registry: &mut FontRegistry,
    offset_y: f32,
) {
    let rect = Rect::new(
        float.rect.x,
        float.rect.y + offset_y,
        float.rect.width,
        float.rect.height,
    );
    let placeholder = placeholder_ops(rect, page);
    if float.behind_text {
        // Два элемента под текстом на одной странице не перекрываются —
        // раскладка разводит их по вертикали (`float::blocking_bottom`), —
        // поэтому порядок между ними не наблюдаем и обратный порядок вставки
        // безопасен.
        ops.splice(0..0, placeholder);
    } else {
        ops.extend(placeholder);
    }
}

/// Операции контура плейсхолдера по прямоугольнику анкера.
///
/// Начало координат printpdf — левый нижний угол страницы, а `rect` отсчитан
/// от левого верхнего, поэтому вертикаль переворачивается. Вершины собираются
/// вручную, как в [`crate::background::fill_rect`]: printpdf рисует
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
