//! Таблицы DOCX в PDF: границы, заливка, выравнивание содержимого.
//!
//! Раскладка уже отдала эффективные границы каждой ячейки
//! ([`TableCellLayout::borders`]: собственные `w:tcBorders` поверх табличных
//! `w:tblBorders`, `nil`/`none` схлопнуты в `None`) и заливку (`w:shd`), так
//! что здесь они только переводятся в операции printpdf.
//!
//! Таблица приходит фрагментами — по одному `LayoutItem::Table` на страницу,
//! а внутри ячейки тем же элементом лежит вложенная таблица, поэтому обход
//! рекурсивный: содержимое ячейки рисует тот же диспетчер (`super::draw_items`),
//! что и тело документа, со сдвигом по вертикали.

use std::collections::BTreeMap;

use doc_converter_docx::layout::{LayoutItem, Rect, TableCellLayout, VAlign};
use doc_converter_docx::{Border, BorderStyle, Color as ModelColor, Shading, ShadingPattern};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::FontRegistry;
use printpdf::{Line, LineDashPattern, LinePoint, Op, Point, Pt};

use crate::fonts::EmbeddedFonts;
use crate::layout::{PageGeometry, RectPx};
use crate::styles::to_pdf_color;

use super::images::Images;

/// Толщина границы, у которой не записан `w:sz`: 0.5 pt — умолчание Word для
/// одиночной линии (`w:sz="4"`).
const DEFAULT_BORDER_PT: f32 = 0.5;

/// Во сколько раз координата ребра укрупняется в ключе склейки: шаг 1/100
/// пикселя раскладки — на порядок больше ошибки сложения `x + width`, но
/// заметно меньше любой реальной границы.
const EDGE_KEY_STEP: f32 = 100.0;

/// Длина штриха пунктира и промежуток между штрихами, pt.
const DOTTED: LineDashPattern = dash(&[1, 2, 0, 0, 0, 0]);
/// Штрих и промежуток штриховой линии, pt.
const DASHED: LineDashPattern = dash(&[4, 3, 0, 0, 0, 0]);
/// Штрих-пунктир: штрих, промежуток, точка, промежуток, pt.
const DASH_DOT: LineDashPattern = dash(&[3, 2, 1, 2, 0, 0]);
/// Штрих с двумя точками.
const DASH_DOT_DOT: LineDashPattern = dash(&[3, 2, 1, 2, 1, 2]);

/// Шаблон штриха по длинам из [`DOTTED`] и соседей; нули — «шага нет».
const fn dash(lengths: &[i64; 6]) -> LineDashPattern {
    const fn step(value: i64) -> Option<i64> {
        if value == 0 {
            None
        } else {
            Some(value)
        }
    }
    LineDashPattern {
        offset: 0,
        dash_1: step(lengths[0]),
        gap_1: step(lengths[1]),
        dash_2: step(lengths[2]),
        gap_2: step(lengths[3]),
        dash_3: step(lengths[4]),
        gap_3: step(lengths[5]),
    }
}

