//! Место ячеек в сетке таблицы: столбцы, вертикальные объединения и продолжения.
//!
//! Общее для [`html`](super::html) и [`markdown`](super::markdown): HTML выражает
//! вертикальное объединение `rowspan`-ом и ячеек-продолжений вовсе не выводит,
//! Markdown — пустой ячейкой (GFM объединений не знает). Считать объединения
//! в каждом экспортёре по-своему значило бы разойтись в том, что такое одна ячейка.
//!
//! Объединение открывается ячейкой с `w:vMerge w:val="restart"` и продолжается
//! ячейками `w:vMerge` (значение по умолчанию — `continue`) следующих строк.
//! Продолжение, которому не предшествует начало, — битый файл: такая ячейка
//! выводится как обычная, без `rowspan` (Word рисует её так же).

use std::collections::{BTreeMap, BTreeSet};

use crate::model::table::{Cell, Row, Table, VMerge};

/// Место ячейки в сетке таблицы.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placed<'a> {
    /// Ячейка модели.
    pub(crate) cell: &'a Cell,
    /// Строка, в которой ячейка начинается.
    pub(crate) row: usize,
    /// Столбец, с которого ячейка начинается: сумма `w:gridSpan` ячеек слева.
    pub(crate) col: usize,
    /// Сколько столбцов занимает ячейка (`w:gridSpan`), не меньше одного.
    pub(crate) cols: u32,
    /// Сколько строк занимает ячейка: `1` — вертикального объединения нет.
    pub(crate) rows: u32,
    /// Ячейка продолжает объединение, начатое выше: своего места в первой строке
    /// объединения у неё нет — в разметке это не отдельная ячейка, а признак того,
    /// что соседка сверху занимает и её место.
    pub(crate) continuation: bool,
}

impl Placed<'_> {
    /// Столбец сразу за ячейкой.
    pub(crate) fn end_col(&self) -> usize {
        self.col
            .saturating_add(usize::try_from(self.cols).unwrap_or(usize::MAX))
    }
}

/// Ячейки всех строк с их местами в сетке, по порядку документа.
///
/// Ячейки-продолжения тоже попадают в результат (`continuation`), но `rows` и
/// столбец начала у них — свои: потребитель отличает их по признаку.
pub(crate) fn place(table: &Table) -> Vec<Placed<'_>> {
    let mut placed: Vec<Placed<'_>> = Vec::new();
    // Открытые объединения: столбец начала → номер ячейки-начала в `placed`.
    let mut open: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, row) in table.rows.iter().enumerate() {
        // Объединения, дожившие до этой строки: начатые в ней и продолженные ею.
        let mut active: BTreeSet<usize> = BTreeSet::new();
        let mut col = 0usize;
        for cell in &row.cells {
            let cols = cell.grid_span.max(1);
            let start = col;
            col = col.saturating_add(usize::try_from(cols).unwrap_or(usize::MAX));
            if cell.v_merge == Some(VMerge::Continue) {
                if let Some(position) = open.get(&start) {
                    let merged = &mut placed[*position];
                    merged.rows = merged.rows.saturating_add(1);
                    active.insert(start);
                    placed.push(Placed {
                        cell,
                        row: index,
                        col: start,
                        cols,
                        rows: 1,
                        continuation: true,
                    });
                    continue;
                }
            }
            // Объединение в этом столбце начинает новая ячейка: прежнее кончилось.
            open.remove(&start);
            if cell.v_merge == Some(VMerge::Restart) {
                open.insert(start, placed.len());
                active.insert(start);
            }
            placed.push(Placed {
                cell,
                row: index,
                col: start,
                cols,
                rows: 1,
                continuation: false,
            });
        }
        // Не продолженные этой строкой объединения закончились: их `rows` набран.
        open.retain(|start, _| active.contains(start));
    }
    placed
}

/// Ячейки одной строки с их столбцами, без учёта вертикальных объединений.
///
/// Нужны для соседей по горизонтали: у объединённой по вертикали ячейки сосед
/// справа в промежуточной строке — ячейка, о которой сам `place` уже не помнит,
/// потому что она пришлась на продолжение с другой стороны.
pub(crate) fn row_cells(row: &Row) -> Vec<(usize, &Cell, u32)> {
    let mut out = Vec::with_capacity(row.cells.len());
    let mut col = 0usize;
    for cell in &row.cells {
        let cols = cell.grid_span.max(1);
        out.push((col, cell, cols));
        col = col.saturating_add(usize::try_from(cols).unwrap_or(usize::MAX));
    }
    out
}

#[cfg(test)]
mod tests {
    use doc_converter_core::NodeId;

    use super::*;
    use crate::model::raw::CellVAlign;
    use crate::model::table::{CellMargins, Row, Table, TableBorders, TableLayout, TableLook};

    /// Ячейка без содержимого: `span` столбцов и, если задано, вид объединения.
    fn cell(span: u32, v_merge: Option<VMerge>) -> Cell {
        Cell {
            id: NodeId::new(1),
            grid_span: span,
            v_merge,
            width: None,
            margins: CellMargins::default(),
            v_align: CellVAlign::Top,
            borders: Box::default(),
            shading: None,
            items: Vec::new(),
        }
    }

