//! Раскладка таблиц DOCX.
//!
//! Ширины колонок берутся из `w:tblGrid`, а если сетки нет — из `w:tcW` первой
//! строки, затем поровну на число колонок. Готовые ширины масштабируются под
//! `w:tblW` и **сжимаются пропорционально**, если сумма шире полосы набора:
//! таблица обязана помещаться на странице. Отклонение от Word (он допускает
//! выход за правое поле) сделано по требованию заказчика. `w:jc` выравнивает
//! таблицу в полосе, `w:tblInd` отступает слева, упираясь в правую границу.
//!
//! Высота строки, не заданной `w:trHeight`, считается по содержимому: текст
//! ячеек переносится тем же [`LineBreaker`]-ом, что и абзацы тела (ADR-0005),
//! иначе высоты таблицы и страницы разъезжались бы.
//!
//! Таблица переносится между страницами построчно — как в Word с включённым
//! «разрешить перенос строки на следующую страницу». Строка, которой не хватило
//! полосы, режется по границам блоков содержимого (строка текста не рвётся
//! пополам, вложенная таблица уходит целиком) и продолжается на следующей
//! странице; `w:cantSplit` переносит строку целиком, а строка выше полосы
//! набора режется принудительно — иначе раскладка зациклилась бы. Строки
//! `w:tblHeader` повторяются сверху каждой страницы-продолжения. Часть
//! разрезанной строки занимает полосу до её низа (так же в Word нижняя линия
//! строки на разрыве ложится на нижнее поле), поэтому ни один прямоугольник
//! ячейки не выходит за полосу; исключение — строка выше полосы набора, где
//! ячейке деваться некуда.
//!
//! Вложенные таблицы в ячейках раскладываются рекурсивно до [`MAX_TABLE_DEPTH`]
//! уровней; между страницами вложенная таблица не переносится — её строка
//! уезжает на следующую страницу целиком.
//!
//! Границы схлопываются здесь же: собственная граница ячейки (`w:tcBorders`)
//! перекрывает табличную (`w:tblBorders`), внешние рёбра таблицы берут
//! top/left/bottom/right, внутренние — insideH/insideV. Общее ребро соседних
//! ячеек отдают обе ячейки, каждая со своей границей. Заливка (`w:shd`) и
//! выравнивание по вертикали (`w:vAlign`) уезжают в раскладку как есть, чтобы
//! рендеру не пришлось возвращаться к модели. `w:tblStyle` (`w:tblLook`) не
//! разрешается: каскада стилей для таблиц нет. Поля ячейки по умолчанию — 108
//! twips по бокам, как в Word, когда ни `w:tcMar`, ни `w:tblCellMar` не заданы.

use std::ops::Range;

use doc_converter_core::NodeId;
use doc_converter_render::font::{FontId, FontRegistry};

use crate::layout::line_break::LineBreaker;
use crate::model::raw::{Border, BorderStyle, Color, HalfPoint, Justification};
use crate::model::{BlockItem, CellBorders, Inline, Paragraph, RunContent};
use crate::{Cell, CellVAlign, CellWidth, HeightRule, Row, Table, TableWidth, Twips, VMerge};

use super::engine::{LayoutItem, Rect, TableCellLayout, VAlign};

/// Преобразовать twips в пиксели: 1 twip = 1/1440 дюйма, 1 пиксель = 1/96 дюйма.
/// Отношение: (1/1440) / (1/96) = 96/1440 = 1/15.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
fn twips_to_px(twips: Twips) -> f32 {
    twips.value() as f32 / 15.0
}

/// Минимальная высота строки таблицы в пикселях.
///
/// Применяется к готовой высоте строки, а не к `w:trHeight`: там `0.0` означает
/// «считать по содержимому», и пол, наложенный раньше, стёр бы это отличие.
const MIN_ROW_HEIGHT: f32 = 20.0;

/// Кегль по умолчанию — тот же, что у остальной раскладки (12 pt).
///
/// `FontRegistry` не ищет шрифт по имени, поэтому `w:rFonts`/`w:sz` из свойств
/// ячейки и run'ов пока не применяются.
const DEFAULT_SIZE_HALF_POINTS: HalfPoint = HalfPoint::new(24);

/// Поля ячейки по умолчанию по бокам: 108 twips (0.075″) — умолчание Word.
///
/// Берётся, когда ни `w:tcMar`, ни `w:tblCellMar` поле не задают; сверху и
/// снизу умолчание нулевое.
const DEFAULT_SIDE_MARGIN_TWIPS: i32 = 108;

/// Предел вложенности таблиц: глубже вложенная таблица пропускается.
///
/// Уровень 0 — таблица тела, 1 — таблица в её ячейке и так далее. Предел нужен,
/// чтобы битый документ не увёл раскладку в бесконечную рекурсию.
pub const MAX_TABLE_DEPTH: usize = 3;

/// Допуск сравнения высот: `f32` из twips на равенство не проверяем.
const EPS: f32 = 0.01;

/// Полоса страницы, в которую кладётся фрагмент таблицы.
#[derive(Debug, Clone, Copy)]
pub struct PageBand {
    /// Верхняя граница полосы (координата страницы).
    pub top: f32,
    /// Доступная высота: ниже `top + height` ячейки фрагмента не заходят.
    pub height: f32,
}

/// Геометрия таблицы в полосе набора: левый край и ширины колонок.
#[derive(Debug, Clone)]
pub struct TableFrame {
    /// Левый край таблицы в координатах страницы.
    pub x: f32,
    /// Ширины колонок в пикселях.
    pub col_widths: Vec<f32>,
}

impl TableFrame {
    /// Ширина таблицы — сумма ширин колонок.
    #[must_use]
    pub fn width(&self) -> f32 {
        self.col_widths.iter().sum()
    }

    /// Левый край колонки `col`.
    #[must_use]
    pub fn column_x(&self, col: usize) -> f32 {
        self.x + self.col_widths.iter().take(col).sum::<f32>()
    }

