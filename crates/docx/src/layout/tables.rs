//! Раскладка таблиц DOCX.
//!
//! Таблицы раскладываются построчно: сначала вычисляется ширина колонок,
//! затем каждая строка раскладывается внутри этой ширины.
//!
//! Высота строки, не заданной `w:trHeight`, считается по содержимому: текст
//! ячеек переносится тем же [`LineBreaker`]-ом, что и абзацы тела (ADR-0005),
//! иначе высоты таблицы и страницы разъезжались бы.

use doc_converter_render::font::{FontId, FontRegistry};

use crate::layout::line_break::LineBreaker;
use crate::model::raw::{Color, HalfPoint};
use crate::model::{BlockItem, Inline, Paragraph, RunContent};
use crate::{Cell, Row, Table, Twips, VMerge};

/// Преобразовать twips в пиксели: 1 twip = 1/1440 дюйма, 1 пиксель = 1/96 дюйма.
/// Отношение: (1/1440) / (1/96) = 96/1440 = 1/15.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
fn twips_to_px(twips: Twips) -> f32 {
    twips.value() as f32 / 15.0
}

use super::engine::{LayoutItem, Rect, TableCellLayout};

/// Минимальная высота строки таблицы в пикселях.
///
/// Применяется к готовой высоте в [`layout_table`], а не к `w:trHeight` в
/// [`compute_row_heights`]: там `0.0` означает «считать по содержимому», и пол,
/// наложенный раньше, стёр бы это отличие — строка `Auto` так и осталась бы
/// нераскрытой.
const MIN_ROW_HEIGHT: f32 = 20.0;

/// Кегль по умолчанию — тот же, что у остальной раскладки (12 pt).
///
/// `FontRegistry` не ищет шрифт по имени, поэтому `w:rFonts`/`w:sz` из свойств
/// ячейки и run'ов пока не применяются.
const DEFAULT_SIZE_HALF_POINTS: HalfPoint = HalfPoint::new(24);

