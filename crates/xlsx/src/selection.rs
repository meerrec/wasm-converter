//! Модельные операции выделения: объединения, видимые адреса и навигация.
//!
//! Состояние ввода живёт на main, а здесь остаются только правила, для которых
//! нужна книга. Диапазоны не разворачиваются в списки ячеек: полный лист —
//! тоже один прямоугольник.

use std::collections::{HashSet, VecDeque};

use crate::cellref::{CellRef, Range, MAX_COL, MAX_ROW};
use crate::layout::SheetLayout;
use crate::model::{CellValue, Sheet};
use crate::paint::{COL_HEADER_HEIGHT, ROW_HEADER_WIDTH};
use crate::Viewport;

/// Снимок выделения без привязки к листу.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Активная ячейка — левая верхняя у объединения.
    pub active: CellRef,
    /// Нормализованные диапазоны в порядке добавления.
    pub ranges: Vec<Range>,
}

/// Команда навигации, результат которой зависит от модели книги.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationCommand {
    Left,
    Right,
    Up,
    Down,
    CtrlLeft,
    CtrlRight,
    CtrlUp,
    CtrlDown,
    Home,
    CtrlHome,
    CtrlEnd,
    StepLeft(u32),
    StepRight(u32),
    StepUp(u32),
    StepDown(u32),
}

/// Прямоугольник ячейки в пикселях раскладки, без зума и DPR.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Есть ли у ячейки содержимое для навигации по областям данных.
fn has_content(sheet: &Sheet, at: CellRef) -> bool {
    sheet
        .cells
        .cell(at)
        .is_some_and(|cell| !matches!(cell.value, CellValue::Empty) || cell.formula.is_some())
}

/// Привести диапазон к прямоугольнику с учётом всех задетых объединений.
///
/// Расширение повторяется: одно объединение может подтянуть следующее.
#[must_use]
pub fn expand_merges(sheet: &Sheet, range: Range) -> Range {
    let mut result = range.normalized();
    loop {
        let mut grown = false;
        for merge in sheet.merges.ranges() {
            let overlaps = result.first.row <= merge.last.row
                && merge.first.row <= result.last.row
                && result.first.col <= merge.last.col
                && merge.first.col <= result.last.col;
            if overlaps {
                let wider = Range {
                    first: CellRef::new(
                        result.first.row.min(merge.first.row),
                        result.first.col.min(merge.first.col),
                    ),
                    last: CellRef::new(
                        result.last.row.max(merge.last.row),
                        result.last.col.max(merge.last.col),
                    ),
                };
                grown |= wider != result;
                result = wider;
            }
        }
        if !grown {
            return result;
        }
    }
}

/// Нормализовать активную ячейку и все диапазоны по модели листа.
#[must_use]
pub fn normalize(sheet: &Sheet, selection: &Selection) -> Selection {
    let active = sheet
        .merges
        .covering(selection.active)
        .map_or(selection.active, |merge| merge.normalized().first);
    Selection {
        active,
        ranges: selection
            .ranges
            .iter()
            .map(|range| expand_merges(sheet, *range))
            .collect(),
    }
}

/// Сдвинуть индекс на один шаг без выхода за адресное пространство.
fn offset(value: u32, delta: i32, maximum: u32) -> Option<u32> {
    match delta {
        1 if value < maximum => Some(value + 1),
        -1 => value.checked_sub(1),
        0 => Some(value),
        _ => None,
    }
}

/// Следующий видимый адрес на одной оси; скрытые позиции пропускаются.
fn visible_step(sheet: &Sheet, at: CellRef, row_delta: i32, col_delta: i32) -> Option<CellRef> {
    let mut candidate = CellRef::new(
        offset(at.row, row_delta, MAX_ROW)?,
        offset(at.col, col_delta, MAX_COL)?,
    );
    loop {
        if !sheet.dims.rows.is_hidden(candidate.row) && !sheet.dims.cols.is_hidden(candidate.col) {
            return Some(candidate);
        }
        candidate = CellRef::new(
            offset(candidate.row, row_delta, MAX_ROW)?,
            offset(candidate.col, col_delta, MAX_COL)?,
        );
    }
}