    /// Ширина ячейки, начинающейся в колонке `col` и занимающей `span` колонок.
    #[must_use]
    pub fn cell_width(&self, col: usize, span: usize) -> f32 {
        self.col_widths.iter().skip(col).take(span.max(1)).sum()
    }
}

/// Раскладка таблицы на одной странице.
#[derive(Debug, Clone)]
pub struct TableLayoutResult {
    /// Общая высота таблицы.
    pub height: f32,
    /// Ширина таблицы: сумма ширин колонок.
    pub width: f32,
    /// Раскладка строк.
    pub rows: Vec<TableRowLayout>,
}

/// Раскладка одной строки таблицы (или её части на этой странице).
#[derive(Debug, Clone)]
pub struct TableRowLayout {
    /// Высота строки — у разрезанной строки высота её части.
    pub height: f32,
    /// Раскладка ячеек.
    pub cells: Vec<TableCellLayout>,
    /// Строка разрезана: продолжение — на следующей странице.
    pub continues_below: bool,
    /// Строка — продолжение разрезанной: начало осталось на прошлой странице.
    pub continued_above: bool,
    /// Строка повторена как заголовок (`w:tblHeader`) на странице-продолжении.
    pub header_repeat: bool,
}

/// Фрагмент таблицы на одной странице.
#[derive(Debug, Clone)]
pub struct TableFragment {
    /// Верх фрагмента в координатах страницы.
    pub y: f32,
    /// Высота фрагмента — сумма высот его строк.
    pub height: f32,
    /// Фрагмент начинает новую страницу: на текущей полосе он не поместился.
    pub starts_new_page: bool,
    /// Строки фрагмента в порядке отрисовки.
    pub rows: Vec<TableRowLayout>,
}

/// Элемент раскладки для фрагмента: ячейки — в координатах его страницы.
///
/// `cells` — плоский список тех же ячеек, что и в `rows`, в порядке отрисовки:
/// потребителям, которым нужны только рамки и содержимое, не приходится
/// разворачивать строки.
#[must_use]
pub fn table_item(table: &Table, frame: &TableFrame, fragment: TableFragment) -> LayoutItem {
    let cells = fragment
        .rows
        .iter()
        .flat_map(|row| row.cells.iter().cloned())
        .collect();
    LayoutItem::Table {
        node_id: table.id,
        rect: Rect::new(frame.x, fragment.y, frame.width(), fragment.height),
        rows: fragment.rows,
        cells,
    }
}

/// Ширины колонок таблицы в пикселях.
///
/// Порядок источников — как у Word: `w:tblGrid`; если сетки нет или она пустая —
/// `w:tcW` ячеек первой строки (ширина ячейки с `w:gridSpan` делится поровну
/// между её колонками); если и их нет — поровну на число колонок, которое
/// задаёт самая широкая строка. Колонок в сетке может не хватить строке с
/// широкими объединениями — недостающие добираются остатком полосы, чтобы
/// ячейка не оказалась нулевой ширины.
///
/// Готовые ширины масштабируются под `w:tblW` и сжимаются пропорционально, если
/// сумма шире `available_width`: таблица обязана помещаться на странице, хотя
/// Word допускает выход за правое поле.
#[must_use]
pub fn compute_column_widths(table: &Table, available_width: f32) -> Vec<f32> {
    let available = available_width.max(0.0);
    let mut widths = base_column_widths(table, available);

    if widths.is_empty() {
        // Ни сетки, ни `w:tcW`: одна колонка во всю полосу — ширина таблицы
        // без строк ничем не определена.
        return vec![available];
    }

    let needed = column_count(table);
    if widths.len() < needed {
        let extra = needed - widths.len();
        let total: f32 = widths.iter().sum();
        let share = (available - total).max(0.0) / extra as f32;
        widths.resize(needed, share);
    }

    let mut total: f32 = widths.iter().sum();
    if let Some(target) = table_width_px(table, available) {
        if total > 0.0 && target > 0.0 {
            scale_to(&mut widths, target / total);
            total = target;
        }
    }
    if total > available && total > 0.0 {
        scale_to(&mut widths, available / total);
    }

    widths
}

/// Ширины колонок без подгонки под `w:tblW` и полосу.
fn base_column_widths(table: &Table, available: f32) -> Vec<f32> {
    let grid: Vec<f32> = table
        .grid
        .iter()
        .map(|col| twips_to_px(col.width))
        .collect();
    if grid.iter().any(|width| *width > 0.0) {
        return grid;
    }

    let count = column_count(table);
    if count == 0 {
        return Vec::new();
    }

    let mut widths = vec![0.0_f32; count];
    let mut defined = false;
    if let Some(row) = table.rows.first() {
        let mut col = 0usize;
        for cell in &row.cells {
            let span = grid_span(cell);
            if let Some(width) = cell_width_px(cell, available) {
                let share = width / span as f32;
                for width in widths.iter_mut().skip(col).take(span) {
                    *width = share;
                }
                defined = true;
            }
            col += span;
        }
    }

    if defined {
        widths
    } else {
        // Строка есть, а ширин в ней нет: колонки поровну.
        vec![available / count as f32; count]
    }
}

/// Ширина ячейки из `w:tcW`; `nil` и отсутствие — `None`.
fn cell_width_px(cell: &Cell, available: f32) -> Option<f32> {
    match cell.width.as_ref()? {
        CellWidth::Dxa(twips) => Some(twips_to_px(*twips)),
        CellWidth::Pct(pct) => Some(available * pct / 100.0),
        CellWidth::Nil => None,
    }
}

/// Ширина таблицы из `w:tblW`; `auto` и отсутствие — `None`.
fn table_width_px(table: &Table, available: f32) -> Option<f32> {
    match table.width.as_ref()? {
        TableWidth::Dxa(twips) => Some(twips_to_px(*twips)),
        TableWidth::Pct(pct) => Some(available * pct / 100.0),
        TableWidth::Auto => None,
    }
}

