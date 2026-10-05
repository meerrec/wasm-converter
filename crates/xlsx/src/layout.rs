//! Раскладка листа: где проходят границы столбцов и строк.
//!
//! Excel не выводит ширину столбца из содержимого — она записана в файле, — и
//! измеряет её не в точках, а в **ширине символа `0`** основного шрифта. Для
//! Calibri 11 это 7 пикселей, плюс пять пикселей полей: привычные 8,43 символа
//! дают ровно 64 пикселя. Высоты строк заданы в пунктах и переводятся по
//! обычной типографской мере: 96/72.
//!
//! Раскладка считается без масштаба: зум и DPR — дело того, кто рисует.
//! Столбцов в листе бывает 16 384, поэтому позиция не накапливается по одному
//! столбцу, а складывается из отрезков: полосы одинаковой ширины из `<cols>` и
//! общая ширина между ними.

use crate::cellref::MAX_COL;
use crate::dims::SheetFormat;
use crate::model::Sheet;

/// Пикселей на пункт: 96 dpi против 72 pt.
pub const PX_PER_POINT: f32 = 96.0 / 72.0;
/// Ширина символа `0` у Calibri 11 — единица измерения ширин Excel.
pub const MAX_DIGIT_WIDTH: f32 = 7.0;
/// Поправка из формулы Excel: `Truncate(128 / ширину символа)`.
const DIGIT_ADJUST: f32 = 18.0;
/// Поля ячейки, которые Excel прибавляет к ширине текста.
pub const CELL_PADDING: f32 = 5.0;

/// Сколько целых шагов укладывается в отрезок.
///
/// Отрицательного смещения тут быть не может — координата уже проверена, — а
/// дробная часть и есть попадание внутрь шага.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn steps(offset: f32, step: f32) -> u32 {
    if step <= 0.0 || offset <= 0.0 {
        return 0;
    }
    (offset / step) as u32
}

/// Привести счётчик (строки, столбцы, их количество) к `f32`.
///
/// Точность здесь не нужна и не важна: координата в листе не превышает
/// 1 048 576 × 20 пикселей, а это меньше 2^25 — то есть заведомо точнее, чем
/// мантисса `f32` позволит измерить.
#[allow(clippy::cast_precision_loss)]
fn to_px(count: u32) -> f32 {
    count as f32
}

/// Перевести ширину Excel («символы») в пиксели.
///
/// Ширина символа берётся для Calibri 11 — шрифта по умолчанию в OOXML.
/// Учёт настоящего шрифта книги — задача Фазы 5, вместе с остальными метриками.
#[must_use]
pub fn width_to_px(chars: f32) -> f32 {
    // Скрытый столбец не занимает места вовсе, и поля ему тоже не положены.
    if chars <= 0.0 {
        return 0.0;
    }
    // Формула Excel: ширина округляется до 1/256 символа вниз, уже потом
    // прибавляются поля. Без этого шага привычные 8,43 дали бы 64,01 пикселя.
    let scaled = (chars * 256.0 + DIGIT_ADJUST) / 256.0 * MAX_DIGIT_WIDTH;
    scaled.trunc() + CELL_PADDING
}

/// Перевести высоту строки из пунктов в пиксели.
#[must_use]
pub fn height_to_px(points: f32) -> f32 {
    points * PX_PER_POINT
}

/// Полоса столбцов одинаковой ширины.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ColSpan {
    first: u32,
    last: u32,
    width: f32,
}

/// Раскладка листа в пикселях, без масштаба.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetLayout {
    /// Ширина столбца по умолчанию.
    default_col_width: f32,
    /// Высота строки по умолчанию.
    default_row_height: f32,
    /// Полосы столбцов своей ширины, по возрастанию.
    spans: Vec<ColSpan>,
    /// Строки своей высоты: номер и высота, по возрастанию.
    row_heights: Vec<(u32, f32)>,
    /// Сколько столбцов и строк в раскладке.
    cols: u32,
    rows: u32,
}

impl SheetLayout {
    /// Разложить лист, отведя под прокрутку используемый диапазон и по одной
    /// строке с столбцом запаса: без запаса лист выглядит обрезанным по краю.
    #[must_use]
    pub fn new(sheet: &Sheet) -> Self {
        Self::with_margin(sheet, 1)
    }