/// Нарисовать таблицу: заливки, границы и содержимое ячеек.
///
/// `offset_y` — сдвиг всей таблицы по вертикали в пикселях раскладки: он не
/// ноль, когда таблица лежит в ячейке другой таблицы, содержимое которой
/// сдвинуто [`v_align_shift`]-ом. Ячейки приходят в координатах страницы
/// (`TableCellLayout::rect`), поэтому больше сдвигать их не нужно.
pub(crate) fn draw(
    ops: &mut Vec<Op>,
    cells: &[TableCellLayout],
    page: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &Images,
    offset_y: f32,
) {
    // Заливка — под содержимым, границы — над ней, поэтому ячейки обходятся
    // трижды: так же устроен canvas-путь и ячейки XLSX (`painter::draw_page`).
    for cell in cells {
        draw_fill(ops, cell, page, offset_y);
    }
    // Общее ребро соседних ячеек отдают обе ячейки: линия на нём должна быть
    // одна, иначе одиночная граница утолщается вдвое.
    let mut edges: BTreeMap<EdgeKey, Edge> = BTreeMap::new();
    for cell in cells {
        collect_edges(&mut edges, cell, page, offset_y);
    }
    // Продолжения вертикального объединения раскладка нарезает по строкам и
    // даёт им границы `insideH`: в Word внутри объединённой ячейки линий нет,
    // поэтому ребро, проходящее через тело ячейки, не рисуется. Ребро на кромке
    // соседа (общая граница строк) остаётся — оно лежит ровно на границе.
    let bounds: Vec<Bounds> = cells
        .iter()
        .map(|cell| Bounds::of(cell, page, offset_y))
        .collect();
    edges.retain(|_, edge| !bounds.iter().any(|bounds| bounds.covers(edge.midpoint())));
    for edge in edges.values() {
        draw_edge(ops, edge);
    }
    for cell in cells {
        // Продолжение вертикального объединения без содержимого: текст уже
        // нарисован в начале объединения, дублировать его нельзя.
        if cell.v_merge_continue {
            continue;
        }
        let dy = offset_y + v_align_shift(cell);
        super::draw_items(&cell.content, ops, page, embedded, registry, images, dy);
    }
    // Штрих — состояние кисти потока: последняя граница могла быть пунктирной,
    // а следующий за таблицей элемент обязан рисоваться сплошной линией.
    ops.push(Op::SetLineDashPattern { dash: solid_dash() });
}

/// Заливка ячейки (`w:shd`): прямоугольник её же площади.
///
/// `w:val="nil"` заливки не задаёт; `clear` (равно как `solid` и узоры)
/// Word рисует фоном `w:fill` — узор растром не переносится, но фон остаётся.
/// Без цвета `w:fill` заливать нечем: `auto` — это «как в приложении», а не
/// чёрный, и Word его фоном не красит.
fn draw_fill(ops: &mut Vec<Op>, cell: &TableCellLayout, page: &PageGeometry, offset_y: f32) {
    let Some(fill) = cell
        .shading
        .as_ref()
        .filter(|shading| !matches!(shading.val, ShadingPattern::Nil))
        .and_then(|shading: &Shading| border_color(shading.fill.as_ref()))
    else {
        return;
    };
    let rect = shifted(&cell.rect, offset_y);
    let pdf = page.rect_to_pt(RectPx::new(rect.x, rect.y, rect.width, rect.height));
    crate::background::fill_rect(ops, pdf, page.height_pt(), fill);
}

/// Собрать рёбра ячейки в общую карту: совпавшие по геометрии склеиваются.
///
/// Рёбра сразу переводятся в точки PDF: у раскладки начало координат в левом
/// верхнем углу страницы, у printpdf — в левом нижнем.
fn collect_edges(
    edges: &mut BTreeMap<EdgeKey, Edge>,
    cell: &TableCellLayout,
    page: &PageGeometry,
    offset_y: f32,
) {
    let rect = shifted(&cell.rect, offset_y);
    let pdf = page.rect_to_pt(RectPx::new(rect.x, rect.y, rect.width, rect.height));
    let (x0, x1) = (pdf.x, pdf.x + pdf.w);
    let y_top = page.height_pt() - pdf.y;
    let y_bottom = page.height_pt() - (pdf.y + pdf.h);
    let sides = [
        (cell.borders.top.as_ref(), (x0, y_top), (x1, y_top)),
        (cell.borders.bottom.as_ref(), (x0, y_bottom), (x1, y_bottom)),
        (cell.borders.left.as_ref(), (x0, y_bottom), (x0, y_top)),
        (cell.borders.right.as_ref(), (x1, y_bottom), (x1, y_top)),
    ];
    for (border, from, to) in sides {
        let Some(border) = border else { continue };
        let key = EdgeKey::new(from, to);
        match edges.get_mut(&key) {
            // Границы на общем ребре разошлись (`w:tcBorders` одной из ячеек):
            // берётся более толстая — тонкая потерялась бы под ней целиком.
            Some(edge) => {
                if width_pt(&edge.border) < width_pt(border) {
                    edge.border = border.clone();
                }
            }
            None => {
                edges.insert(
                    key,
                    Edge {
                        from,
                        to,
                        border: border.clone(),
                    },
                );
            }
        }
    }
}