/// Число колонок сетки, которое требует самая широкая строка таблицы.
fn column_count(table: &Table) -> usize {
    table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(grid_span).sum::<usize>())
        .max()
        .unwrap_or(0)
}

/// Число колонок, которое занимает ячейка: `w:gridSpan`, но не меньше одной.
fn grid_span(cell: &Cell) -> usize {
    cell.grid_span.max(1) as usize
}

/// Умножить ширины колонок на коэффициент.
fn scale_to(widths: &mut [f32], factor: f32) {
    for width in widths.iter_mut() {
        *width *= factor;
    }
}

/// Геометрия таблицы в полосе набора: ширины колонок и левый край с учётом
/// `w:jc` и `w:tblInd`.
#[must_use]
pub fn compute_table_frame(table: &Table, x: f32, available_width: f32) -> TableFrame {
    let col_widths = compute_column_widths(table, available_width);
    let width: f32 = col_widths.iter().sum();
    let offset = table_offset(table, available_width.max(0.0), width);
    TableFrame {
        x: x + offset,
        col_widths,
    }
}

/// Сдвиг таблицы в полосе: `w:jc` выравнивает, `w:tblInd` отступает слева.
///
/// Итог не выводит таблицу за правую границу полосы: `w:tblInd` сверх
/// свободного места упирается в границу, а не выносит таблицу за поле.
fn table_offset(table: &Table, available: f32, table_width: f32) -> f32 {
    let free = (available - table_width).max(0.0);
    let aligned = match table.jc {
        Some(Justification::Center) => free / 2.0,
        Some(Justification::Right | Justification::End) => free,
        _ => 0.0,
    };
    let indent = table.indent.map_or(0.0, twips_to_px).max(0.0);
    (aligned + indent).min(free)
}

/// Вычислить заданные высоты строк таблицы.
///
/// # Arguments
/// * `rows` - строки таблицы
///
/// # Returns
/// Вектор высот строк в пикселях. `0.0` означает «высота по содержимому»
/// (`w:trHeight` отсутствует или `w:hRule="auto"`): содержимого эта функция не
/// видит, высоту считает [`measure_row_heights`], там же применяется и минимум
/// [`MIN_ROW_HEIGHT`] (приватен, поэтому без ссылки).
#[must_use]
pub fn compute_row_heights(rows: &[Row]) -> Vec<f32> {
    rows.iter()
        .map(|row| match row.height.as_ref() {
            Some(row_height) => match row_height.rule {
                HeightRule::Exact | HeightRule::AtLeast => twips_to_px(row_height.value),
                HeightRule::Auto => 0.0,
            },
            None => 0.0,
        })
        .collect()
}

/// Высоты строк таблицы: `w:trHeight` либо по содержимому, не ниже [`MIN_ROW_HEIGHT`].
///
/// `w:hRule="exact"` держит высоту ровно заданной (содержимое может её
/// перерасти — так же в Word), `atLeast` — не ниже её, `auto` и отсутствие
/// `w:trHeight` — по самой высокой ячейке.
#[must_use]
pub fn measure_row_heights(
    table: &Table,
    col_widths: &[f32],
    fonts: &mut FontRegistry,
) -> Vec<f32> {
    let frame = TableFrame {
        x: 0.0,
        col_widths: col_widths.to_vec(),
    };
    measure_rows(table, &frame, fonts, MAX_TABLE_DEPTH)
        .iter()
        .map(|row| row.height)
        .collect()
}

/// Разложить таблицу целиком от `y`, без переноса на другие страницы.
///
/// Так ложатся вложенные таблицы и так же — таблица, поместившаяся на текущей
/// полосе. Переносом занимается [`layout_table_fragments`].
///
/// # Arguments
/// * `table` - таблица для раскладки
/// * `x` - позиция X (сдвиг `w:jc`/`w:tblInd` считается внутри `available_width`)
/// * `y` - позиция Y
/// * `available_width` - доступная ширина
/// * `fonts` - реестр шрифтов для измерения текста ячеек
///
/// # Returns
/// Результат раскладки таблицы. `height` — сумма высот строк.
#[must_use]
pub fn layout_table(
    table: &Table,
    x: f32,
    y: f32,
    available_width: f32,
    fonts: &mut FontRegistry,
) -> TableLayoutResult {
    let frame = compute_table_frame(table, x, available_width);
    let measured = measure_rows(table, &frame, fonts, MAX_TABLE_DEPTH);
    layout_whole(table, &frame, &measured, y, fonts, MAX_TABLE_DEPTH)
}