/// Ближайшая видимая ячейка в обратном направлении — для края используемого
/// диапазона, который сам может оказаться скрытым.
fn visible_back(sheet: &Sheet, at: CellRef, vertical: bool, horizontal: bool) -> CellRef {
    let mut result = at;
    loop {
        let row_hidden = vertical && sheet.dims.rows.is_hidden(result.row);
        let col_hidden = horizontal && sheet.dims.cols.is_hidden(result.col);
        if !row_hidden && !col_hidden {
            return result;
        }
        if row_hidden {
            if result.row == 0 {
                return at;
            }
            result.row -= 1;
        }
        if col_hidden {
            if result.col == 0 {
                return at;
            }
            result.col -= 1;
        }
    }
}

/// Содержимое отдельной CSR-ячейки, без обращения к ней по адресу.
fn cell_has_content(cell: &crate::Cell) -> bool {
    !matches!(cell.value, CellValue::Empty) || cell.formula.is_some()
}

/// Горизонтальный Ctrl-переход по CSR-строке.
fn jump_horizontal(sheet: &Sheet, at: CellRef, right: bool) -> CellRef {
    let boundary = if right { MAX_COL } else { 0 };
    let Some(next_col) = offset(at.col, if right { 1 } else { -1 }, MAX_COL) else {
        return at;
    };
    let cells = sheet.cells.cells_of_row(at.row);
    let step = if right { 1 } else { -1 };
    let next = CellRef::new(at.row, next_col);
    let continue_block = has_content(sheet, at) && has_content(sheet, next);

    let target = if continue_block {
        let mut target = next.col;
        while let Some(following) = offset(target, step, MAX_COL) {
            let has_following = cells
                .binary_search_by_key(&following, |cell| cell.col)
                .ok()
                .and_then(|index| cells.get(index))
                .is_some_and(cell_has_content);
            if !has_following {
                break;
            }
            target = following;
        }
        target
    } else {
        let found = if right {
            cells
                .iter()
                .filter(|cell| cell.col > at.col)
                .find(|cell| cell_has_content(cell))
        } else {
            cells
                .iter()
                .rev()
                .filter(|cell| cell.col < at.col)
                .find(|cell| cell_has_content(cell))
        };
        found.map_or(boundary, |cell| cell.col)
    };

    let target = CellRef::new(at.row, target);
    if sheet.dims.cols.is_hidden(target.col) {
        visible_step(sheet, at, 0, step).unwrap_or(at)
    } else {
        target
    }
}

/// Вертикальный Ctrl-переход; строки ищутся в CSR бинарным поиском.
fn jump_vertical(sheet: &Sheet, at: CellRef, down: bool) -> CellRef {
    let boundary = if down { MAX_ROW } else { 0 };
    let Some(next_row) = offset(at.row, if down { 1 } else { -1 }, MAX_ROW) else {
        return at;
    };
    let step = if down { 1 } else { -1 };
    let next = CellRef::new(next_row, at.col);
    let continue_block = has_content(sheet, at) && has_content(sheet, next);

    let target = if continue_block {
        let mut target = next.row;
        while let Some(following) = offset(target, step, MAX_ROW) {
            if !has_content(sheet, CellRef::new(following, at.col)) {
                break;
            }
            target = following;
        }
        target
    } else {
        sheet
            .cells
            .next_row_with_content(at.col, at.row, down)
            .unwrap_or(boundary)
    };

    let target = CellRef::new(target, at.col);
    if sheet.dims.rows.is_hidden(target.row) {
        visible_step(sheet, at, step, 0).unwrap_or(at)
    } else {
        target
    }
}

/// Несколько видимых шагов для PageUp/PageDown; внешний ввод ограничен,
/// чтобы ошибочный запрос не превратился в длинный цикл воркера.
fn visible_steps(sheet: &Sheet, active: CellRef, row: i32, col: i32, count: u32) -> CellRef {
    let mut target = active;
    for _ in 0..count.min(1_000) {
        let Some(next) = visible_step(sheet, target, row, col) else {
            break;
        };
        target = next;
    }
    target
}

