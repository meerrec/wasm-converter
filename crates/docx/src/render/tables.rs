//! Таблица на canvas: заливка ячеек, границы и содержимое.
//!
//! Раскладка отдаёт границы уже эффективными (`w:tcBorders` поверх
//! `w:tblBorders`, `nil` и `none` отфильтрованы), но общее ребро соседних
//! ячеек приходит дважды — по одному от каждой, — а ячейка с `w:gridSpan`
//! перекрывает рёбра соседей снизу. Поэтому рёбра всех ячеек фрагмента
//! собираются, схлопываются по геометрии ([`draw_borders`]) и рисуются по
//! одному разу: иначе линии наложились бы и удвоились по толщине.
//!
//! Содержимое ячейки приходит в координатах страницы (`layout_cell_items`
//! отсчитывает его от верхнего левого угла ячейки, уже сдвинув по `w:vAlign`), —
//! сдвигать его при отрисовке не нужно и нечем: повторный сдвиг увёл бы текст
//! за пределы ячейки.

use doc_converter_render::display_list::{Color, DisplayList, DrawCommand, LineStyle, StringRef};
use doc_converter_render::viewport::Viewport;

use crate::layout::engine::{TableCellLayout, PX_PER_POINT};
use crate::model::raw::{Border, BorderStyle, Color as OoxmlColor, Shading, ShadingPattern};

use super::{draw_items, fill_rect, text_color, to_screen};

/// Толщина границы по умолчанию в восьмых долях пункта: `w:sz="4"` — те
/// полпункта, которые Word ставит одиночной границе без `w:sz`.
const DEFAULT_BORDER_SZ: u32 = 4;

/// Ниже этой толщины граница не опускается.
///
/// `w:sz="0"` — линия нулевой толщины, которую canvas не нарисует вовсе
/// (`lineWidth = 0` для него не значение, а «оставить прежнее»): волосяная
/// линия честнее невидимой.
const MIN_BORDER_W: f32 = 0.5;

/// Порог совпадения координат рёбер, в пикселях раскладки.
///
/// Рёбра соседних ячеек считает одна и та же арифметика раскладки, но проверять
/// их на точное равенство `f32` нельзя: 1/32 px заведомо больше накопленной
/// погрешности и заведомо меньше любого настоящего зазора между границами.
const EDGE_EPS: f32 = 1.0 / 32.0;

/// Таблица: заливка, схлопнутые границы, содержимое.
pub(super) fn draw_table(
    cells: &[TableCellLayout],
    viewport: &Viewport,
    font: StringRef,
    out: &mut DisplayList,
) {
    // Порядок слоёв — снизу вверх: заливка, границы, текст. Границы рисуются
    // все до единого текста: иначе линия следующей ячейки легла бы поверх
    // текста предыдущей.
    for cell in cells {
        draw_shading(cell, viewport, out);
    }
    draw_borders(cells, viewport, out);
    for cell in cells {
        // Продолжение вертикального объединения: содержимое осталось в начале
        // объединения, здесь только геометрия — но заливка и границы нужны.
        if cell.v_merge_continue {
            continue;
        }
        draw_items(&cell.content, viewport, font, out);
    }
}

/// Заливка ячейки (`w:shd`) — прямоугольник под её границами и текстом.
fn draw_shading(cell: &TableCellLayout, viewport: &Viewport, out: &mut DisplayList) {
    let Some(fill) = shading_fill(cell.shading.as_ref()) else {
        return;
    };
    let rect = &cell.rect;
    let (x, y) = to_screen(viewport, rect.x, rect.y);
    fill_rect(
        out,
        x,
        y,
        rect.width * viewport.scale,
        rect.height * viewport.scale,
        fill,
    );
}