/// Ребро ячейки: отрезок в точках PDF и выигравшая граница.
struct Edge {
    from: (f32, f32),
    to: (f32, f32),
    border: Border,
}

impl Edge {
    /// Середина ребра: по ней ребро проверяется на «проходит через ячейку».
    // `f32::midpoint` появился только в Rust 1.85, а MSRV workspace — 1.82.
    #[allow(clippy::manual_midpoint)]
    fn midpoint(&self) -> (f32, f32) {
        (
            (self.from.0 + self.to.0) / 2.0,
            (self.from.1 + self.to.1) / 2.0,
        )
    }
}

/// Прямоугольник ячейки в точках PDF: левый, верхний, правый, нижний края.
struct Bounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

/// Насколько точка должна отстоять от кромки, чтобы считаться внутри тела
/// ячейки: ребро соседа ложится ровно на кромку, и разница только в округлении
/// координат.
const INSIDE_EPS_PT: f32 = 0.05;

impl Bounds {
    fn of(cell: &TableCellLayout, page: &PageGeometry, offset_y: f32) -> Self {
        let rect = shifted(&cell.rect, offset_y);
        let pdf = page.rect_to_pt(RectPx::new(rect.x, rect.y, rect.width, rect.height));
        Self {
            left: pdf.x,
            top: page.height_pt() - pdf.y,
            right: pdf.x + pdf.w,
            bottom: page.height_pt() - (pdf.y + pdf.h),
        }
    }

    /// Точка строго внутри прямоугольника — не на кромке.
    fn covers(&self, (x, y): (f32, f32)) -> bool {
        x > self.left + INSIDE_EPS_PT
            && x < self.right - INSIDE_EPS_PT
            && y < self.top - INSIDE_EPS_PT
            && y > self.bottom + INSIDE_EPS_PT
    }
}

/// Ключ склейки рёбер: концы, округлённые до [`EDGE_KEY_STEP`].
///
/// Ребро — отрезок по оси, поэтому его ключ упорядочивается по концам: у
/// ячейки слева ребро идёт снизу вверх, у соседа справа — сверху вниз.
/// Геометрия самого ребра хранится отдельно и не округляется: по ключу
/// только ищутся совпадения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct EdgeKey {
    from: (i64, i64),
    to: (i64, i64),
}

impl EdgeKey {
    fn new(from: (f32, f32), to: (f32, f32)) -> Self {
        // Координаты страницы далеки от пределов `i64` при этом шаге.
        #[allow(clippy::cast_possible_truncation)]
        let quantize = |v: f32| (v * EDGE_KEY_STEP).round() as i64;
        let a = (quantize(from.0), quantize(from.1));
        let b = (quantize(to.0), quantize(to.1));
        if a <= b {
            Self { from: a, to: b }
        } else {
            Self { from: b, to: a }
        }
    }
}