/// Переместить активную ячейку на один видимый экран по фактической раскладке.
///
/// Высоты строк и ширины столбцов берутся из модели, поэтому нестандартные
/// размеры и закреплённые полосы не заменяются «средней» оценкой на main.
#[must_use]
pub fn page_navigate(
    sheet: &Sheet,
    active: CellRef,
    viewport: Viewport,
    show_headers: bool,
    horizontal: bool,
    backward: bool,
) -> CellRef {
    let layout = SheetLayout::full(sheet);
    let scale = if viewport.scale > 0.0 {
        viewport.scale
    } else {
        1.0
    };
    let pane = sheet.view.pane.filter(|pane| !pane.is_empty());
    let (frozen_rows, frozen_cols) = pane.map_or((0, 0), |pane| (pane.rows, pane.cols));
    let frozen_w = layout.column_x(frozen_cols) * scale;
    let frozen_h = layout.row_y(frozen_rows) * scale;
    let header_w = if show_headers {
        ROW_HEADER_WIDTH * scale
    } else {
        0.0
    };
    let header_h = if show_headers {
        COL_HEADER_HEIGHT * scale
    } else {
        0.0
    };

    if horizontal {
        let view_left = layout.column_x(frozen_cols) + viewport.x.max(0.0);
        let visible_w = (viewport.w - header_w - frozen_w).max(0.0) / scale;
        let (first, last) = layout.columns_in(view_left, view_left + visible_w);
        let count = last.saturating_sub(first).max(1);
        let delta = if backward { -1 } else { 1 };
        visible_steps(sheet, active, 0, delta, count)
    } else {
        let view_top = layout.row_y(frozen_rows) + viewport.y.max(0.0);
        let visible_h = (viewport.h - header_h - frozen_h).max(0.0) / scale;
        let (first, last) = layout.rows_in(view_top, view_top + visible_h);
        let count = last.saturating_sub(first).max(1);
        let delta = if backward { -1 } else { 1 };
        visible_steps(sheet, active, delta, 0, count)
    }
}

/// Переместить активную ячейку по команде навигации.
#[must_use]
pub fn navigate(sheet: &Sheet, active: CellRef, command: NavigationCommand) -> CellRef {
    let step = match command {
        NavigationCommand::StepLeft(count) => {
            return visible_steps(sheet, active, 0, -1, count);
        }
        NavigationCommand::StepRight(count) => {
            return visible_steps(sheet, active, 0, 1, count);
        }
        NavigationCommand::StepUp(count) => {
            return visible_steps(sheet, active, -1, 0, count);
        }
        NavigationCommand::StepDown(count) => {
            return visible_steps(sheet, active, 1, 0, count);
        }
        NavigationCommand::Left => Some((0, -1)),
        NavigationCommand::Right => Some((0, 1)),
        NavigationCommand::Up => Some((-1, 0)),
        NavigationCommand::Down => Some((1, 0)),
        _ => None,
    };
    if let Some((row_delta, col_delta)) = step {
        // Из объединения выходим за его ближайшую границу, иначе нормализация
        // вернёт следующий шаг обратно к якорю и стрелка застрянет.
        let origin = sheet.merges.covering(active).map_or(active, |merge| {
            let merge = merge.normalized();
            CellRef::new(
                if row_delta > 0 {
                    merge.last.row
                } else {
                    merge.first.row
                },
                if col_delta > 0 {
                    merge.last.col
                } else {
                    merge.first.col
                },
            )
        });
        return visible_step(sheet, origin, row_delta, col_delta).unwrap_or(active);
    }

    match command {
        NavigationCommand::CtrlLeft => jump_horizontal(sheet, active, false),
        NavigationCommand::CtrlRight => jump_horizontal(sheet, active, true),
        NavigationCommand::CtrlUp => jump_vertical(sheet, active, false),
        NavigationCommand::CtrlDown => jump_vertical(sheet, active, true),
        NavigationCommand::Home => visible_back(sheet, CellRef::new(active.row, 0), false, true),
        NavigationCommand::CtrlHome => visible_back(sheet, CellRef::new(0, 0), true, true),
        NavigationCommand::CtrlEnd => sheet.visible_used_range().map_or(active, |range| {
            visible_back(sheet, range.normalized().last, true, true)
        }),
        NavigationCommand::Left
        | NavigationCommand::Right
        | NavigationCommand::Up
        | NavigationCommand::Down
        | NavigationCommand::StepLeft(_)
        | NavigationCommand::StepRight(_)
        | NavigationCommand::StepUp(_)
        | NavigationCommand::StepDown(_) => active,
    }
}