    /// То же, но с явным запасом строк и столбцов за используемым диапазоном.
    #[must_use]
    pub fn with_margin(sheet: &Sheet, margin: u32) -> Self {
        let format = sheet.dims.format;
        let used = sheet.cells.used_range();
        let (cols, rows) = used.map_or((1, 1), |range| {
            (
                (range.last.col + 1 + margin).min(MAX_COL + 1),
                range.last.row + 1 + margin,
            )
        });

        let spans = sheet
            .dims
            .cols
            .spans()
            .iter()
            .filter(|span| span.first < cols)
            .map(|span| ColSpan {
                first: span.first,
                last: span.last.min(cols - 1),
                width: width_to_px(span.effective_width()),
            })
            .collect();

        let row_heights = sheet
            .dims
            .rows
            .rows()
            .iter()
            .filter(|entry| entry.row < rows)
            .map(|entry| {
                (
                    entry.row,
                    height_to_px(if entry.hidden { 0.0 } else { entry.height }),
                )
            })
            .collect();

        Self {
            default_col_width: width_to_px(format.default_col_width),
            default_row_height: height_to_px(format.effective_row_height()),
            spans,
            row_heights,
            cols,
            rows,
        }
    }

    /// Ширина столбца по умолчанию.
    #[must_use]
    pub fn default_col_width(&self) -> f32 {
        self.default_col_width
    }

    /// Высота строки по умолчанию.
    #[must_use]
    pub fn default_row_height(&self) -> f32 {
        self.default_row_height
    }

    /// Число столбцов в раскладке.
    #[must_use]
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// Число строк в раскладке.
    #[must_use]
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Ширина столбца в пикселях; у скрытого она нулевая.
    #[must_use]
    pub fn column_width(&self, col: u32) -> f32 {
        self.spans
            .iter()
            .find(|span| col >= span.first && col <= span.last)
            .map_or(self.default_col_width, |span| span.width)
    }

    /// Высота строки в пикселях; у скрытой она нулевая.
    #[must_use]
    pub fn row_height(&self, row: u32) -> f32 {
        self.row_heights
            .binary_search_by_key(&row, |(r, _)| *r)
            .map_or(self.default_row_height, |i| self.row_heights[i].1)
    }

    /// Левая граница столбца.
    #[must_use]
    pub fn column_x(&self, col: u32) -> f32 {
        let mut x = to_px(col) * self.default_col_width;
        for span in &self.spans {
            if span.first >= col {
                break;
            }
            // Пересечение полосы с `[0, col)`.
            let end = span.last.min(col - 1);
            let count = to_px(end - span.first + 1);
            x += count * (span.width - self.default_col_width);
        }
        x.max(0.0)
    }

    /// Верхняя граница строки.
    #[must_use]
    pub fn row_y(&self, row: u32) -> f32 {
        let mut y = to_px(row) * self.default_row_height;
        for (r, height) in &self.row_heights {
            if *r >= row {
                break;
            }
            y += height - self.default_row_height;
        }
        y.max(0.0)
    }

    /// Полная ширина листа.
    #[must_use]
    pub fn total_width(&self) -> f32 {
        self.column_x(self.cols)
    }