/// Раскладка таблицы на странице.
#[derive(Debug, Clone)]
pub struct TableLayoutResult {
    /// Общая высота таблицы.
    pub height: f32,
    /// Ширина таблицы: сумма ширин колонок.
    pub width: f32,
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

/// Вычислить заданные высоты строк таблицы.
///
/// # Arguments
/// * `rows` - строки таблицы
///
/// # Returns
/// Вектор высот строк в пикселях. `0.0` означает «высота по содержимому»
/// (`w:trHeight` отсутствует или `w:hRule="auto"`): содержимого эта функция не
/// видит, высоту считает [`layout_table`], там же применяется и минимум
/// [`MIN_ROW_HEIGHT`].
#[must_use]
pub fn compute_row_heights(rows: &[Row]) -> Vec<f32> {
    rows.iter()
        .map(|row| match row.height.as_ref() {
            Some(row_height) => match row_height.rule {
                crate::HeightRule::Exact | crate::HeightRule::AtLeast => {
                    twips_to_px(row_height.value)
                }
                crate::HeightRule::Auto => 0.0,
            },
            None => 0.0,
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
    let col_widths = compute_column_widths(table, available_width);

    // Высоты считаем до раскладки: объединённой ячейке (`vMerge`-начало) нужны
    // высоты строк ниже, а они к тому моменту уже должны быть посчитаны по
    // содержимому, а не остаться нулями.
    let declared_heights = compute_row_heights(&table.rows);
    let mut row_heights = Vec::with_capacity(table.rows.len());
    for (row_idx, row) in table.rows.iter().enumerate() {
        let declared = declared_heights.get(row_idx).copied().unwrap_or(0.0);
        let height = if declared > 0.0 {
            declared
        } else {
            compute_row_height_from_content(row, &col_widths, fonts)
        };
        row_heights.push(height.max(MIN_ROW_HEIGHT));
    }

    let mut result_rows = Vec::new();
    let mut total_height = 0.0;
    let mut current_y = y;

    for (row_idx, row) in table.rows.iter().enumerate() {
        let row_height = row_heights.get(row_idx).copied().unwrap_or(MIN_ROW_HEIGHT);
        let mut row_cells = Vec::new();
        let mut current_x = x;

        for (cell_idx, cell) in row.cells.iter().enumerate() {
            let col_width = col_widths.get(cell_idx).copied().unwrap_or(100.0);

            // Вычисляем позицию и размер ячейки
            let cell_x = current_x;
            let cell_width = compute_cell_width(cell, col_width, &col_widths, cell_idx);

            // Содержимое ложится от верхнего левого угла ячейки, поэтому его
            // раскладка от высоты ячейки не зависит: высота считается после.
            let (content, content_height) =
                layout_cell_items(cell, cell_x, current_y, cell_width, fonts);

            // Определяем высоту ячейки
            let cell_height = if cell.v_merge == Some(VMerge::Restart) {
                // Начало объединения - вычисляем высоту по нескольким строкам
                compute_merged_cell_height(row_idx, cell_idx, table, &row_heights)
            } else {
                row_height.max(content_height)
            };

            row_cells.push(TableCellLayout {
                rect: Rect::new(cell_x, current_y, cell_width, cell_height),
                content,
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
        width: col_widths.iter().sum(),
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

/// Разложить содержимое ячейки в элементы: стопка строк от её верхнего левого угла.
///
/// Абзацы ячейки переносятся [`LineBreaker`]-ом — тем же кодом, что и абзацы
/// тела (ADR-0005); кегль — умолчание раскладки. Ширина для переноса — ширина
/// ячейки за вычетом боковых полей `w:tcMar`; сверху и снизу поля добавляются
/// к высоте. Незаполненный абзац занимает строку: пустая ячейка не должна
/// схлопываться в ноль.
///
/// Высоту содержимого отдаёт вместе с элементами та же функция: отдельный
/// счётчик разошёлся бы с тем, что легло в элементы, и строка таблицы
/// перестала бы совпадать с содержимым.
///
/// Не учтены (вне слайса): межстрочный интервал и отступы абзаца из каскада,
/// `w:rFonts`/`w:sz` (реестр не ищет шрифт по имени), выравнивание и
/// вертикальное выравнивание содержимого, вложенные таблицы и рисунки в ячейке.
#[must_use]
#[allow(clippy::cast_precision_loss)]
fn layout_cell_items(
    cell: &Cell,
    x: f32,
    y: f32,
    width: f32,
    fonts: &mut FontRegistry,
) -> (Vec<LayoutItem>, f32) {
    let padding_left = side_margin(cell.margins.left);
    let padding_x = padding_left + side_margin(cell.margins.right);
    let padding_y = side_margin(cell.margins.top) + side_margin(cell.margins.bottom);

    // Узкая колонка с широкими полями даёт нулевую (и отрицательную) ширину:
    // разбивка на такой ширине возвращает пустой список, высота остаётся
    // конечной — одной строкой на абзац.
    let inner_width = (width - padding_x).max(0.0);

    let mut breaker = LineBreaker::new(fonts, FontId::default());
    let line_height = breaker.line_height(DEFAULT_SIZE_HALF_POINTS);

    let mut content = Vec::new();
    let mut content_height = 0.0;
    let mut line_top = y + side_margin(cell.margins.top);

    for item in &cell.items {
        let BlockItem::Paragraph(paragraph) = item else {
            continue;
        };
        let text = paragraph_text(paragraph);
        let mut ranges = breaker.break_lines(
            &text,
            FontId::default(),
            DEFAULT_SIZE_HALF_POINTS,
            inner_width,
        );
        // Пустой абзац — одна строка нулевой ширины, как и в абзацах тела.
        if ranges.is_empty() {
            ranges.push(0..0);
        }
        content_height += ranges.len() as f32 * line_height;

        let color = paragraph_color(paragraph);
        for range in ranges {
            let line_text = &text[range];
            let line_width =
                breaker.measure_text(line_text, FontId::default(), DEFAULT_SIZE_HALF_POINTS);
            content.push(LayoutItem::Paragraph {
                node_id: paragraph.id,
                rect: Rect::new(x + padding_left, line_top, line_width, line_height),
                text: line_text.to_owned(),
                line_height,
                color,
            });
            line_top += line_height;
        }
    }

    (content, content_height + padding_y)
}

/// Вычислить высоту содержимого ячейки.
///
/// Считает та же [`layout_cell_items`], что отдаёт элементы: свой счётчик
/// разошёлся бы с раскладкой, и высота строки перестала бы её покрывать.
#[must_use]
fn layout_cell_content(cell: &Cell, width: f32, fonts: &mut FontRegistry) -> f32 {
    layout_cell_items(cell, 0.0, 0.0, width, fonts).1
}

/// Цвет текста абзаца ячейки — первый явный `w:color` среди его runs.
///
/// Каскад до ячеек пока не доходит, поэтому читается собственный `w:rPr` run'а;
/// `auto` и `none` цвета не задают. Строка несёт один цвет, а run'ы ячейки не
/// разбиваются по метрикам, поэтому из нескольких цветов берётся первый.
#[must_use]
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

/// Поле ячейки в пикселях; незаданное поле — ноль.
fn side_margin(margin: Option<Twips>) -> f32 {
    margin.map_or(0.0, twips_to_px)
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

/// Вычислить высоту строки по содержимому ячеек.
#[must_use]
fn compute_row_height_from_content(row: &Row, col_widths: &[f32], fonts: &mut FontRegistry) -> f32 {
    let mut row_height = 0.0_f32;
    for (idx, cell) in row.cells.iter().enumerate() {
        let col_width = col_widths.get(idx).copied().unwrap_or(100.0);
        // Ширина та же, что и при раскладке ячейки: иначе объединённая по
        // горизонтали ячейка мерилась бы по одной колонке, а ложилась по сумме.
        let width = compute_cell_width(cell, col_width, col_widths, idx);
        row_height = row_height.max(layout_cell_content(cell, width, fonts));
    }
    row_height
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
    use crate::model::{
        CellBorders, CellMargins, CellVAlign, GridCol, Run, TableBorders, TableLayout, TableLook,
    };
    use doc_converter_core::NodeId;

    /// Абзац из одного run'а с одним текстом.
    fn paragraph(text: &str) -> Paragraph {
        Paragraph {
            runs: vec![Inline::Run(Run {
                content: vec![RunContent::Text(text.to_string())],
                ..Run::default()
            })],
            ..Paragraph::default()
        }
    }

    /// Ячейка с одним абзацем текста и без полей.
    fn cell(text: &str) -> Cell {
        Cell {
            id: NodeId::new(1),
            grid_span: 1,
            v_merge: None,
            width: None,
            margins: CellMargins::default(),
            v_align: CellVAlign::Top,
            borders: Box::new(CellBorders::default()),
            shading: None,
            items: vec![BlockItem::Paragraph(paragraph(text))],
        }
    }

    /// Строка без `w:trHeight` — высота по содержимому.
    fn row(cells: Vec<Cell>) -> Row {
        Row {
            id: NodeId::new(1),
            cells,
            height: None,
            cant_split: false,
            header: false,
        }
    }

    /// Высота содержимого ячейки: насколько его элементы уходят ниже её верха.
    fn content_height(cell: &TableCellLayout) -> f32 {
        cell.content
            .iter()
            .fold(cell.rect.y, |bottom, item| match item {
                LayoutItem::Paragraph { rect, .. } => bottom.max(rect.y + rect.height),
                _ => bottom,
            })
            - cell.rect.y
    }

    /// Таблица из одной колонки заданной ширины.
    fn one_column_table(col_width: Twips, rows: Vec<Row>) -> Table {
        Table {
            id: NodeId::new(0),
            style_ref: None,
            grid: vec![GridCol { width: col_width }],
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

    /// Таблица из фикстуры, путь — от `CARGO_MANIFEST_DIR`.
    fn fixture_table(name: &str) -> Table {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx/tables")
            .join(name);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("Не удалось прочитать {}: {error}", path.display()));
        let document = crate::open(bytes).expect("Фикстура должна разбираться");
        document
            .body
            .items
            .into_iter()
            .find_map(|item| match item {
                BlockItem::Table(table) => Some(table),
                _ => None,
            })
            .expect("В фикстуре должна быть таблица")
    }

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
        // Заданная высота отдаётся как есть (288/15 = 19.2 px) — пол 20.0 px
        // применяет layout_table, а не эта функция.
        assert!((heights[0] - 19.2).abs() < 0.01);
    }

    #[test]
    fn auto_height_is_zero_before_layout() {
        let rows = vec![row(vec![cell("текст")])];
        let heights = compute_row_heights(&rows);
        assert_eq!(
            heights,
            vec![0.0],
            "Auto-строка — 0.0, высота по содержимому"
        );
    }

    #[test]
    fn cell_content_height_grows_with_text() {
        let mut fonts = FontRegistry::new(64);
        // 60 px = 900 twips; около 200 символов текста.
        let table = one_column_table(Twips::new(900), vec![row(vec![cell(&"word ".repeat(40))])]);

        let result = layout_table(&table, 0.0, 0.0, 500.0, &mut fonts);
        let height = content_height(&result.rows[0].cells[0]);
        assert!(
            height > 20.0,
            "высота по содержимому должна превысить константу 20.0, получено {height}"
        );
    }

    #[test]
    fn auto_row_height_follows_three_lines_of_text() {
        let mut fonts = FontRegistry::new(64);
        let inner_width = 200.0;
        let text = &"word ".repeat(15);

        let line_count = LineBreaker::new(&mut fonts, FontId::default())
            .break_lines(
                text,
                FontId::default(),
                DEFAULT_SIZE_HALF_POINTS,
                inner_width,
            )
            .len();
        assert_eq!(line_count, 3, "текст подобран под три строки");

        let table = one_column_table(
            // 200 px = 3000 twips
            Twips::new(3000),
            vec![row(vec![cell(text)])],
        );
        let result = layout_table(&table, 0.0, 0.0, 500.0, &mut fonts);
        let height = result.rows[0].height;
        // 3 строки по 12 pt * 1.2 = 3 * 19.2 = 57.6 px
        assert!(
            (height - 57.6).abs() < 0.01,
            "ожидалось 57.6 px по трём строкам текста, получено {height}"
        );
    }

    #[test]
    fn fixture_table_height_is_the_sum_of_its_rows() {
        let mut fonts = FontRegistry::new(64);
        let table = fixture_table("simple_2x2.docx");

        let result = layout_table(&table, 0.0, 0.0, 500.0, &mut fonts);
        let sum: f32 = result.rows.iter().map(|row| row.height).sum();

        assert!(
            (result.height - sum).abs() < 0.001,
            "height = {} не совпадает с суммой высот строк {sum}",
            result.height
        );
        assert!(
            result.rows.iter().all(|row| row.height >= MIN_ROW_HEIGHT),
            "ни одна строка не ниже минимума"
        );
    }

    #[test]
    fn fixture_cell_content_is_measured_not_assumed() {
        let mut fonts = FontRegistry::new(64);
        let table = fixture_table("simple_2x2.docx");

        let result = layout_table(&table, 0.0, 0.0, 500.0, &mut fonts);
        let height = content_height(&result.rows[0].cells[0]);
        // «A1» — одна строка 12 pt: 19.2 px, а не прежняя константа 20.0 px.
        assert!(
            height > 0.0 && height < MIN_ROW_HEIGHT,
            "ожидалась измеренная одна строка текста, получено {height}"
        );
    }

    #[test]
    fn fixture_row_grows_past_the_floor_by_content() {
        let mut fonts = FontRegistry::new(64);
        let table = fixture_table("alignment_widths.docx");

        // Ширина 600 px — сетка [2000, 5000, 2000] twips влезает целиком,
        // «Первая строка Вторая строка» в колонке 133 px переносится на две.
        let result = layout_table(&table, 0.0, 0.0, 600.0, &mut fonts);
        let tallest = result
            .rows
            .iter()
            .map(|row| row.height)
            .fold(0.0_f32, f32::max);
        assert!(
            tallest > MIN_ROW_HEIGHT,
            "строка с перенесённым текстом должна превысить пол, получено {tallest}"
        );
    }
}