/// Текущая область данных вокруг активной ячейки для Ctrl+A.
///
/// Обход идёт только по связанным непустым ячейкам; пустой активный адрес
/// области не имеет. Пустые строки и столбцы естественно разрывают область.
#[must_use]
pub fn current_region(sheet: &Sheet, active: CellRef) -> Option<Range> {
    if !has_content(sheet, active) {
        return None;
    }
    let mut queue = VecDeque::from([active]);
    let mut visited = HashSet::from([active]);
    let mut first = active;
    let mut last = active;

    while let Some(at) = queue.pop_front() {
        first.row = first.row.min(at.row);
        first.col = first.col.min(at.col);
        last.row = last.row.max(at.row);
        last.col = last.col.max(at.col);
        let neighbours = [
            at.row.checked_sub(1).map(|row| CellRef::new(row, at.col)),
            (at.row < MAX_ROW).then(|| CellRef::new(at.row + 1, at.col)),
            at.col.checked_sub(1).map(|col| CellRef::new(at.row, col)),
            (at.col < MAX_COL).then(|| CellRef::new(at.row, at.col + 1)),
        ];
        for next in neighbours.into_iter().flatten() {
            if has_content(sheet, next) && visited.insert(next) {
                queue.push_back(next);
            }
        }
    }
    Some(Range { first, last })
}