    /// Полная высота листа.
    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.row_y(self.rows)
    }

    /// Столбец, в который попадает координата `x`.
    #[must_use]
    pub fn column_at(&self, x: f32) -> u32 {
        if x <= 0.0 {
            return 0;
        }
        let mut left = 0.0_f32;
        let mut col = 0_u32;
        for span in &self.spans {
            let gap = to_px(span.first - col) * self.default_col_width;
            if x < left + gap && self.default_col_width > 0.0 {
                return (col + steps(x - left, self.default_col_width)).min(self.cols);
            }
            left += gap;
            if span.width <= 0.0 {
                // Скрытые столбцы места не занимают, но номер пропускают.
                col = span.last + 1;
                continue;
            }
            let span_width = to_px(span.last - span.first + 1) * span.width;
            if x < left + span_width {
                return (span.first + steps(x - left, span.width)).min(self.cols);
            }
            left += span_width;
            col = span.last + 1;
        }
        if self.default_col_width <= 0.0 {
            return col.min(self.cols);
        }
        (col + steps(x - left, self.default_col_width)).min(self.cols)
    }

    /// Строка, в которую попадает координата `y`.
    #[must_use]
    pub fn row_at(&self, y: f32) -> u32 {
        if y <= 0.0 {
            return 0;
        }
        let mut top = 0.0_f32;
        let mut row = 0_u32;
        for (number, height) in &self.row_heights {
            let gap = to_px(*number - row) * self.default_row_height;
            if y < top + gap && self.default_row_height > 0.0 {
                return (row + steps(y - top, self.default_row_height)).min(self.rows);
            }
            top += gap;
            row = *number;
            if *height <= 0.0 {
                row += 1;
                continue;
            }
            if y < top + height {
                return (*number).min(self.rows);
            }
            top += height;
            row += 1;
        }
        if self.default_row_height <= 0.0 {
            return row.min(self.rows);
        }
        (row + steps(y - top, self.default_row_height)).min(self.rows)
    }

    /// Столбцы, попадающие в горизонтальный отрезок `[from, to)`.
    ///
    /// Границы включительные: частично видимый столбец рисуется целиком, иначе
    /// на краю окна появлялась бы полоса пустоты.
    #[must_use]
    pub fn columns_in(&self, from: f32, to: f32) -> (u32, u32) {
        let first = self.column_at(from);
        let last = self.column_at((to - 0.001).max(from));
        (first, last.min(self.cols.saturating_sub(1)))
    }

    /// Строки, попадающие в вертикальный отрезок `[from, to)`.
    #[must_use]
    pub fn rows_in(&self, from: f32, to: f32) -> (u32, u32) {
        let first = self.row_at(from);
        let last = self.row_at((to - 0.001).max(from));
        (first, last.min(self.rows.saturating_sub(1)))
    }
}

/// Формат листа, каким он был бы у пустого листа Excel.
#[must_use]
pub fn default_format() -> SheetFormat {
    SheetFormat::default()
}