/// Цвет заливки ячейки; `None` — заливать нечего.
///
/// `clear` — не «без заливки», а обычная заливка Word: так он пишет фон ячейки
/// (`w:fill` — цвет фона, `w:val="clear"` — «без узора»), и файл, где шапку
/// красили из Word, приходит именно так (`tables/borders_shading.docx`).
/// У `solid` фон закрыт узором, поэтому у него берётся цвет узора (`w:color`),
/// а `w:fill` — лишь запасной. `nil`, `auto`/`none` и отсутствующий `w:fill`
/// не заливают ничего; узорные `val` (полосы, клетка) аппроксимируются фоном
/// `w:fill`: рисовать в кадре штриховку нечем, а фон узора ближе к картинке
/// Word, чем сплошной цвет штрихов.
fn shading_fill(shading: Option<&Shading>) -> Option<Color> {
    let shading = shading?;
    let fill = explicit_color(shading.fill.as_ref());
    match shading.val {
        ShadingPattern::Nil => None,
        ShadingPattern::Clear => fill,
        ShadingPattern::Solid => explicit_color(shading.color.as_ref()).or(fill),
        _ => fill.or_else(|| explicit_color(shading.color.as_ref())),
    }
}

/// Ребро ячейки: отрезок с границей из модели.
struct Edge<'a> {
    /// Ребро горизонтальное (постоянна координата `y`) или вертикальное (`x`).
    horizontal: bool,
    /// Координата постоянной оси в пикселях раскладки.
    fixed: f32,
    /// Начало и конец отрезка по другой оси.
    start: f32,
    end: f32,
    border: &'a Border,
}

/// Границы фрагмента таблицы: собрать рёбра, схлопнуть общие, нарисовать.
///
/// Схлопывание — по геометрии, как в таблице с `border-collapse`: рёбра одного
/// направления с одной координатой (в пределах [`EDGE_EPS`]) сливаются в один
/// отрезок, и границу отрезка выбирает [`border_weight`] — побеждает более
/// тяжёлая, при равенстве верхняя или левая (её ребро идёт первым в порядке
/// отрисовки ячеек). Перекрытие, а не касание: у соседей по строке рёбра
/// разные, и стирать их в один отрезок нельзя, — а у соседей по столбцу ребро
/// одно и приходит дважды.
fn draw_borders(cells: &[TableCellLayout], viewport: &Viewport, out: &mut DisplayList) {
    let mut edges = Vec::new();
    collect_edges(cells, &mut edges);
    edges.sort_by(|a, b| {
        a.horizontal
            .cmp(&b.horizontal)
            .then(quantized(a.fixed).cmp(&quantized(b.fixed)))
            .then(a.start.total_cmp(&b.start))
    });

    let mut index = 0;
    while index < edges.len() {
        let (horizontal, fixed) = (edges[index].horizontal, quantized(edges[index].fixed));
        // Группа — рёбра одной оси с одной координатой.
        let group_end = edges[index..]
            .iter()
            .take_while(|edge| edge.horizontal == horizontal && quantized(edge.fixed) == fixed)
            .count()
            + index;

        // Внутри группы рёбра идут по возрастанию начала, поэтому общий отрезок
        // набирается одним проходом: следующее ребро продолжает текущий
        // отрезок, пока перекрывается с ним.
        let mut at = index;
        while at < group_end {
            let start = edges[at].start;
            let mut end = edges[at].end;
            let mut winner = &edges[at];
            at += 1;
            while at < group_end && edges[at].start < end - EDGE_EPS {
                if border_weight(edges[at].border) > border_weight(winner.border) {
                    winner = &edges[at];
                }
                end = end.max(edges[at].end);
                at += 1;
            }
            push_edge(out, viewport, horizontal, winner, start, end);
        }
        index = group_end;
    }
}

/// Собрать рёбра ячеек фрагмента: по одному на каждую нарисованную сторону.
fn collect_edges<'a>(cells: &'a [TableCellLayout], out: &mut Vec<Edge<'a>>) {
    for cell in cells {
        let rect = cell.rect;
        let sides = [
            (
                cell.borders.top.as_ref(),
                true,
                rect.y,
                rect.x,
                rect.x + rect.width,
            ),
            (
                cell.borders.bottom.as_ref(),
                true,
                rect.y + rect.height,
                rect.x,
                rect.x + rect.width,
            ),
            (
                cell.borders.left.as_ref(),
                false,
                rect.x,
                rect.y,
                rect.y + rect.height,
            ),
            (
                cell.borders.right.as_ref(),
                false,
                rect.x + rect.width,
                rect.y,
                rect.y + rect.height,
            ),
        ];
        for (border, horizontal, fixed, start, end) in sides {
            let Some(border) = border else {
                continue;
            };
            if border_line(border).is_none() {
                continue;
            }
            out.push(Edge {
                horizontal,
                fixed,
                start,
                end,
                border,
            });
        }
    }
}