/// Разложить таблицу по страницам: строки режутся полосой набора.
///
/// Первая полоса — остаток текущей страницы (`first`), каждая следующая —
/// полоса продолжения (`next`): геометрия продолжений одна и та же, потому что
/// перенос таблицы не меняет секцию. Фрагменты возвращаются в порядке страниц;
/// пустой первый фрагмент (таблица не влезла в остаток страницы) не отдаётся
/// вовсе, а следующий за ним начинается с новой страницы.
///
/// # Arguments
/// * `table` - таблица для раскладки
/// * `frame` - геометрия таблицы в полосе (см. [`compute_table_frame`])
/// * `first` - полоса на текущей странице
/// * `next` - полоса на странице-продолжении
/// * `fonts` - реестр шрифтов для измерения текста ячеек
///
/// # Returns
/// Фрагменты таблицы в порядке страниц. У разрезанной строки
/// [`TableRowLayout::continues_below`] отмечает разрез, а `w:tblHeader`
/// повторяется сверху каждой страницы-продолжения.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn layout_table_fragments(
    table: &Table,
    frame: &TableFrame,
    first: PageBand,
    next: PageBand,
    fonts: &mut FontRegistry,
) -> Vec<TableFragment> {
    let measured = measure_rows(table, frame, fonts, MAX_TABLE_DEPTH);
    let header_rows = header_row_count(table);
    let mut fragments = Vec::new();

    let mut row = 0usize;
    // С какого блока продолжается разрезанная строка — по одной записи на ячейку.
    let mut start: Vec<usize> = Vec::new();
    let mut partial = false;
    let mut band = first;
    let mut starts_new_page = false;

    loop {
        let mut pieces: Vec<RowPiece> = Vec::new();
        let mut cursor = band.top;
        let mut avail = band.height.max(0.0);
        let mut body = 0usize;

        // Шапка повторяется сверху страницы-продолжения: строки `w:tblHeader`
        // идут первыми в таблице, поэтому `header < row` — тело уже началось.
        if starts_new_page && header_rows > 0 && row > 0 {
            for header in 0..header_rows.min(row) {
                if let Some(measure) = measured.get(header) {
                    let height = measure.height;
                    pieces.push(whole_piece(&measured, header, cursor, height, true));
                    cursor += height;
                    avail = (avail - height).max(0.0);
                }
            }
        }

        loop {
            let Some(measure) = measured.get(row) else {
                break;
            };
            let remainder = if partial {
                row_rest_height(measure, &start)
            } else {
                measure.height
            };

            if remainder <= avail + EPS {
                pieces.push(RowPiece {
                    row,
                    y: cursor,
                    height: remainder,
                    parts: ranges_from(measure, &start),
                    continued_above: partial,
                    continues_below: false,
                    header_repeat: false,
                });
                cursor += remainder;
                avail = (avail - remainder).max(0.0);
                body += 1;
                row += 1;
                start.clear();
                partial = false;
                continue;
            }

            let cant_split = table.rows.get(row).is_some_and(|row| row.cant_split);
            if !cant_split {
                if let Some(parts) = fit_blocks(measure, &start, partial, avail) {
                    let done = exhausted(measure, &parts);
                    pieces.push(RowPiece {
                        row,
                        y: cursor,
                        height: avail,
                        parts,
                        continued_above: partial,
                        continues_below: !done,
                        header_repeat: false,
                    });
                    if done {
                        row += 1;
                        start.clear();
                        partial = false;
                    } else {
                        start = ends(&pieces[pieces.len() - 1].parts);
                        partial = true;
                    }
                    break;
                }
            }

            // Строка выше полосы набора: режем принудительно, иначе раскладка
            // не сдвинулась бы с места. Первой полосе этот случай не отдаётся —
            // её остаток может быть крошечным, и строка уходит на следующую.
            if body == 0 && starts_new_page {
                if let Some((parts, height)) = force_blocks(measure, &start, partial) {
                    let done = exhausted(measure, &parts);
                    pieces.push(RowPiece {
                        row,
                        y: cursor,
                        height,
                        parts,
                        continued_above: partial,
                        continues_below: !done,
                        header_repeat: false,
                    });
                    if done {
                        row += 1;
                        start.clear();
                        partial = false;
                    } else {
                        start = ends(&pieces[pieces.len() - 1].parts);
                        partial = true;
                    }
                    break;
                }
                // Блоков не осталось, а высота есть: отдаём остаток целиком —
                // иначе строка из пустых ячеек зацикливалась бы.
                pieces.push(RowPiece {
                    row,
                    y: cursor,
                    height: remainder,
                    parts: ranges_from(measure, &start),
                    continued_above: partial,
                    continues_below: false,
                    header_repeat: false,
                });
                row += 1;
                start.clear();
                partial = false;
                break;
            }

            // Не помещается и резать нечего: строка уходит на следующую страницу.
            break;
        }

        if !pieces.is_empty() {
            fragments.push(TableFragment {
                y: band.top,
                height: pieces.iter().map(|piece| piece.height).sum(),
                starts_new_page,
                rows: layout_pieces(table, frame, &pieces, &measured, fonts, MAX_TABLE_DEPTH),
            });
        }
        if row >= table.rows.len() {
            break;
        }
        band = next;
        starts_new_page = true;
    }

    fragments
}

/// Разложить таблицу целиком от `y` — путь целой и вложенной таблицы.
fn layout_whole(
    table: &Table,
    frame: &TableFrame,
    measured: &[MeasuredRow<'_>],
    y: f32,
    fonts: &mut FontRegistry,
    depth: usize,
) -> TableLayoutResult {
    let mut pieces = Vec::with_capacity(measured.len());
    let mut cursor = y;
    for (row, measure) in measured.iter().enumerate() {
        let height = measure.height;
        pieces.push(RowPiece {
            row,
            y: cursor,
            height,
            parts: ranges_from(measure, &vec![0; measure.cells.len()]),
            continued_above: false,
            continues_below: false,
            header_repeat: false,
        });
        cursor += height;
    }

    let height = cursor - y;
    TableLayoutResult {
        height,
        width: frame.width(),
        rows: layout_pieces(table, frame, &pieces, measured, fonts, depth),
    }
}

/// Разложить вложенную таблицу и вернуть её элемент раскладки вместе с высотой.
fn layout_nested(
    table: &Table,
    x: f32,
    y: f32,
    width: f32,
    fonts: &mut FontRegistry,
    depth: usize,
) -> (LayoutItem, f32) {
    let frame = compute_table_frame(table, x, width);
    let measured = measure_rows(table, &frame, fonts, depth);
    let result = layout_whole(table, &frame, &measured, y, fonts, depth);
    let fragment = TableFragment {
        y,
        height: result.height,
        starts_new_page: false,
        rows: result.rows,
    };
    (table_item(table, &frame, fragment), result.height)
}

/// Число строк заголовка — ведущие строки с `w:tblHeader`.
///
/// Word повторяет только непрерывный блок шапки сверху таблицы, поэтому
/// `w:tblHeader` в середине ни на что не влияет.
fn header_row_count(table: &Table) -> usize {
    table.rows.iter().take_while(|row| row.header).count()
}

/// Часть строки, занимающая её целиком.
fn whole_piece(
    measured: &[MeasuredRow<'_>],
    row: usize,
    y: f32,
    height: f32,
    header_repeat: bool,
) -> RowPiece {
    let parts = measured.get(row).map_or_else(Vec::new, |measure| {
        measure
            .cells
            .iter()
            .map(|cell| 0..cell.blocks.len())
            .collect()
    });
    RowPiece {
        row,
        y,
        height,
        parts,
        continued_above: false,
        continues_below: false,
        header_repeat,
    }
}

/// Часть строки на одной полосе.
#[derive(Debug, Clone)]
struct RowPiece {
    /// Индекс строки в `table.rows`.
    row: usize,
    /// Верх части в координатах страницы.
    y: f32,
    /// Высота части.
    height: f32,
    /// Диапазоны блоков, попавшие в часть — по одной записи на ячейку.
    parts: Vec<Range<usize>>,
    /// Строка — продолжение разрезанной.
    continued_above: bool,
    /// Строка разрезана: продолжение на следующей полосе.
    continues_below: bool,
    /// Строка повторена как заголовок.
    header_repeat: bool,
}

/// Диапазоны блоков от `start` до конца содержимого каждой ячейки.
fn ranges_from(measure: &MeasuredRow<'_>, start: &[usize]) -> Vec<Range<usize>> {
    measure
        .cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let from = start
                .get(index)
                .copied()
                .unwrap_or(0)
                .min(cell.blocks.len());
            from..cell.blocks.len()
        })
        .collect()
}

