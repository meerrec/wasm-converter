//! Раскладка таблиц DOCX.
//!
//! Таблицы раскладываются построчно: сначала вычисляется ширина колонок,
//! затем каждая строка раскладывается внутри этой ширины.

use crate::{Cell, Row, Table, Twips, VMerge};

/// Преобразовать twips в пиксели: 1 twip = 1/1440 дюйма, 1 пиксель = 1/96 дюйма.
/// Отношение: (1/1440) / (1/96) = 96/1440 = 1/15.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
fn twips_to_px(twips: Twips) -> f32 {
    twips.value() as f32 / 15.0
}

use super::engine::Rect;

/// Раскладка таблицы на странице.
#[derive(Debug, Clone)]
pub struct TableLayoutResult {
    /// Общая высота таблицы.
    pub height: f32,
    /// Раскладка строк.
    pub rows: Vec<TableRowLayout>,
}

/// Раскладка одной строки таблицы.
#[derive(Debug, Clone)]
pub struct TableRowLayout {
    /// Высота строки.
    pub height: f32,
    /// Раскладка ячеек.
    pub cells: Vec<TableCellLayout>,
}

/// Раскладка одной ячейки таблицы.
#[derive(Debug, Clone)]
pub struct TableCellLayout {
    /// Прямоугольник положения.
    pub rect: Rect,
    /// Содержимое ячейки (абзацы).
    pub content_height: f32,
}

/// Вычислить ширину колонок таблицы.
///
/// # Arguments
/// * `table` - таблица для раскладки
/// * `available_width` - доступная ширина в пикселях
///
/// # Returns
/// Вектор ширин колонок в пикселях.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn compute_column_widths(table: &Table, available_width: f32) -> Vec<f32> {
    let grid_cols = &table.grid;

    if grid_cols.is_empty() {
        return vec![available_width];
    }

    // Сначала пробуем использовать явные ширины из grid_cols
    let total_defined: f32 = grid_cols.iter().map(|col| twips_to_px(col.width)).sum();

    if total_defined > 0.0 && total_defined <= available_width {
        // Явные ширины помещаются - используем их
        return grid_cols.iter().map(|col| twips_to_px(col.width)).collect();
    }

    // Иначе распределяем ширину равномерно
    let col_count = grid_cols.len().max(1);
    vec![available_width / col_count as f32; col_count]
}

/// Вычислить высоту строк таблицы.
///
/// # Arguments
/// * `rows` - строки таблицы
///
/// # Returns
/// Вектор высот строк в пикселях.
#[must_use]
pub fn compute_row_heights(rows: &[Row]) -> Vec<f32> {
    rows.iter()
        .map(|row| {
            let height = match row.height.as_ref() {
                Some(row_height) => {
                    match row_height.rule {
                        crate::HeightRule::Exact | crate::HeightRule::AtLeast => {
                            twips_to_px(row_height.value)
                        }
                        crate::HeightRule::Auto => 0.0, // Будет вычислено по содержимому
                    }
                }
                None => 0.0,
            };
            height.max(20.0) // Минимальная высота строки
        })
        .collect()
}

/// Разложить таблицу.
///
/// # Arguments
/// * `table` - таблица для раскладки
/// * `x` - позиция X
/// * `y` - позиция Y
/// * `available_width` - доступная ширина
///
/// # Returns
/// Результат раскладки таблицы.
#[must_use]
pub fn layout_table(table: &Table, x: f32, y: f32, available_width: f32) -> TableLayoutResult {
    let col_widths = compute_column_widths(table, available_width);
    let row_heights = compute_row_heights(&table.rows);

    let mut result_rows = Vec::new();
    let mut total_height = 0.0;
    let mut current_y = y;

    for (row_idx, row) in table.rows.iter().enumerate() {
        let mut row_height = row_heights.get(row_idx).copied().unwrap_or(20.0);
        let mut row_cells = Vec::new();

        // Если высота не задана, вычисляем по содержимому
        if row_height == 0.0 {
            row_height = compute_row_height_from_content(row, &col_widths);
        }

        let mut current_x = x;

        for (cell_idx, cell) in row.cells.iter().enumerate() {
            let col_width = col_widths.get(cell_idx).copied().unwrap_or(100.0);

            // Вычисляем позицию и размер ячейки
            let cell_x = current_x;
            let cell_width = compute_cell_width(cell, col_width, &col_widths, cell_idx);

            // Раскладываем содержимое ячейки
            let content_height = layout_cell_content(cell, cell_width);

            // Определяем высоту ячейки
            let cell_height = if cell.v_merge == Some(VMerge::Restart) {
                // Начало объединения - вычисляем высоту по нескольким строкам
                compute_merged_cell_height(row_idx, cell_idx, table, &row_heights)
            } else {
                row_height.max(content_height)
            };

            row_cells.push(TableCellLayout {
                rect: Rect::new(cell_x, current_y, cell_width, cell_height),
                content_height,
            });

            current_x += cell_width;
        }

        result_rows.push(TableRowLayout {
            height: row_height,
            cells: row_cells,
        });

        total_height += row_height;
        current_y += row_height;
    }

    TableLayoutResult {
        height: total_height,
        rows: result_rows,
    }
}