/// Одна команда `Line` на схлопнутое ребро.
fn push_edge(
    out: &mut DisplayList,
    viewport: &Viewport,
    horizontal: bool,
    edge: &Edge<'_>,
    start: f32,
    end: f32,
) {
    let Some((style, width)) = border_line(edge.border) else {
        return;
    };
    let (x1, y1, x2, y2) = if horizontal {
        (start, edge.fixed, end, edge.fixed)
    } else {
        (edge.fixed, start, edge.fixed, end)
    };
    let (x1, y1) = to_screen(viewport, x1, y1);
    let (x2, y2) = to_screen(viewport, x2, y2);
    out.push(DrawCommand::Line {
        x1,
        y1,
        x2,
        y2,
        stroke: ooxml_color(edge.border.color.as_ref()),
        // Толщина — в тех же физических пикселях, что и координаты.
        stroke_w: width * viewport.scale,
        style,
    });
}

/// Координата ребра, округлённая до [`EDGE_EPS`]: так рёбра соседей попадают в
/// одну группу, даже если разошлись на округлении `f32`.
#[allow(clippy::cast_possible_truncation)]
fn quantized(value: f32) -> i32 {
    (value / EDGE_EPS).round() as i32
}

/// Линия границы: рисунок штриха и толщина в пикселях раскладки.
///
/// `w:sz` — восьмые доли пункта, поэтому `w:sz="4"` (полпункта) при 96 dpi
/// меньше пикселя; пересчёт — тот же [`PX_PER_POINT`], которым меряет раскладка.
/// `LineStyle` знает штрих, точки и двойную линию, поэтому `dashed` и `dotted`
/// доезжают как есть, а `double` получает полную толщину — painter сам делит её
/// на две линии и зазор (так же поступает XLSX-путь, `xlsx::paint::border_line`).
/// Тройные, волнистые, трёхмерные и прочие экзотические границы Word
/// аппроксимируются одиночной линией: рисунка для них в `LineStyle` нет.
#[allow(clippy::cast_precision_loss)]
fn border_line(border: &Border) -> Option<(LineStyle, f32)> {
    let (style, default_sz) = match border.val {
        BorderStyle::Nil | BorderStyle::None => return None,
        BorderStyle::Dotted | BorderStyle::DashSmallGap => (LineStyle::Dotted, DEFAULT_BORDER_SZ),
        BorderStyle::Dashed
        | BorderStyle::DotDash
        | BorderStyle::DotDotDash
        | BorderStyle::DashDotStroked => (LineStyle::Dashed, DEFAULT_BORDER_SZ),
        BorderStyle::Double => (LineStyle::Double, 6),
        BorderStyle::Thick => (LineStyle::Solid, 12),
        _ => (LineStyle::Solid, DEFAULT_BORDER_SZ),
    };
    let sz = border.sz.unwrap_or(default_sz);
    let width = (sz as f32 / 8.0 * PX_PER_POINT).max(MIN_BORDER_W);
    Some((style, width))
}

/// Вес границы в конфликте на общем ребре: побеждает более тяжёлая.
///
/// Порядок — как у приоритета границ Word: двойная тяжелее утолщённой, та —
/// одиночной, одиночная — штриха, штрих — пунктира. При равном стиле решает
/// толщина `w:sz`.
fn border_weight(border: &Border) -> (u8, u32) {
    let rank = match border.val {
        BorderStyle::Nil | BorderStyle::None => 0,
        BorderStyle::Dotted | BorderStyle::DashSmallGap => 1,
        BorderStyle::Dashed
        | BorderStyle::DotDash
        | BorderStyle::DotDotDash
        | BorderStyle::DashDotStroked => 2,
        BorderStyle::Thick => 5,
        BorderStyle::Double => 6,
        // Одиночная и вся экзотика, которую [`border_line`] рисует одиночной.
        _ => 3,
    };
    (rank, border.sz.unwrap_or(DEFAULT_BORDER_SZ))
}