/// Границы ячейки или её объединения в полной раскладке листа.
#[must_use]
pub fn cell_rect(sheet: &Sheet, active: CellRef) -> CellRect {
    let range = sheet.merges.covering(active).map_or_else(
        || Range {
            first: active,
            last: active,
        },
        Range::normalized,
    );
    let layout = SheetLayout::full(sheet);
    let x = layout.column_x(range.first.col);
    let y = layout.row_y(range.first.row);
    CellRect {
        x,
        y,
        width: layout.column_x(range.last.col + 1) - x,
        height: layout.row_y(range.last.row + 1) - y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, SheetContent, WorksheetBuilder, WorksheetMeta};
    use crate::{CellValue, SheetState};

    fn sheet(rows: &[(u32, &[u32])]) -> Sheet {
        let mut builder = WorksheetBuilder::new("xl/worksheets/sheet1.xml");
        for (row, cols) in rows {
            for col in *cols {
                builder
                    .push(*row, Cell::new(*col, 0, CellValue::Number(1.0)))
                    .unwrap();
            }
        }
        let mut content = SheetContent {
            cells: builder.finish(),
            ..SheetContent::default()
        };
        content.merges.push(Range::parse("B2:C3").unwrap());
        Sheet::new(
            WorksheetMeta {
                name: "Лист1".to_owned(),
                part: "xl/worksheets/sheet1.xml".to_owned(),
                state: SheetState::Visible,
            },
            content,
        )
    }

    #[test]
    fn normalization_expands_merges_and_moves_active_to_anchor() {
        let book = sheet(&[(1, &[1])]);
        let selection = Selection {
            active: CellRef::new(2, 2),
            ranges: vec![Range::parse("C3:D4").unwrap()],
        };
        let normalized = normalize(&book, &selection);
        assert_eq!(normalized.active, CellRef::new(1, 1));
        assert_eq!(normalized.ranges[0], Range::parse("B2:D4").unwrap());
    }

    #[test]
    fn simple_navigation_skips_hidden_cells() {
        let mut book = sheet(&[(0, &[0, 2])]);
        book.dims.cols.push(crate::ColWidth {
            first: 1,
            last: 1,
            width: 8.43,
            custom: false,
            hidden: true,
            best_fit: false,
        });
        assert_eq!(
            navigate(&book, CellRef::new(0, 0), NavigationCommand::Right),
            CellRef::new(0, 2)
        );
    }

    #[test]
    fn ctrl_navigation_crosses_blocks_and_empty_space() {
        let book = sheet(&[(0, &[1, 2, 3, 7])]);
        assert_eq!(
            navigate(&book, CellRef::new(0, 1), NavigationCommand::CtrlRight),
            CellRef::new(0, 3)
        );
        assert_eq!(
            navigate(&book, CellRef::new(0, 3), NavigationCommand::CtrlRight),
            CellRef::new(0, 7)
        );
        assert_eq!(
            navigate(&book, CellRef::new(0, 7), NavigationCommand::CtrlRight),
            CellRef::new(0, MAX_COL)
        );
    }

    #[test]
    fn ctrl_home_returns_from_the_last_cell_to_a1() {
        let book = sheet(&[]);
        assert_eq!(
            navigate(
                &book,
                CellRef::new(MAX_ROW, MAX_COL),
                NavigationCommand::CtrlHome
            ),
            CellRef::new(0, 0)
        );
    }

    #[test]
    fn page_navigation_uses_the_visible_row_count() {
        let book = sheet(&[]);
        let target = page_navigate(
            &book,
            CellRef::new(0, 0),
            Viewport {
                w: 800.0,
                h: 220.0,
                scale: 1.0,
                ..Viewport::default()
            },
            true,
            false,
            false,
        );
        assert_eq!(target, CellRef::new(9, 0));
    }

    #[test]
    fn arrows_leave_a_merge_and_skip_adjacent_hidden_columns() {
        let mut book = sheet(&[(1, &[1])]);
        let active = CellRef::new(1, 1);
        assert_eq!(
            navigate(&book, active, NavigationCommand::Right),
            CellRef::new(1, 3)
        );
        assert_eq!(
            navigate(&book, active, NavigationCommand::Left),
            CellRef::new(1, 0)
        );
        assert_eq!(
            navigate(&book, active, NavigationCommand::Down),
            CellRef::new(3, 1)
        );
        assert_eq!(
            navigate(&book, active, NavigationCommand::Up),
            CellRef::new(0, 1)
        );
        book.dims.cols.push(crate::ColWidth {
            first: 3,
            last: 3,
            width: 8.43,
            custom: false,
            hidden: true,
            best_fit: false,
        });
        assert_eq!(
            navigate(&book, active, NavigationCommand::Right),
            CellRef::new(1, 4)
        );
    }

    #[test]
    fn ctrl_vertical_navigation_uses_sparse_row_ids() {
        let book = sheet(&[(1_000, &[0])]);
        assert_eq!(
            navigate(&book, CellRef::new(0, 0), NavigationCommand::CtrlDown),
            CellRef::new(1_000, 0)
        );
        let empty = sheet(&[]);
        assert_eq!(
            navigate(&empty, CellRef::new(0, 0), NavigationCommand::CtrlDown),
            CellRef::new(MAX_ROW, 0)
        );
    }

    #[test]
    fn current_region_stops_at_empty_rows_and_columns() {
        let book = sheet(&[(0, &[0, 1]), (1, &[0, 1]), (4, &[4])]);
        assert_eq!(
            current_region(&book, CellRef::new(1, 1)),
            Some(Range::parse("A1:B2").unwrap())
        );
        assert_eq!(current_region(&book, CellRef::new(3, 3)), None);
    }

    #[test]
    fn full_sheet_rect_reaches_the_last_excel_cell() {
        let book = sheet(&[]);
        let rect = cell_rect(&book, CellRef::new(MAX_ROW, MAX_COL));
        assert!(rect.x > 0.0 && rect.y > 0.0);
        assert!(rect.width > 0.0 && rect.height > 0.0);
    }
}