/// Где начинать блоки после части — конец каждого её диапазона.
fn ends(parts: &[Range<usize>]) -> Vec<usize> {
    parts.iter().map(|part| part.end).collect()
}

/// Все блоки строки разложены.
fn exhausted(measure: &MeasuredRow<'_>, parts: &[Range<usize>]) -> bool {
    measure.cells.iter().enumerate().all(|(index, cell)| {
        parts
            .get(index)
            .is_none_or(|part| part.end >= cell.blocks.len())
    })
}

/// Высота остатка строки, начиная с блоков `start`.
///
/// Продолжение строки мерится как отдельная строка: верхнее поле ячейки уже
/// осталось на прошлой странице, а нижнее — её.
fn row_rest_height(measure: &MeasuredRow<'_>, start: &[usize]) -> f32 {
    measure
        .cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let from = start.get(index).copied().unwrap_or(0);
            cell.padding_bottom
                + cell
                    .blocks
                    .iter()
                    .skip(from)
                    .map(MeasuredBlock::height)
                    .sum::<f32>()
        })
        .fold(MIN_ROW_HEIGHT, f32::max)
}

/// Блоки, помещающиеся в `avail`: по одной границе на ячейку.
///
/// `None` — ни одна ячейка не отдала ни одного блока: резать нечего, и Word в
/// этом случае переносит строку целиком.
fn fit_blocks(
    measure: &MeasuredRow<'_>,
    start: &[usize],
    partial: bool,
    avail: f32,
) -> Option<Vec<Range<usize>>> {
    let mut parts = Vec::with_capacity(measure.cells.len());
    let mut any = false;
    for (index, cell) in measure.cells.iter().enumerate() {
        let from = start
            .get(index)
            .copied()
            .unwrap_or(0)
            .min(cell.blocks.len());
        // Верхнее поле ячейки остаётся на первой странице: у продолжения его нет.
        let mut used = if partial { 0.0 } else { cell.padding_top };
        let mut end = from;
        for block in cell.blocks.iter().skip(from) {
            if used + block.height() <= avail + EPS {
                used += block.height();
                end += 1;
            } else {
                break;
            }
        }
        any |= end > from;
        parts.push(from..end);
    }
    any.then_some(parts)
}

/// По одному блоку с каждой ячейки — принудительный разрез строки выше полосы.
///
/// Отдаёт границы блоков и высоту, которая им нужна: она больше полосы, иначе
/// сюда бы не попали.
fn force_blocks(
    measure: &MeasuredRow<'_>,
    start: &[usize],
    partial: bool,
) -> Option<(Vec<Range<usize>>, f32)> {
    let mut parts = Vec::with_capacity(measure.cells.len());
    let mut height = 0.0_f32;
    let mut any = false;
    for (index, cell) in measure.cells.iter().enumerate() {
        let from = start
            .get(index)
            .copied()
            .unwrap_or(0)
            .min(cell.blocks.len());
        let end = if cell.blocks.len() > from {
            from + 1
        } else {
            from
        };
        if end > from {
            any = true;
            let top = if partial { 0.0 } else { cell.padding_top };
            height = height.max(top + cell.blocks[from].height());
        }
        parts.push(from..end);
    }
    any.then_some((parts, height))
}

/// Измеренная строка таблицы: высота и содержимое ячеек без координат.
struct MeasuredRow<'a> {
    /// Высота строки: `w:trHeight` либо по содержимому, не ниже [`MIN_ROW_HEIGHT`].
    height: f32,
    /// Содержимое ячеек — по одной записи на ячейку.
    cells: Vec<MeasuredCell<'a>>,
}

/// Измеренное содержимое ячейки: блоки и поля.
struct MeasuredCell<'a> {
    padding_top: f32,
    padding_bottom: f32,
    padding_left: f32,
    /// Блоки содержимого в порядке следования.
    blocks: Vec<MeasuredBlock<'a>>,
}

impl MeasuredCell<'_> {
    /// Высота содержимого с полями.
    fn height(&self) -> f32 {
        self.padding_top
            + self.padding_bottom
            + self.blocks.iter().map(MeasuredBlock::height).sum::<f32>()
    }
}

/// Блок содержимого ячейки — единица разрезания строки при переносе.
///
/// Строку текста нельзя порвать пополам, а вложенная таблица в этом слайсе не
/// переносится вовсе, поэтому разрез проходит только по границам блоков.
enum MeasuredBlock<'a> {
    /// Строка текста абзаца.
    Line(CellLine),
    /// Вложенная таблица — уходит на следующую страницу целиком.
    Nested {
        table: &'a Table,
        width: f32,
        height: f32,
    },
}

impl MeasuredBlock<'_> {
    /// Высота блока.
    fn height(&self) -> f32 {
        match self {
            Self::Line(line) => line.height,
            Self::Nested { height, .. } => *height,
        }
    }
}