/// Цвет границы; отсутствие цвета и `auto` — чёрный.
fn ooxml_color(color: Option<&OoxmlColor>) -> Color {
    explicit_color(color).unwrap_or_else(|| text_color(None))
}

/// Цвет из модели (`RRGGBB`) в цвет кадра; `auto`/`none` и отсутствие — `None`.
fn explicit_color(color: Option<&OoxmlColor>) -> Option<Color> {
    match color {
        Some(OoxmlColor::Rgb(rgb)) => Some(text_color(Some(*rgb))),
        Some(OoxmlColor::Auto | OoxmlColor::None) | None => None,
    }
}

#[cfg(test)]
mod tests {
    use doc_converter_core::NodeId;

    use super::*;
    use crate::layout::engine::{LayoutItem, Rect, VAlign};
    use crate::model::table::CellBorders;

    /// Кадр без прокрутки и зума: координаты команд равны координатам раскладки.
    fn viewport() -> Viewport {
        Viewport::new(0.0, 0.0, 800.0, 600.0, 1.0)
    }

    /// Сплошная граница: `sz` — восьмые доли пункта.
    fn border(val: BorderStyle, sz: u32, color: OoxmlColor) -> Border {
        Border {
            val,
            sz: Some(sz),
            space: Some(0),
            color: Some(color),
        }
    }

    /// Границы ячейки из сторон: сверху, справа, снизу, слева.
    fn sides(
        top: Option<Border>,
        right: Option<Border>,
        bottom: Option<Border>,
        left: Option<Border>,
    ) -> CellBorders {
        CellBorders {
            top,
            left,
            bottom,
            right,
        }
    }

    /// Ячейка с заданными прямоугольником, границами и содержимым.
    fn cell(rect: Rect, borders: CellBorders, content: Vec<LayoutItem>) -> TableCellLayout {
        TableCellLayout {
            rect,
            content,
            v_align: VAlign::Top,
            shading: None,
            borders: Box::new(borders),
            v_merge_continue: false,
            table_id: doc_converter_core::NodeId::ROOT,
            cell_id: doc_converter_core::NodeId::ROOT,
            row: 0,
            col: 0,
            grid_span: 1,
            row_span: 1,
            is_header: false,
            header_repeat: false,
        }
    }

    /// Нарисовать таблицу и вернуть кадр.
    fn draw(cells: &[TableCellLayout]) -> DisplayList {
        let mut out = DisplayList::new();
        draw_table(cells, &viewport(), StringRef::default(), &mut out);
        out
    }

    /// Команды `Line` кадра: координаты, стиль и толщина.
    fn lines(dl: &DisplayList) -> Vec<(LineStyle, f32, f32, Color)> {
        (0..dl.len())
            .filter_map(|i| match dl.cmd(i) {
                Some(DrawCommand::Line {
                    y1,
                    stroke,
                    stroke_w,
                    style,
                    ..
                }) => Some((*style, *y1, *stroke_w, *stroke)),
                _ => None,
            })
            .collect()
    }

    /// Прямоугольники заливки кадра.
    fn rects(dl: &DisplayList) -> Vec<(f32, f32, f32, f32, Color)> {
        (0..dl.len())
            .filter_map(|i| match dl.cmd(i) {
                Some(DrawCommand::Rect {
                    x, y, w, h, fill, ..
                }) => Some((*x, *y, *w, *h, *fill)),
                _ => None,
            })
            .collect()
    }