    /// Строка из ячеек.
    fn row(cells: Vec<Cell>) -> Row {
        Row {
            id: NodeId::new(1),
            cells,
            height: None,
            cant_split: false,
            header: false,
        }
    }

    /// Таблица из строк.
    fn table(rows: Vec<Row>) -> Table {
        Table {
            id: NodeId::new(1),
            style_ref: None,
            grid: Vec::new(),
            rows,
            layout: TableLayout::Autofit,
            width: None,
            borders: TableBorders::default(),
            look: TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: CellMargins::default(),
        }
    }

    /// Ячейка по номеру строки и признаку продолжения.
    fn find<'a>(placed: &'a [Placed<'a>], row: usize, continuation: bool) -> &'a Placed<'a> {
        placed
            .iter()
            .find(|place| place.row == row && place.continuation == continuation)
            .expect("ячейка на месте")
    }

    #[test]
    fn a_merge_counts_the_rows_that_continue_it() {
        // Три строки, объединение в первом столбце — от `restart` до последней строки.
        let table = table(vec![
            row(vec![cell(1, Some(VMerge::Restart)), cell(1, None)]),
            row(vec![cell(1, Some(VMerge::Continue)), cell(1, None)]),
            row(vec![cell(1, Some(VMerge::Continue)), cell(1, None)]),
        ]);
        let placed = place(&table);
        assert_eq!(placed[0].rows, 3, "объединение на три строки");
        assert!(!placed[0].continuation);
        assert_eq!(
            placed.iter().filter(|place| place.continuation).count(),
            2,
            "продолжений два"
        );
        assert_eq!(
            placed[3].col, 1,
            "правая колонка начинается со второго столбца"
        );
    }

    #[test]
    fn a_merge_that_starts_below_the_first_row_covers_only_its_rows() {
        let table = table(vec![
            row(vec![cell(1, None), cell(1, None)]),
            row(vec![cell(1, Some(VMerge::Restart)), cell(1, None)]),
            row(vec![cell(1, Some(VMerge::Continue)), cell(1, None)]),
        ]);
        let placed = place(&table);
        assert_eq!(placed[0].rows, 1, "первая строка не объединена");
        assert_eq!(placed[2].rows, 2, "объединение со второй строки");
    }

    #[test]
    fn a_gap_in_the_middle_ends_the_merge_where_it_ended() {
        // В третьей строке продолжения нет: объединение кончилось на второй,
        // а продолжение в четвёртой строке — уже битый файл, оно не объединяет.
        let table = table(vec![
            row(vec![cell(1, Some(VMerge::Restart)), cell(1, None)]),
            row(vec![cell(1, Some(VMerge::Continue)), cell(1, None)]),
            row(vec![cell(1, None), cell(1, None)]),
            row(vec![cell(1, Some(VMerge::Continue)), cell(1, None)]),
        ]);
        let placed = place(&table);
        assert_eq!(placed[0].rows, 2, "объединение кончилось на дыре");
        let orphan = placed
            .iter()
            .find(|place| place.row == 3)
            .expect("ячейка четвёртой строки");
        assert!(
            !orphan.continuation,
            "продолжение без начала — обычная ячейка"
        );
        assert_eq!(orphan.rows, 1);
    }

    #[test]
    fn a_merge_keeps_its_horizontal_span() {
        // `w:gridSpan` у начала объединения: столбец следующий — через два.
        let table = table(vec![
            row(vec![cell(2, Some(VMerge::Restart)), cell(1, None)]),
            row(vec![cell(2, Some(VMerge::Continue)), cell(1, None)]),
        ]);
        let placed = place(&table);
        assert_eq!(placed[0].cols, 2, "объединение занимает два столбца");
        assert_eq!(placed[0].rows, 2);
        assert_eq!(placed[0].end_col(), 2);
        assert_eq!(
            find(&placed, 1, false).col,
            2,
            "ячейка справа — в третьем столбце"
        );
    }

    #[test]
    fn a_row_that_does_not_continue_the_merge_leaves_it_closed() {
        // Пятая строка начинается новым объединением в том же столбце: прежнее
        // закрывается, новое считается с неё.
        let table = table(vec![
            row(vec![cell(1, Some(VMerge::Restart))]),
            row(vec![cell(1, Some(VMerge::Continue))]),
            row(vec![cell(1, Some(VMerge::Restart))]),
        ]);
        let placed = place(&table);
        assert_eq!(placed[0].rows, 2);
        assert_eq!(placed[2].rows, 1);
    }

    #[test]
    fn rows_of_a_row_repeat_the_columns_of_its_cells() {
        let row = row(vec![cell(3, None), cell(2, None)]);
        let cells = row_cells(&row);
        assert_eq!(
            cells
                .iter()
                .map(|(col, _, cols)| (*col, *cols))
                .collect::<Vec<_>>(),
            vec![(0, 3), (3, 2)]
        );
    }
}