/// Строка текста абзаца ячейки.
struct CellLine {
    node_id: NodeId,
    text: String,
    height: f32,
    color: Option<u32>,
}

/// Измерить строки таблицы: высоты и блоки содержимого.
fn measure_rows<'a>(
    table: &'a Table,
    frame: &TableFrame,
    fonts: &mut FontRegistry,
    depth: usize,
) -> Vec<MeasuredRow<'a>> {
    let declared = compute_row_heights(&table.rows);
    let mut measured = Vec::with_capacity(table.rows.len());

    for (index, row) in table.rows.iter().enumerate() {
        let value = declared.get(index).copied().unwrap_or(0.0);
        let mut cells = Vec::with_capacity(row.cells.len());
        let mut content_height = 0.0_f32;
        let mut col = 0usize;
        for cell in &row.cells {
            let span = grid_span(cell);
            let width = frame.cell_width(col, span);
            col += span;
            let measured_cell = measure_cell(cell, table, width, fonts, depth);
            content_height = content_height.max(measured_cell.height());
            cells.push(measured_cell);
        }

        let height = match row.height.as_ref().map(|height| height.rule) {
            Some(HeightRule::Exact) => value,
            Some(HeightRule::AtLeast) => value.max(content_height),
            _ => content_height,
        };
        measured.push(MeasuredRow {
            height: height.max(MIN_ROW_HEIGHT),
            cells,
        });
    }

    measured
}

/// Измерить содержимое ячейки: блоки строк текста и вложенных таблиц.
fn measure_cell<'a>(
    cell: &'a Cell,
    table: &Table,
    width: f32,
    fonts: &mut FontRegistry,
    depth: usize,
) -> MeasuredCell<'a> {
    let padding = cell_padding(cell, table);
    // Узкая колонка с широкими полями даёт нулевую (и отрицательную) ширину:
    // разбивка на такой ширине возвращает пустой список, высота остаётся
    // конечной — одной строкой на абзац.
    let inner_width = (width - padding.left - padding.right).max(0.0);

    let line_height =
        LineBreaker::new(fonts, FontId::default()).line_height(DEFAULT_SIZE_HALF_POINTS);
    let mut blocks = Vec::new();

    for item in &cell.items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                let text = paragraph_text(paragraph);
                // Разбиватель держит `fonts` заимствованными: его заимствование
                // живёт ровно на время разбивки одного абзаца, чтобы не
                // пересечься с измерением вложенной таблицы ниже.
                let mut ranges = {
                    let mut breaker = LineBreaker::new(fonts, FontId::default());
                    breaker.break_lines(
                        &text,
                        FontId::default(),
                        DEFAULT_SIZE_HALF_POINTS,
                        inner_width,
                    )
                };
                // Пустой абзац — одна строка нулевой ширины, как и в абзацах тела.
                if ranges.is_empty() {
                    ranges.push(0..0);
                }
                let color = paragraph_color(paragraph);
                for range in ranges {
                    blocks.push(MeasuredBlock::Line(CellLine {
                        node_id: paragraph.id,
                        text: text[range].to_owned(),
                        height: line_height,
                        color,
                    }));
                }
            }
            BlockItem::Table(nested) if depth > 0 => {
                let height = nested_table_height(nested, inner_width, fonts, depth - 1);
                blocks.push(MeasuredBlock::Nested {
                    table: nested,
                    width: inner_width,
                    height,
                });
            }
            _ => {}
        }
    }

    MeasuredCell {
        padding_top: padding.top,
        padding_bottom: padding.bottom,
        padding_left: padding.left,
        blocks,
    }
}

/// Высота вложенной таблицы: сумма высот её строк.
fn nested_table_height(table: &Table, width: f32, fonts: &mut FontRegistry, depth: usize) -> f32 {
    let frame = compute_table_frame(table, 0.0, width);
    measure_rows(table, &frame, fonts, depth)
        .iter()
        .map(|row| row.height)
        .sum()
}

/// Поля ячейки в пикселях.
#[derive(Debug, Clone, Copy)]
struct Padding {
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
}

/// Поля ячейки: `w:tcMar`, затем `w:tblCellMar`, затем умолчание Word.
fn cell_padding(cell: &Cell, table: &Table) -> Padding {
    let side = twips_to_px(Twips::new(DEFAULT_SIDE_MARGIN_TWIPS));
    let resolve = |own: Option<Twips>, shared: Option<Twips>, default: f32| {
        own.or(shared).map_or(default, twips_to_px)
    };
    Padding {
        top: resolve(cell.margins.top, table.cell_margins.top, 0.0),
        bottom: resolve(cell.margins.bottom, table.cell_margins.bottom, 0.0),
        left: resolve(cell.margins.left, table.cell_margins.left, side),
        right: resolve(cell.margins.right, table.cell_margins.right, side),
    }
}

/// Текст абзаца: runs, а также содержимое ссылок и результаты полей — по порядку.
fn paragraph_text(paragraph: &Paragraph) -> String {
    let mut text = String::new();
    collect_inline_text(&paragraph.runs, &mut text);
    text
}

/// Собрать текст inline-элементов в `out`, заходя во вложенные runs.
fn collect_inline_text(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Run(run) => {
                for content in &run.content {
                    if let RunContent::Text(text) = content {
                        out.push_str(text);
                    }
                }
            }
            Inline::Hyperlink(link) => collect_inline_text(&link.runs, out),
            Inline::Field(field) => collect_inline_text(&field.result, out),
            _ => {}
        }
    }
}