/// Нарисовать ребро: толщина — `w:sz`/8 pt, штрих — по `w:val`.
///
/// `double` рисуется двумя линиями по краям полосы шириной `w:sz` — так же
/// выглядит двойная граница в Word; остальные стили (`triple`, `wave`,
/// `thickThin*Gap`, рельеф) аппроксимируются одиночной линией.
fn draw_edge(ops: &mut Vec<Op>, edge: &Edge) {
    let (from, to, border) = (edge.from, edge.to, &edge.border);
    let width = width_pt(border);
    ops.push(Op::SetLineDashPattern {
        dash: border_dash(&border.val),
    });
    ops.push(Op::SetOutlineColor {
        col: to_pdf_color(border_color(border.color.as_ref()).unwrap_or(Color::BLACK)),
    });
    // Двойная линия — две тонкие по краям полосы: каждая в треть толщины,
    // разнесённые на треть от центра.
    let (line_w, offsets): (f32, &[f32]) =
        if matches!(border.val, BorderStyle::Double | BorderStyle::DoubleWave) {
            (width / 3.0, &[-width / 3.0, width / 3.0])
        } else {
            (width, &[0.0])
        };
    ops.push(Op::SetOutlineThickness { pt: Pt(line_w) });
    // Нормаль к ребру: смещения двойной линии — поперёк него.
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    let normal = if length > 0.0 {
        (-dy / length, dx / length)
    } else {
        (0.0, 0.0)
    };
    for offset in offsets {
        let shift = (normal.0 * offset, normal.1 * offset);
        ops.push(Op::DrawLine {
            line: segment(
                (from.0 + shift.0, from.1 + shift.1),
                (to.0 + shift.0, to.1 + shift.1),
            ),
        });
    }
}

/// Штрих границы по её стилю (`ST_Border`).
///
/// Волнистые линии и «тройные» штрихи узоров PDF не знает: у них берётся
/// сплошная линия той же толщины.
fn border_dash(style: &BorderStyle) -> LineDashPattern {
    match style {
        BorderStyle::Dotted => DOTTED,
        BorderStyle::Dashed | BorderStyle::DashSmallGap | BorderStyle::DashDotStroked => DASHED,
        BorderStyle::DotDash => DASH_DOT,
        BorderStyle::DotDotDash => DASH_DOT_DOT,
        _ => solid_dash(),
    }
}

/// Сплошная линия: пустой шаблон штриха сбрасывает состояние кисти потока.
const fn solid_dash() -> LineDashPattern {
    dash(&[0, 0, 0, 0, 0, 0])
}

/// Толщина границы в точках: `w:sz` хранится в восьмых долях пункта.
// `w:sz` — восьмые доли пункта, заведомо меньше 2^24: точность не теряется.
#[allow(clippy::cast_precision_loss)]
fn width_pt(border: &Border) -> f32 {
    match border.sz {
        Some(sz) => sz as f32 / 8.0,
        None => DEFAULT_BORDER_PT,
    }
}

/// Цвет границы или заливки: `RRGGBB` из модели → `RRGGBBAA` `DisplayList`.
///
/// `auto` и отсутствие цвета — чёрный: `auto` в OOXML и есть цвет по
/// умолчанию. Та же перестановка каналов, что у цвета текста
/// ([`super::text_color`]).
fn border_color(color: Option<&ModelColor>) -> Option<Color> {
    match color {
        Some(ModelColor::Rgb(rgb)) => Some(Color(((rgb & 0x00FF_FFFF) << 8) | 0xFF)),
        _ => None,
    }
}

/// Сдвиг содержимого ячейки по `w:vAlign`, в пикселях раскладки.
///
/// Свободное место ячейки делится по вертикали: `Top` оставляет его снизу,
/// `Center` — поровну, `Bottom` — сверху. Отступ содержимого от верхней кромки
/// (`w:tcMar top`) сохраняется и снизу — так же кладёт текст Word, а поля ячейки
/// до PDF не доезжают.
fn v_align_shift(cell: &TableCellLayout) -> f32 {
    if matches!(cell.v_align, VAlign::Top) {
        return 0.0;
    }
    let Some((top, bottom)) = content_bounds(&cell.content) else {
        return 0.0;
    };
    let free = (cell.rect.y + cell.rect.height) - bottom - (top - cell.rect.y);
    match cell.v_align {
        VAlign::Top => 0.0,
        VAlign::Center => (free / 2.0).max(0.0),
        VAlign::Bottom => free.max(0.0),
    }
}