#[cfg(test)]
// Ширины и высоты в тестах — точные литералы, представимые в `f32`.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::model::{Cell, CellValue, SheetContent, WorksheetBuilder, WorksheetMeta};
    use crate::{Pane, Sheet, SheetState};

    const PART: &str = "xl/worksheets/sheet1.xml";

    /// Лист с ячейками в указанных строках и столбцах.
    fn sheet_with(content: SheetContent) -> Sheet {
        Sheet::new(
            WorksheetMeta {
                name: "Лист1".into(),
                part: PART.into(),
                state: SheetState::Visible,
            },
            content,
        )
    }

    fn grid(rows: &[u32], cols: &[u32]) -> Sheet {
        build(rows, cols, |_| {})
    }

    /// Лист с ячейками-заглушками и правкой геометрии поверх них.
    ///
    /// Ячейки нужны не для красоты: раскладка отводит место по используемому
    /// диапазону, и лист без ячеек — это одна строка и один столбец.
    fn build(rows: &[u32], cols: &[u32], tweak: impl FnOnce(&mut SheetContent)) -> Sheet {
        let mut builder = WorksheetBuilder::new(PART);
        for row in rows {
            for col in cols {
                builder
                    .push(*row, Cell::new(*col, 0, CellValue::Number(1.0)))
                    .unwrap();
            }
        }
        let mut content = SheetContent {
            cells: builder.finish(),
            ..SheetContent::default()
        };
        tweak(&mut content);
        sheet_with(content)
    }

    /// Раскладка листа с ячейками в столбцах `0..cols` и строках `0..rows`.
    fn layout_of(rows: u32, cols: u32, tweak: impl FnOnce(&mut SheetContent)) -> SheetLayout {
        let rows: Vec<u32> = (0..rows).collect();
        let cols: Vec<u32> = (0..cols).collect();
        SheetLayout::new(&build(&rows, &cols, tweak))
    }

    #[test]
    fn default_geometry_matches_excel() {
        let layout = SheetLayout::new(&grid(&[0], &[0]));

        // 8,43 символа Calibri 11 — это ровно 64 пикселя, 15 пунктов — 20.
        assert_eq!(layout.default_col_width(), 64.0);
        assert_eq!(layout.default_row_height(), 20.0);
        assert_eq!(width_to_px(8.43), 64.0);
        assert_eq!(height_to_px(15.0), 20.0);
    }

    #[test]
    fn positions_accumulate_over_spans() {
        let layout = layout_of(3, 4, |content| {
            content.dims.cols.push(crate::ColWidth {
                first: 1,
                last: 1,
                width: 20.0,
                custom: true,
                hidden: false,
                best_fit: false,
            });
            content.dims.rows.push(crate::RowHeight {
                row: 1,
                height: 30.0,
                custom: true,
                hidden: false,
                outline_level: 0,
            });
        });

        assert_eq!(layout.column_width(0), 64.0);
        assert_eq!(layout.column_width(1), width_to_px(20.0));
        assert_eq!(layout.column_x(0), 0.0);
        assert_eq!(layout.column_x(1), 64.0);
        assert_eq!(layout.column_x(2), 64.0 + width_to_px(20.0));
        assert_eq!(layout.column_x(3), 64.0 + width_to_px(20.0) + 64.0);

        assert_eq!(layout.row_y(0), 0.0);
        assert_eq!(layout.row_y(1), 20.0);
        assert_eq!(layout.row_y(2), 20.0 + height_to_px(30.0));

        // Пустой лист всё равно даёт одну строку и столбец.
        let empty = SheetLayout::new(&sheet_with(SheetContent::default()));
        assert_eq!((empty.cols(), empty.rows()), (1, 1));
    }

    #[test]
    fn hidden_tracks_take_no_space() {
        let layout = layout_of(1, 5, |content| {
            for col in [1, 3] {
                content.dims.cols.push(crate::ColWidth {
                    first: col,
                    last: col,
                    width: 8.43,
                    custom: true,
                    hidden: true,
                    best_fit: false,
                });
            }
        });

        assert_eq!(layout.column_width(1), 0.0);
        // Столбцы 0…3 занимают 64 + 0 + 64 + 0 пикселя.
        assert_eq!(layout.column_x(2), 64.0);
        assert_eq!(layout.column_x(3), 128.0);
        assert_eq!(layout.column_x(4), 128.0);
        // Скрытый столбец не занимает места, поэтому координата в нём попадает
        // в следующий за ним.
        assert_eq!(layout.column_at(64.0), 2);
        assert_eq!(layout.column_at(0.0), 0);
    }

    #[test]
    fn lookup_is_the_inverse_of_position() {
        let layout = layout_of(12, 12, |content| {
            content.dims.cols.push(crate::ColWidth {
                first: 0,
                last: 2,
                width: 4.0,
                custom: true,
                hidden: false,
                best_fit: false,
            });
        });

        for col in 0..12 {
            let x = layout.column_x(col);
            assert_eq!(layout.column_at(x), col, "столбец {col} по x={x}");
            // Внутри столбца координата тоже даёт его номер.
            assert_eq!(layout.column_at(x + layout.column_width(col) / 2.0), col);
        }
        for row in 0..12 {
            assert_eq!(layout.row_at(layout.row_y(row)), row, "строка {row}");
        }
    }

    #[test]
    fn visible_ranges_cover_partial_cells() {
        let layout = SheetLayout::new(&grid(&[0, 1, 2], &[0, 1, 2]));

        // Окно шириной в полтора столбца: второй показан частично, но целиком.
        assert_eq!(layout.columns_in(0.0, 96.0), (0, 1));
        assert_eq!(layout.columns_in(64.0, 200.0), (1, 3));
        assert_eq!(layout.rows_in(0.0, 30.0), (0, 1));
        assert_eq!(layout.rows_in(0.0, 10_000.0).1, layout.rows() - 1);
        assert_eq!(layout.columns_in(0.0, 10_000.0).1, layout.cols() - 1);
    }

    #[test]
    fn total_size_follows_content() {
        let layout = SheetLayout::new(&grid(&[0, 4], &[0, 2]));

        // Запас в строку и столбец за используемым диапазоном.
        assert_eq!(layout.cols(), 4);
        assert_eq!(layout.rows(), 6);
        assert_eq!(layout.total_width(), 4.0 * 64.0);
        assert_eq!(layout.total_height(), 6.0 * 20.0);
    }

    #[test]
    fn frozen_pane_is_just_a_pane() {
        // Раскладка о закреплении не знает: его разыгрывает тот, кто рисует.
        let mut content = SheetContent::default();
        content.view.pane = Some(Pane {
            cols: 1,
            rows: 1,
            state: crate::PaneState::Frozen,
            active: crate::PaneKind::BottomRight,
            top_left: None,
        });
        let layout = SheetLayout::new(&sheet_with(content));

        assert_eq!(layout.column_x(1), 64.0);
    }
}