    /// Сравнить координату или размер: `f32` на равенство не проверяем.
    fn assert_px(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{what}: ожидалось {expected}, получено {actual}"
        );
    }

    /// Общее ребро соседей по столбцу — одна линия, и её границу выбирает вес.
    #[test]
    fn a_shared_edge_is_one_line_and_the_heavier_border_wins() {
        let thin = border(BorderStyle::Single, 4, OoxmlColor::Rgb(0x00_00_00));
        let thick = border(BorderStyle::Single, 16, OoxmlColor::Rgb(0xFF_00_00));
        let upper = cell(
            Rect::new(0.0, 0.0, 100.0, 20.0),
            sides(None, None, Some(thin), None),
            Vec::new(),
        );
        let lower = cell(
            Rect::new(0.0, 20.0, 100.0, 20.0),
            sides(Some(thick), None, None, None),
            Vec::new(),
        );

        let drawn = lines(&draw(&[upper, lower]));

        assert_eq!(drawn.len(), 1, "ребро нарисовано один раз: {drawn:?}");
        assert_eq!(drawn[0].0, LineStyle::Solid);
        assert_px(drawn[0].1, 20.0, "линия на общей границе ячеек");
        assert_eq!(drawn[0].3, Color(0xFF00_00FF), "победила тяжёлая граница");
        assert_px(drawn[0].2, 16.0 / 8.0 * PX_PER_POINT, "толщина победителя");
    }

    /// Равные границы на общем ребре тоже дают одну линию, а не две.
    #[test]
    fn equal_borders_on_a_shared_edge_are_drawn_once() {
        let same = border(BorderStyle::Single, 4, OoxmlColor::Rgb(0x2F_54_96));
        let upper = cell(
            Rect::new(0.0, 0.0, 100.0, 20.0),
            sides(None, None, Some(same.clone()), None),
            Vec::new(),
        );
        let lower = cell(
            Rect::new(0.0, 20.0, 100.0, 20.0),
            sides(Some(same), None, None, None),
            Vec::new(),
        );

        let drawn = lines(&draw(&[upper, lower]));

        assert_eq!(drawn.len(), 1, "линия не удваивается: {drawn:?}");
        assert_eq!(drawn[0].3, Color(0x2F54_96FF));
    }

    /// Соседи по строке — разные рёбра: их линии не сливаются в одну.
    #[test]
    fn edges_of_neighbours_in_a_row_keep_their_own_borders() {
        let left = cell(
            Rect::new(0.0, 0.0, 50.0, 20.0),
            sides(
                None,
                None,
                Some(border(BorderStyle::Single, 4, OoxmlColor::Rgb(0xFF_00_00))),
                None,
            ),
            Vec::new(),
        );
        let right = cell(
            Rect::new(50.0, 0.0, 50.0, 20.0),
            sides(
                None,
                None,
                Some(border(BorderStyle::Double, 6, OoxmlColor::Rgb(0x00_00_FF))),
                None,
            ),
            Vec::new(),
        );

        let drawn = lines(&draw(&[left, right]));

        assert_eq!(drawn.len(), 2, "каждое ребро своё: {drawn:?}");
        assert_eq!(drawn[0].0, LineStyle::Solid);
        assert_eq!(drawn[0].3, Color(0xFF00_00FF));
        assert_eq!(drawn[1].0, LineStyle::Double);
        assert_eq!(drawn[1].3, Color(0x0000_FFFF));
    }

    /// `nil` и `none` не рисуют линию, `dotted` и `dashed` доезжают рисунком,
    /// `double` получает полную толщину, `w:sz="0"` — волосяную.
    #[test]
    fn border_styles_become_line_styles() {
        let one = |val: BorderStyle, sz: Option<u32>| {
            let mut border = border(val, 4, OoxmlColor::Rgb(0x11_11_11));
            border.sz = sz;
            cell(
                Rect::new(0.0, 0.0, 100.0, 20.0),
                sides(Some(border), None, None, None),
                Vec::new(),
            )
        };
        let pt = |sz: f32| sz / 8.0 * PX_PER_POINT;

        for val in [BorderStyle::Nil, BorderStyle::None] {
            assert!(
                lines(&draw(&[one(val, Some(4))])).is_empty(),
                "грани нет — линии нет"
            );
        }
        let dotted = lines(&draw(&[one(BorderStyle::Dotted, Some(4))]));
        assert_eq!(dotted[0].0, LineStyle::Dotted);
        assert!((dotted[0].2 - pt(4.0)).abs() < 0.01, "толщина `w:sz`");

        let dashed = lines(&draw(&[one(BorderStyle::Dashed, Some(4))]));
        assert_eq!(dashed[0].0, LineStyle::Dashed);

        // `w:sz` без значения — толщина по умолчанию, у двойной своя.
        for sz in [Some(6), None] {
            let double = lines(&draw(&[one(BorderStyle::Double, sz)]));
            assert_eq!(double[0].0, LineStyle::Double);
            assert!(
                (double[0].2 - pt(6.0)).abs() < 0.01,
                "двойная линия получает полную толщину: {}",
                double[0].2
            );
        }

        let hairline = lines(&draw(&[one(BorderStyle::Single, Some(0))]));
        assert!(
            (hairline[0].2 - MIN_BORDER_W).abs() < 0.01,
            "линия не тоньше волосяной"
        );
    }

    /// Заливка: `clear` и `solid` — по цвету, `nil` и `auto` — ничего.
    #[test]
    fn cell_shading_fills_the_cell() {
        let shaded = |shading: Shading| {
            let mut cell = cell(
                Rect::new(0.0, 0.0, 100.0, 20.0),
                CellBorders::default(),
                Vec::new(),
            );
            cell.shading = Some(shading);
            cell
        };
        let rgb = |value| Some(OoxmlColor::Rgb(value));

        let clear = shaded(Shading {
            val: ShadingPattern::Clear,
            color: Some(OoxmlColor::Auto),
            fill: rgb(0xDE_EA_F6),
        });
        let filled = rects(&draw(&[clear]));
        assert_eq!(filled.len(), 1, "заливка — прямоугольник ячейки");
        let (x, y, w, h, color) = filled[0];
        assert_px(x, 0.0, "левый край заливки");
        assert_px(y, 0.0, "верхний край заливки");
        assert_px(w, 100.0, "ширина заливки");
        assert_px(h, 20.0, "высота заливки");
        assert_eq!(color, Color(0xDEEA_F6FF), "цвет заливки из `w:fill`");

        // У `solid` фон закрыт узором: красит цвет узора, а не фон.
        let solid = shaded(Shading {
            val: ShadingPattern::Solid,
            color: rgb(0xFF_00_00),
            fill: rgb(0xDE_EA_F6),
        });
        assert_eq!(rects(&draw(&[solid]))[0].4, Color(0xFF00_00FF));

        for shading in [
            Shading {
                val: ShadingPattern::Nil,
                color: None,
                fill: rgb(0xDE_EA_F6),
            },
            Shading {
                val: ShadingPattern::Clear,
                color: None,
                fill: Some(OoxmlColor::Auto),
            },
            Shading {
                val: ShadingPattern::Clear,
                color: None,
                fill: None,
            },
        ] {
            assert!(
                rects(&draw(&[shaded(shading)])).is_empty(),
                "заливать нечем — заливки нет"
            );
        }
    }

    /// Продолжение вертикального объединения: заливка и границы есть, а
    /// содержимого нет — оно осталось в начале объединения.
    #[test]
    fn a_merge_continuation_draws_geometry_but_no_content() {
        let inner = |x: f32, y: f32| {
            cell(
                Rect::new(x, y, 40.0, 10.0),
                sides(
                    Some(border(BorderStyle::Single, 4, OoxmlColor::Rgb(0x00_00_00))),
                    Some(border(BorderStyle::Single, 4, OoxmlColor::Rgb(0x00_00_00))),
                    Some(border(BorderStyle::Single, 4, OoxmlColor::Rgb(0x00_00_00))),
                    Some(border(BorderStyle::Single, 4, OoxmlColor::Rgb(0x00_00_00))),
                ),
                Vec::new(),
            )
        };
        let nested = |x: f32, y: f32| LayoutItem::Table {
            node_id: NodeId::new(1),
            rect: Rect::new(x, y, 40.0, 10.0),
            rows: Vec::new(),
            cells: vec![inner(x, y)],
        };

        let mut cell = cell(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            CellBorders::default(),
            vec![nested(10.0, 10.0)],
        );
        assert_eq!(lines(&draw(&[cell.clone()])).len(), 4, "вложенная таблица");

        cell.v_merge_continue = true;
        assert!(
            lines(&draw(&[cell])).is_empty(),
            "содержимое продолжения не рисуется"
        );
    }
}