/// Цвет текста абзаца ячейки — первый явный `w:color` среди его runs.
///
/// Каскад до ячеек пока не доходит, поэтому читается собственный `w:rPr` run'а;
/// `auto` и `none` цвета не задают. Строка несёт один цвет, а run'ы ячейки не
/// разбиваются по метрикам, поэтому из нескольких цветов берётся первый.
fn paragraph_color(paragraph: &Paragraph) -> Option<u32> {
    fn first_color(inlines: &[Inline]) -> Option<u32> {
        for inline in inlines {
            match inline {
                Inline::Run(run) => {
                    if let Some(Color::Rgb(value)) = &run.rpr.color {
                        return Some(*value);
                    }
                }
                Inline::Hyperlink(link) => {
                    if let Some(value) = first_color(&link.runs) {
                        return Some(value);
                    }
                }
                Inline::Field(field) => {
                    if let Some(value) = first_color(&field.result) {
                        return Some(value);
                    }
                }
                _ => {}
            }
        }
        None
    }

    first_color(&paragraph.runs)
}

/// Что даёт ячейке вертикальное объединение.
#[derive(Debug, Clone, Copy)]
enum MergeSlot {
    /// Ячейка не участвует в объединении.
    None,
    /// Начало объединения (`w:vMerge w:val="restart"`): тянется вниз до `bottom`.
    Start {
        /// Низ объединения в пределах этого фрагмента.
        bottom: f32,
    },
    /// Продолжение: содержимое не рисуется (в Word текст живёт в начале
    /// объединения), ячейка тянется вниз до `bottom`.
    Continue {
        /// Низ объединения в пределах этого фрагмента.
        bottom: f32,
    },
}

/// Границы объединения по вертикали: `w:Continue` в той же колонке.
fn v_merge_bounds(table: &Table, row: usize, cell: usize) -> (usize, usize) {
    let mut start = row;
    while start > 0 && is_merge_continue(table, start - 1, cell) {
        start -= 1;
    }
    let mut end = row;
    while is_merge_continue(table, end + 1, cell) {
        end += 1;
    }
    (start, end)
}

/// Ячейка `cell` строки `row` — продолжение объединения.
fn is_merge_continue(table: &Table, row: usize, cell: usize) -> bool {
    table
        .rows
        .get(row)
        .and_then(|row| row.cells.get(cell))
        .is_some_and(|cell| cell.v_merge == Some(VMerge::Continue))
}

/// Что даёт объединение каждой ячейке каждой части: по одной записи на ячейку.
fn merge_slots(table: &Table, pieces: &[RowPiece]) -> Vec<Vec<MergeSlot>> {
    let mut slots: Vec<Vec<MergeSlot>> = pieces
        .iter()
        .map(|piece| {
            vec![MergeSlot::None; table.rows.get(piece.row).map_or(0, |row| row.cells.len())]
        })
        .collect();

    for (index, piece) in pieces.iter().enumerate() {
        let Some(row) = table.rows.get(piece.row) else {
            continue;
        };
        for (cell_index, cell) in row.cells.iter().enumerate() {
            let Some(merge) = cell.v_merge else {
                continue;
            };
            let start = match merge {
                VMerge::Restart => piece.row,
                VMerge::Continue => v_merge_bounds(table, piece.row, cell_index).0,
            };
            let end = v_merge_bounds(table, start, cell_index).1;
            // Объединение обрывается краем страницы: на следующей оно
            // продолжается пустой ячейкой той же ширины.
            let bottom = pieces
                .iter()
                .filter(|other| other.row >= start && other.row <= end)
                .map(|other| other.y + other.height)
                .fold(piece.y + piece.height, f32::max);
            slots[index][cell_index] = match merge {
                VMerge::Restart => MergeSlot::Start { bottom },
                VMerge::Continue => MergeSlot::Continue { bottom },
            };
        }
    }

    slots
}