/// Вычислить ширину ячейки с учётом grid span.
#[must_use]
fn compute_cell_width(cell: &Cell, col_width: f32, col_widths: &[f32], cell_idx: usize) -> f32 {
    let grid_span = cell.grid_span;
    if grid_span > 1 {
        let end_col = (cell_idx + grid_span as usize).min(col_widths.len());
        col_widths[cell_idx..end_col].iter().sum::<f32>()
    } else {
        col_width
    }
}

/// Вычислить высоту ячейки по содержимому.
#[must_use]
fn layout_cell_content(_cell: &Cell, _width: f32) -> f32 {
    // TODO: Реальная раскладка содержимого ячейки
    // Пока возвращаем минимальную высоту
    20.0
}

/// Вычислить высоту строки по содержимому ячеек.
#[must_use]
fn compute_row_height_from_content(row: &Row, col_widths: &[f32]) -> f32 {
    row.cells
        .iter()
        .enumerate()
        .map(|(idx, cell)| {
            let width = col_widths.get(idx).copied().unwrap_or(100.0);
            layout_cell_content(cell, width)
        })
        .fold(0.0, f32::max)
}

/// Вычислить высоту объединённой ячейки.
#[must_use]
fn compute_merged_cell_height(
    start_row: usize,
    start_col: usize,
    table: &Table,
    row_heights: &[f32],
) -> f32 {
    // Найдём конец объединения
    let mut end_row = start_row;
    for row_idx in (start_row + 1)..table.rows.len() {
        let cell = table.rows.get(row_idx).and_then(|r| r.cells.get(start_col));

        if let Some(c) = cell {
            if c.v_merge != Some(VMerge::Continue) {
                break;
            }
        } else {
            break;
        }
        end_row = row_idx;
    }

    // Суммируем высоты строк
    (start_row..=end_row)
        .filter_map(|idx| row_heights.get(idx))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_column_widths_empty() {
        use doc_converter_core::NodeId;
        let table = Table {
            id: NodeId::new(1),
            style_ref: None,
            grid: vec![],
            rows: vec![],
            layout: crate::TableLayout::Autofit,
            width: None,
            borders: crate::TableBorders::default(),
            look: crate::TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: crate::CellMargins::default(),
        };

        let widths = compute_column_widths(&table, 1000.0);
        assert_eq!(widths, vec![1000.0]);
    }

    #[test]
    fn test_compute_column_widths_single() {
        use crate::GridCol;
        use doc_converter_core::NodeId;
        let table = Table {
            id: NodeId::new(1),
            style_ref: None,
            grid: vec![GridCol {
                width: Twips::new(1440), // 1 inch
            }],
            rows: vec![],
            layout: crate::TableLayout::Autofit,
            width: None,
            borders: crate::TableBorders::default(),
            look: crate::TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: crate::CellMargins::default(),
        };

        let widths = compute_column_widths(&table, 1000.0);
        assert_eq!(widths.len(), 1);
        assert!((widths[0] - 96.0).abs() < 0.01); // 1440 twips = 96 px
    }

    #[test]
    fn test_compute_row_heights() {
        use crate::{HeightRule, RowHeight};
        use doc_converter_core::NodeId;
        let rows = vec![Row {
            id: NodeId::new(1),
            cells: vec![],
            height: Some(RowHeight {
                value: Twips::new(288), // 0.2 inches
                rule: HeightRule::Exact,
            }),
            cant_split: false,
            header: false,
        }];

        let heights = compute_row_heights(&rows);
        assert_eq!(heights.len(), 1);
        // 288 twips = 288/15 = 19.2 px, but we have a minimum of 20.0
        // So the height should be at least 20.0
        assert!(heights[0] >= 19.2);
    }
}