/// Верх и низ содержимого: объединение прямоугольников его элементов.
fn content_bounds(items: &[LayoutItem]) -> Option<(f32, f32)> {
    let mut bounds: Option<(f32, f32)> = None;
    for item in items {
        let rect = match item {
            LayoutItem::Paragraph { rect, .. }
            | LayoutItem::Table { rect, .. }
            | LayoutItem::Image { rect, .. } => rect,
            LayoutItem::Float(float) => &float.rect,
            LayoutItem::PageBreak | LayoutItem::ColumnBreak => continue,
        };
        let bottom = rect.y + rect.height;
        bounds = Some(match bounds {
            Some((top, low)) => (top.min(rect.y), low.max(bottom)),
            None => (rect.y, bottom),
        });
    }
    bounds
}

/// Прямоугольник, сдвинутый по вертикали на `dy` пикселей раскладки.
fn shifted(rect: &Rect, dy: f32) -> Rect {
    Rect::new(rect.x, rect.y + dy, rect.width, rect.height)
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

#[cfg(test)]
mod tests {
    use super::*;
    use doc_converter_core::NodeId;
    use doc_converter_docx::CellBorders;

    /// Геометрия страницы, как её собрал бы экспорт: без полей и масштаба.
    fn page() -> PageGeometry {
        PageGeometry::new(&crate::options::PageConfig::default())
    }

    fn border(style: BorderStyle, sz: u32) -> Border {
        Border {
            val: style,
            sz: Some(sz),
            space: Some(0),
            color: Some(ModelColor::Rgb(0x0011_2233)),
        }
    }

    /// Ячейка с заданными границами и содержимым.
    fn cell(rect: Rect, borders: CellBorders, content: Vec<LayoutItem>) -> TableCellLayout {
        TableCellLayout {
            rect,
            content,
            v_align: VAlign::Top,
            shading: None,
            borders: Box::new(borders),
            v_merge_continue: false,
        }
    }

    fn paragraph(y: f32, height: f32) -> LayoutItem {
        LayoutItem::Paragraph {
            node_id: NodeId::new(1),
            rect: Rect::new(0.0, y, 100.0, height),
            text: "x".to_owned(),
            line_height: height,
            color: None,
            font_family: None,
            size: doc_converter_docx::HalfPoint::new(24),
            bold: false,
            italic: false,
            underline: false,
        }
    }

    /// Толщина берётся из `w:sz` (восьмые доли пункта), а без неё — 0.5 pt.
    #[test]
    fn width_is_eighths_of_a_point() {
        assert!((width_pt(&border(BorderStyle::Single, 8)) - 1.0).abs() < 1e-6);
        assert!((width_pt(&border(BorderStyle::Double, 6)) - 0.75).abs() < 1e-6);
        let no_sz = Border {
            sz: None,
            ..border(BorderStyle::Single, 0)
        };
        assert!((width_pt(&no_sz) - DEFAULT_BORDER_PT).abs() < 1e-6);
    }

    /// Цвет — `RRGGBB` из модели; `auto` и отсутствие цвета — чёрный.
    #[test]
    fn color_comes_from_the_border() {
        assert_eq!(
            border_color(Some(&ModelColor::Rgb(0x002F_5496))),
            Some(Color(0x2F54_96FF))
        );
        assert_eq!(border_color(Some(&ModelColor::Auto)), None);
        assert_eq!(border_color(Some(&ModelColor::None)), None);
        assert_eq!(border_color(None), None);
    }

    /// Штриховые стили становятся шаблоном штриха, остальные — сплошной.
    #[test]
    fn dashed_styles_become_dash_patterns() {
        assert_eq!(border_dash(&BorderStyle::Single), solid_dash());
        assert_eq!(border_dash(&BorderStyle::Dotted), DOTTED);
        assert_eq!(border_dash(&BorderStyle::Dashed), DASHED);
        assert_eq!(border_dash(&BorderStyle::DashSmallGap), DASHED);
        assert_eq!(border_dash(&BorderStyle::DotDash), DASH_DOT);
        assert_eq!(border_dash(&BorderStyle::DotDotDash), DASH_DOT_DOT);
        // Волна и «тройные» линии PDF не знает: они рисуются сплошной.
        assert_eq!(border_dash(&BorderStyle::Wave), solid_dash());
        assert!(DOTTED.as_array().len() >= 2 && solid_dash().as_array().is_empty());
    }

    /// Двойная граница — две линии по краям полосы, каждая в треть толщины.
    #[test]
    fn double_border_draws_two_parallel_lines() {
        let edge = Edge {
            from: (10.0, 20.0),
            to: (110.0, 20.0),
            border: border(BorderStyle::Double, 8),
        };
        let mut ops = Vec::new();
        draw_edge(&mut ops, &edge);

        let mut widths = Vec::new();
        let mut ys = Vec::new();
        for op in &ops {
            match op {
                Op::SetOutlineThickness { pt } => widths.push(pt.0),
                Op::DrawLine { line } => ys.push(line.points[0].p.y.0),
                _ => {}
            }
        }
        assert_eq!(ys.len(), 2, "двойная граница — две линии");
        // 1 pt / 3 → 0.333 pt каждая, разнесены на треть от центра ребра.
        assert!((widths[0] - 1.0 / 3.0).abs() < 1e-6);
        ys.sort_by(f32::total_cmp);
        assert!((ys[1] - ys[0] - 2.0 / 3.0).abs() < 1e-5, "{ys:?}");
        assert!((ys[0] - (20.0 - 1.0 / 3.0)).abs() < 1e-5, "{ys:?}");
    }

    /// Общее ребро двух ячеек склеивается, а из разошедшихся границ остаётся
    /// более толстая.
    #[test]
    fn shared_edge_keeps_the_thicker_border() {
        let left = cell(
            Rect::new(0.0, 0.0, 50.0, 20.0),
            CellBorders {
                right: Some(border(BorderStyle::Single, 4)),
                ..CellBorders::default()
            },
            Vec::new(),
        );
        let right = cell(
            Rect::new(50.0, 0.0, 50.0, 20.0),
            CellBorders {
                left: Some(border(BorderStyle::Single, 16)),
                ..CellBorders::default()
            },
            Vec::new(),
        );
        let mut edges = BTreeMap::new();
        collect_edges(&mut edges, &left, &page(), 0.0);
        assert_eq!(edges.len(), 1, "у ячейки одна граница — одно ребро");
        collect_edges(&mut edges, &right, &page(), 0.0);
        assert_eq!(edges.len(), 1, "общее ребро соседей склеивается");
        let edge = edges.values().next().unwrap();
        assert_eq!(edge.border.sz, Some(16), "осталась более толстая граница");
    }

    /// Продолжение вертикального объединения приходит без содержимого.
    #[test]
    fn v_align_shift_divides_the_free_space() {
        let mut cell = cell(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            CellBorders::default(),
            vec![paragraph(10.0, 20.0)],
        );
        // Свободно 100 - 20 текста - 10 поля сверху = 70; снизу остаётся столько
        // же, сколько сверху, поэтому сдвиг максимум 70 - 10 = 60.
        cell.v_align = VAlign::Top;
        assert!(v_align_shift(&cell).abs() < 1e-6);
        cell.v_align = VAlign::Center;
        assert!((v_align_shift(&cell) - 30.0).abs() < 1e-6);
        cell.v_align = VAlign::Bottom;
        assert!((v_align_shift(&cell) - 60.0).abs() < 1e-6);
        // Содержимое выше ячейки не поднимается.
        cell.v_align = VAlign::Bottom;
        cell.content = vec![paragraph(0.0, 200.0)];
        assert!(v_align_shift(&cell).abs() < 1e-6);
    }
}