/// Разложить части строк в готовые строки: ячейки, объединения, границы.
fn layout_pieces(
    table: &Table,
    frame: &TableFrame,
    pieces: &[RowPiece],
    measured: &[MeasuredRow<'_>],
    fonts: &mut FontRegistry,
    depth: usize,
) -> Vec<TableRowLayout> {
    let slots = merge_slots(table, pieces);
    let last_col = frame.col_widths.len().saturating_sub(1);
    let mut rows = Vec::with_capacity(pieces.len());

    for (index, piece) in pieces.iter().enumerate() {
        let Some(row) = table.rows.get(piece.row) else {
            continue;
        };
        let Some(measure) = measured.get(piece.row) else {
            continue;
        };
        let slots_row = slots.get(index).map_or(&[][..], Vec::as_slice);
        let last_row = piece.row + 1 == table.rows.len();

        let mut cells = Vec::with_capacity(row.cells.len());
        let mut col = 0usize;
        for (cell_index, cell) in row.cells.iter().enumerate() {
            let span = grid_span(cell);
            let slot = slots_row
                .get(cell_index)
                .copied()
                .unwrap_or(MergeSlot::None);
            let (height, cut) = match slot {
                MergeSlot::Start { bottom } => (
                    (bottom - piece.y).max(piece.height),
                    CellCut {
                        top: piece.continued_above,
                        bottom: false,
                    },
                ),
                MergeSlot::Continue { bottom } => {
                    ((bottom - piece.y).max(piece.height), CellCut::default())
                }
                MergeSlot::None => (
                    piece.height,
                    CellCut {
                        top: piece.continued_above,
                        bottom: piece.continues_below,
                    },
                ),
            };

            let Some(cell_measure) = measure.cells.get(cell_index) else {
                continue;
            };
            let part = piece
                .parts
                .get(cell_index)
                .cloned()
                .unwrap_or(0..cell_measure.blocks.len());
            let rect = Rect::new(
                frame.column_x(col),
                piece.y,
                frame.cell_width(col, span),
                height,
            );
            // Содержимое продолжения объединения не рисуется: в Word текст
            // объединённой ячейки живёт только в её начале.
            let content = if matches!(slot, MergeSlot::Continue { .. }) {
                Vec::new()
            } else {
                layout_cell_blocks(cell, cell_measure, &part, &rect, fonts, depth)
            };

            cells.push(TableCellLayout {
                rect,
                content,
                v_align: vertical_align(cell),
                shading: cell.shading.clone(),
                borders: Box::new(cell_borders(
                    table, piece.row, col, span, last_col, last_row, cut,
                )),
                v_merge_continue: cell.v_merge == Some(VMerge::Continue),
            });
            col += span;
        }

        rows.push(TableRowLayout {
            height: piece.height,
            cells,
            continues_below: piece.continues_below,
            continued_above: piece.continued_above,
            header_repeat: piece.header_repeat,
        });
    }

    rows
}

/// Разложить блоки ячейки, попавшие в часть строки.
fn layout_cell_blocks(
    cell: &Cell,
    measure: &MeasuredCell<'_>,
    part: &Range<usize>,
    rect: &Rect,
    fonts: &mut FontRegistry,
    depth: usize,
) -> Vec<LayoutItem> {
    let blocks: Vec<&MeasuredBlock<'_>> = measure
        .blocks
        .iter()
        .skip(part.start)
        .take(part.end.saturating_sub(part.start))
        .collect();
    let content_height: f32 = blocks.iter().map(|block| block.height()).sum();

    // Верхнее поле осталось на прошлой странице вместе с началом строки.
    let top = if part.start == 0 {
        measure.padding_top
    } else {
        0.0
    };
    let room = (rect.height - top - measure.padding_bottom).max(0.0);
    let shift = match cell.v_align {
        CellVAlign::Center => ((room - content_height) / 2.0).max(0.0),
        CellVAlign::Bottom => (room - content_height).max(0.0),
        CellVAlign::Top => 0.0,
    };

    let left = rect.x + measure.padding_left;
    let mut cursor = rect.y + top + shift;
    let mut items = Vec::with_capacity(blocks.len());

    for block in blocks {
        match block {
            MeasuredBlock::Line(line) => {
                // Разбиватель держит `fonts` заимствованными; измерение строки
                // заканчивается до ветки вложенной таблицы, которой `fonts`
                // нужен сам.
                let line_width = {
                    let mut breaker = LineBreaker::new(fonts, FontId::default());
                    breaker.measure_text(&line.text, FontId::default(), DEFAULT_SIZE_HALF_POINTS)
                };
                items.push(LayoutItem::Paragraph {
                    node_id: line.node_id,
                    rect: Rect::new(left, cursor, line_width, line.height),
                    text: line.text.clone(),
                    line_height: line.height,
                    color: line.color,
                    // Ячейки не проходят каскад: гарнитура и начертание берутся
                    // не из `w:rPr`, а из умолчания движка.
                    font_family: None,
                    size: DEFAULT_SIZE_HALF_POINTS,
                    bold: false,
                    italic: false,
                    underline: false,
                });
                cursor += line.height;
            }
            MeasuredBlock::Nested {
                table,
                width: nested_width,
                ..
            } => {
                let (item, height) = layout_nested(
                    table,
                    left,
                    cursor,
                    *nested_width,
                    fonts,
                    depth.saturating_sub(1),
                );
                items.push(item);
                cursor += height;
            }
        }
    }

    items
}

/// Как разрез страницы проходит по ячейке: рёбра разреза линию не рисуют.
#[derive(Debug, Clone, Copy, Default)]
struct CellCut {
    /// Верхнее ребро — разрез: строка продолжается с прошлой страницы.
    top: bool,
    /// Нижнее ребро — разрез: строка продолжается на следующей странице.
    bottom: bool,
}

/// Эффективные границы ячейки: собственные поверх табличных.
///
/// Внешние рёбра таблицы берут у неё `top`/`left`/`bottom`/`right`, внутренние —
/// `insideH`/`insideV`. Общее ребро соседних ячеек отдают обе ячейки, каждая со
/// своей границей. `w:tblStyle` не разрешается: каскада стилей для таблиц нет.
#[allow(clippy::too_many_arguments)]
fn cell_borders(
    table: &Table,
    row: usize,
    col: usize,
    span: usize,
    last_col: usize,
    last_row: bool,
    cut: CellCut,
) -> CellBorders {
    let borders = &table.borders;
    let own = table
        .rows
        .get(row)
        .and_then(|row| {
            row.cells.iter().find(|cell| {
                let mut start = 0usize;
                for other in &row.cells {
                    if std::ptr::eq(other, *cell) {
                        break;
                    }
                    start += grid_span(other);
                }
                (start..start + grid_span(cell)).contains(&col)
            })
        })
        .map(|cell| cell.borders.as_ref());

    let top = if cut.top {
        None
    } else {
        pick(
            own.and_then(|cell| cell.top.as_ref()),
            if row == 0 {
                borders.top.as_ref()
            } else {
                borders.inside_h.as_ref()
            },
        )
    };
    let bottom = if cut.bottom {
        None
    } else {
        pick(
            own.and_then(|cell| cell.bottom.as_ref()),
            if last_row {
                borders.bottom.as_ref()
            } else {
                borders.inside_h.as_ref()
            },
        )
    };
    let left = pick(
        own.and_then(|cell| cell.left.as_ref()),
        if col == 0 {
            borders.left.as_ref()
        } else {
            borders.inside_v.as_ref()
        },
    );
    let right = pick(
        own.and_then(|cell| cell.right.as_ref()),
        if col + span > last_col {
            borders.right.as_ref()
        } else {
            borders.inside_v.as_ref()
        },
    );

    CellBorders {
        top,
        left,
        bottom,
        right,
    }
}

/// Граница ячейки поверх табличной; `nil` и `none` линии не рисуют.
fn pick(own: Option<&Border>, shared: Option<&Border>) -> Option<Border> {
    let border = own.or(shared)?;
    match border.val {
        BorderStyle::Nil | BorderStyle::None => None,
        _ => Some(border.clone()),
    }
}

/// Выравнивание содержимого ячейки по вертикали (`w:vAlign`).
fn vertical_align(cell: &Cell) -> VAlign {
    match cell.v_align {
        CellVAlign::Center => VAlign::Center,
        CellVAlign::Bottom => VAlign::Bottom,
        CellVAlign::Top => VAlign::Top,
    }
}
