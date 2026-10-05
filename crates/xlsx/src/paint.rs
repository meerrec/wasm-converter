//! Лист в команды рисования: сетка, заливки, текст, заголовки.
//!
//! Координаты `DisplayList` — физические пиксели canvas: масштаб (зум, умноженный
//! на DPR) применяется здесь, а раскладка считается без него. Порядок слоёв
//! повторяет Excel: заливки, поверх них сетка, поверх всего текст. Объединённые
//! ячейки рисуются после сетки — иначе она просвечивала бы сквозь них.
//!
//! Закреплённые области разбивают окно на четыре квадранта: закреплённые строки
//! и столбцы не прокручиваются, остальное — да. Каждый квадрант рисуется под
//! своим `PushClip`, поэтому содержимое не выползает за его границы.

use doc_converter_render::display_list::{
    Color, DisplayList, DrawCommand, LineStyle, TextAlign, TextBaseline,
};

use crate::cellref::{column_name, row_name};
use crate::layout::{SheetLayout, PX_PER_POINT};
use crate::model::{
    Cell, CellValue, Color as CellColor, HorizontalAlign, Sheet, Theme, VerticalAlign,
};
use crate::numfmt;
use crate::Workbook;

/// Ширина полосы заголовков строк, пиксели раскладки.
pub const ROW_HEADER_WIDTH: f32 = 44.0;
/// Высота полосы заголовков столбцов.
pub const COL_HEADER_HEIGHT: f32 = 20.0;
/// Поля текста внутри ячейки.
const TEXT_PADDING: f32 = 3.0;

/// Что видно в окне.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Горизонтальная прокрутка, пиксели раскладки.
    pub scroll_x: f32,
    /// Вертикальная прокрутка, пиксели раскладки.
    pub scroll_y: f32,
    /// Ширина окна в физических пикселях canvas.
    pub width: f32,
    /// Высота окна в физических пикселях canvas.
    pub height: f32,
    /// Масштаб: зум, умноженный на плотность пикселей экрана.
    pub scale: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_y: 0.0,
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        }
    }
}

/// Что рисовать поверх содержимого.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintOptions {
    /// Показывать сетку.
    pub show_grid: bool,
    /// Показывать заголовки строк и столбцов.
    pub show_headers: bool,
    /// Фон листа.
    pub background: Color,
    /// Цвет сетки.
    pub grid: Color,
    /// Фон заголовков.
    pub header_background: Color,
    /// Цвет текста заголовков.
    pub header_foreground: Color,
}

/// Цвета по умолчанию: те же, что у Excel в светлой теме.
mod dl_color {
    use super::Color;

    pub const WHITE: Color = Color::WHITE;
    pub const GRID: Color = Color(0xD9_D9_D9_FF);
    pub const HEADER_BG: Color = Color(0xF5_F5_F5_FF);
    pub const HEADER_FG: Color = Color(0x44_44_44_FF);
}

impl Default for PaintOptions {
    fn default() -> Self {
        Self {
            show_grid: true,
            show_headers: true,
            background: dl_color::WHITE,
            grid: dl_color::GRID,
            header_background: dl_color::HEADER_BG,
            header_foreground: dl_color::HEADER_FG,
        }
    }
}

/// Геометрия окна: полосы заголовков, закреплённые части и границы прокрутки.
///
/// Одна на рисование и на попадание по координате: если считать их порознь,
/// клик рано или поздно разъедется с картинкой.
#[derive(Debug, Clone)]
struct Geometry {
    layout: SheetLayout,
    scale: f32,
    header_w: f32,
    header_h: f32,
    frozen_cols: u32,
    frozen_rows: u32,
    frozen_w: f32,
    frozen_h: f32,
    /// Координата раскладки на левом краю прокручиваемой части.
    view_left: f32,
    view_top: f32,
}

impl Geometry {
    fn new(sheet: &Sheet, viewport: Viewport, show_headers: bool) -> Self {
        let layout = SheetLayout::new(sheet);
        let scale = if viewport.scale > 0.0 {
            viewport.scale
        } else {
            1.0
        };
        let pane = sheet.view.pane.filter(|pane| !pane.is_empty());
        let frozen_cols = pane.map_or(0, |pane| pane.cols);
        let frozen_rows = pane.map_or(0, |pane| pane.rows);

        Self {
            header_w: if show_headers {
                ROW_HEADER_WIDTH * scale
            } else {
                0.0
            },
            header_h: if show_headers {
                COL_HEADER_HEIGHT * scale
            } else {
                0.0
            },
            frozen_w: layout.column_x(frozen_cols) * scale,
            frozen_h: layout.row_y(frozen_rows) * scale,
            view_left: layout.column_x(frozen_cols) + viewport.scroll_x.max(0.0),
            view_top: layout.row_y(frozen_rows) + viewport.scroll_y.max(0.0),
            layout,
            scale,
            frozen_cols,
            frozen_rows,
        }
    }

    /// Координата раскладки по экранной: закреплённая часть не прокручивается.
    fn layout_x(&self, x: f32) -> f32 {
        if x < self.header_w + self.frozen_w {
            (x - self.header_w).max(0.0) / self.scale
        } else {
            self.view_left + (x - self.header_w - self.frozen_w) / self.scale
        }
    }

    fn layout_y(&self, y: f32) -> f32 {
        if y < self.header_h + self.frozen_h {
            (y - self.header_h).max(0.0) / self.scale
        } else {
            self.view_top + (y - self.header_h - self.frozen_h) / self.scale
        }
    }
}

/// Ячейка под точкой окна, в физических пикселях canvas.
///
/// Точка в полосе заголовков даёт крайнюю ячейку: заголовок — это тоже место
/// листа, и возвращать «никуда» было бы неудобно выделению.
#[must_use]
pub fn hit_test(sheet: &Sheet, viewport: Viewport, x: f32, y: f32) -> crate::cellref::CellRef {
    let geometry = Geometry::new(sheet, viewport, true);
    crate::cellref::CellRef::new(
        geometry.layout.row_at(geometry.layout_y(y)),
        geometry.layout.column_at(geometry.layout_x(x)),
    )
}

/// Квадрант окна: какие строки и столбцы в нём видны и где они начинаются.
#[derive(Debug, Clone, Copy)]
struct Region {
    rows: (u32, u32),
    cols: (u32, u32),
    /// Экранная координата левого верхнего угла квадранта.
    screen_x: f32,
    screen_y: f32,
    /// Координата раскладки, попадающая в левый верхний угол квадранта.
    layout_x: f32,
    layout_y: f32,
    /// Прямоугольник отсечения в физических пикселях.
    clip: (f32, f32, f32, f32),
}

impl Region {
    fn screen_x(&self, layout_x: f32, scale: f32) -> f32 {
        self.screen_x + (layout_x - self.layout_x) * scale
    }

    fn screen_y(&self, layout_y: f32, scale: f32) -> f32 {
        self.screen_y + (layout_y - self.layout_y) * scale
    }
}

/// Собрать лист в `DisplayList`.
///
/// Координаты — физические пиксели canvas; `viewport.scale` уже включает и зум,
/// и плотность пикселей экрана.
pub fn build(
    book: &Workbook,
    sheet: &Sheet,
    viewport: Viewport,
    options: &PaintOptions,
    out: &mut DisplayList,
) {
    let Geometry {
        layout,
        scale,
        header_w,
        header_h,
        frozen_cols,
        frozen_rows,
        frozen_w,
        frozen_h,
        view_left,
        view_top,
    } = Geometry::new(sheet, viewport, options.show_headers);

    // Список команд переиспользуется между кадрами: без очистки следующий
    // кадр лёг бы поверх предыдущего, и картинка задвоилась бы.
    out.clear();
    out.push(DrawCommand::Clear);
    fill_rect(
        out,
        0.0,
        0.0,
        viewport.width,
        viewport.height,
        options.background,
    );

    let scroll_w = (viewport.width - header_w - frozen_w).max(0.0) / scale;
    let scroll_h = (viewport.height - header_h - frozen_h).max(0.0) / scale;
    let (first_col, last_col) = layout.columns_in(view_left, view_left + scroll_w);
    let (first_row, last_row) = layout.rows_in(view_top, view_top + scroll_h);

    // Квадранты окна. Закреплённых нет — квадрант один, иначе ячейки
    // закреплённой части нарисовались бы дважды.
    let mut regions: Vec<((u32, u32), (u32, u32))> = Vec::with_capacity(4);
    if frozen_rows > 0 && frozen_cols > 0 {
        regions.push(((0, frozen_rows - 1), (0, frozen_cols - 1)));
    }
    if frozen_rows > 0 {
        regions.push(((0, frozen_rows - 1), (first_col, last_col)));
    }
    if frozen_cols > 0 {
        regions.push(((first_row, last_row), (0, frozen_cols - 1)));
    }
    regions.push(((first_row, last_row), (first_col, last_col)));

    for (rows, cols) in regions {
        if rows.0 > rows.1 || cols.0 > cols.1 {
            continue;
        }
        let (layout_x, screen_x) = if cols.0 < frozen_cols {
            (0.0, header_w)
        } else {
            (view_left, header_w + frozen_w)
        };
        let (layout_y, screen_y) = if rows.0 < frozen_rows {
            (0.0, header_h)
        } else {
            (view_top, header_h + frozen_h)
        };

        let region = Region {
            rows,
            cols,
            screen_x,
            screen_y,
            layout_x,
            layout_y,
            clip: (
                screen_x,
                screen_y,
                (viewport.width - screen_x).max(0.0),
                (viewport.height - screen_y).max(0.0),
            ),
        };
        draw_region(book, sheet, &layout, scale, &region, options, out);
    }

    if options.show_headers {
        draw_headers(
            &Headers {
                layout: &layout,
                options,
                scale,
                viewport,
                header_w,
                header_h,
                frozen_cols,
                frozen_rows,
                view_left,
                view_top,
            },
            out,
        );
    }
}

/// Фон, заливки, сетка и текст одного квадранта.
fn draw_region(
    book: &Workbook,
    sheet: &Sheet,
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    options: &PaintOptions,
    out: &mut DisplayList,
) {
    out.push(DrawCommand::PushClip {
        x: region.clip.0,
        y: region.clip.1,
        w: region.clip.2,
        h: region.clip.3,
    });

    // Заливки. Объединённые ячейки пропускаются: их рисуют целиком ниже, поверх
    // сетки — иначе она просвечивала бы сквозь объединение.
    for row in region.rows.0..=region.rows.1 {
        for (col, cell) in visible_cells(sheet, row, region.cols) {
            if sheet.merges.covering(cell.at(row)).is_some() {
                continue;
            }
            let format = book.styles().resolve(cell.style);
            let Some(color) = fill_color(book, format.fill) else {
                continue;
            };
            let x = region.screen_x(layout.column_x(col), scale);
            let y = region.screen_y(layout.row_y(row), scale);
            let w = layout.column_width(col) * scale;
            let h = layout.row_height(row) * scale;
            fill_rect(out, x, y, w, h, color);
        }
    }

    if options.show_grid {
        draw_grid(layout, scale, region, options.grid, out);
    }

    for range in sheet.merges.ranges() {
        if range.last.row < region.rows.0
            || range.first.row > region.rows.1
            || range.last.col < region.cols.0
            || range.first.col > region.cols.1
        {
            continue;
        }
        let x = region.screen_x(layout.column_x(range.first.col), scale);
        let y = region.screen_y(layout.row_y(range.first.row), scale);
        let w = (layout.column_x(range.last.col + 1) - layout.column_x(range.first.col)) * scale;
        let h = (layout.row_y(range.last.row + 1) - layout.row_y(range.first.row)) * scale;

        let anchor = sheet.cells.cell(range.first);
        let format = anchor.map(|cell| book.styles().resolve(cell.style));
        let color = format
            .and_then(|format| fill_color(book, format.fill))
            .unwrap_or(options.background);
        fill_rect(out, x, y, w, h, color);

        if let (Some(cell), Some(format)) = (anchor, format) {
            draw_text(book, cell, format, x, y, w, h, scale, &TextClip::Rect, out);
        }
    }

    for row in region.rows.0..=region.rows.1 {
        for (col, cell) in visible_cells(sheet, row, region.cols) {
            if sheet.merges.covering(cell.at(row)).is_some() {
                continue;
            }
            let format = book.styles().resolve(cell.style);
            let x = region.screen_x(layout.column_x(col), scale);
            let y = region.screen_y(layout.row_y(row), scale);
            let w = layout.column_width(col) * scale;
            let h = layout.row_height(row) * scale;

            // Excel пускает текст в соседние ячейки, пока те пусты, и обрезает
            // его, как только рядом есть содержимое. Числа он не выпускает
            // никогда: не помещается — показывает «#####» (см. `draw_text`).
            let align = horizontal(format.alignment.horizontal, &cell.value);
            let clip = if align == HorizontalAlign::Right
                || !next_is_free(sheet, row, col, region.cols.1)
            {
                TextClip::Rect
            } else {
                TextClip::None
            };
            draw_text(book, cell, format, x, y, w, h, scale, &clip, out);
        }
    }

    out.push(DrawCommand::PopClip);
}

/// Сетка квадранта: линии по границам видимых столбцов и строк.
fn draw_grid(
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    color: Color,
    out: &mut DisplayList,
) {
    let left = region.screen_x(layout.column_x(region.cols.0), scale);
    let right = region.screen_x(layout.column_x(region.cols.1 + 1), scale);
    let top = region.screen_y(layout.row_y(region.rows.0), scale);
    let bottom = region.screen_y(layout.row_y(region.rows.1 + 1), scale);

    for row in region.rows.0..=region.rows.1 + 1 {
        let y = region.screen_y(layout.row_y(row), scale);
        out.push(DrawCommand::Line {
            x1: left,
            y1: y,
            x2: right,
            y2: y,
            stroke: color,
            stroke_w: 1.0,
            style: LineStyle::Solid,
        });
    }
    for col in region.cols.0..=region.cols.1 + 1 {
        let x = region.screen_x(layout.column_x(col), scale);
        out.push(DrawCommand::Line {
            x1: x,
            y1: top,
            x2: x,
            y2: bottom,
            stroke: color,
            stroke_w: 1.0,
            style: LineStyle::Solid,
        });
    }
}

/// Геометрия полос заголовков: окно, закрепления и границы прокрутки.
struct Headers<'a> {
    layout: &'a SheetLayout,
    options: &'a PaintOptions,
    scale: f32,
    viewport: Viewport,
    header_w: f32,
    header_h: f32,
    frozen_cols: u32,
    frozen_rows: u32,
    view_left: f32,
    view_top: f32,
}

/// Полосы заголовков: буквы столбцов сверху, номера строк слева.
fn draw_headers(headers: &Headers<'_>, out: &mut DisplayList) {
    fill_rect(
        out,
        0.0,
        0.0,
        headers.viewport.width,
        headers.header_h,
        headers.options.header_background,
    );
    fill_rect(
        out,
        0.0,
        0.0,
        headers.header_w,
        headers.viewport.height,
        headers.options.header_background,
    );

    draw_column_headers(headers, out);
    draw_row_headers(headers, out);

    let line = Color(0xC0_C0_C0_FF);
    out.push(DrawCommand::Line {
        x1: 0.0,
        y1: headers.header_h,
        x2: headers.viewport.width,
        y2: headers.header_h,
        stroke: line,
        stroke_w: 1.0,
        style: LineStyle::Solid,
    });
    out.push(DrawCommand::Line {
        x1: headers.header_w,
        y1: 0.0,
        x2: headers.header_w,
        y2: headers.viewport.height,
        stroke: line,
        stroke_w: 1.0,
        style: LineStyle::Solid,
    });
}

/// Буквы столбцов: закреплённые стоят на месте, остальные едут с прокруткой.
fn draw_column_headers(headers: &Headers<'_>, out: &mut DisplayList) {
    let Headers {
        layout,
        options,
        scale,
        viewport,
        header_w,
        header_h,
        frozen_cols,
        frozen_rows: _,
        view_left,
        ..
    } = *headers;

    let frozen_w = layout.column_x(frozen_cols) * scale;
    let scroll_w = (viewport.width - header_w - frozen_w).max(0.0) / scale;
    let (first, last) = layout.columns_in(view_left, view_left + scroll_w);
    let size = 11.0 * PX_PER_POINT * scale;

    let strips = [
        // `then`, а не `then_some`: аргумент последнего вычисляется всегда,
        // и при отсутствии закрепления `frozen_cols - 1` уходил бы в минус.
        (frozen_cols > 0).then(|| (0, frozen_cols - 1, 0.0, header_w)),
        Some((first, last, view_left, header_w + frozen_w)),
    ];
    for (first, last, base, origin) in strips.into_iter().flatten() {
        for col in first..=last {
            let x = origin + (layout.column_x(col) - base) * scale;
            let w = layout.column_width(col) * scale;
            if w <= 0.0 || x + w < header_w {
                continue;
            }
            let visible_x = x.max(header_w);
            let visible_w = (x + w - visible_x).min(viewport.width - visible_x);
            if visible_w <= 0.0 {
                continue;
            }
            let text = out.intern(&column_name(col));
            let font = out.intern("Calibri");
            out.push(DrawCommand::PushClip {
                x: visible_x,
                y: 0.0,
                w: visible_w,
                h: header_h,
            });
            out.push(DrawCommand::Text {
                x: x + w / 2.0,
                y: header_h / 2.0,
                text,
                font,
                size,
                color: options.header_foreground,
                align: TextAlign::Center,
                baseline: TextBaseline::Middle,
                bold: false,
                italic: false,
                underline: false,
            });
            out.push(DrawCommand::PopClip);
        }
    }
}

/// Номера строк.
fn draw_row_headers(headers: &Headers<'_>, out: &mut DisplayList) {
    let Headers {
        layout,
        options,
        scale,
        viewport,
        header_w,
        header_h,
        frozen_cols: _,
        frozen_rows,
        view_top,
        ..
    } = *headers;

    let frozen_h = layout.row_y(frozen_rows) * scale;
    let scroll_h = (viewport.height - header_h - frozen_h).max(0.0) / scale;
    let (first, last) = layout.rows_in(view_top, view_top + scroll_h);
    let size = 11.0 * PX_PER_POINT * scale;

    let strips = [
        (frozen_rows > 0).then(|| (0, frozen_rows - 1, 0.0, header_h)),
        Some((first, last, view_top, header_h + frozen_h)),
    ];
    for (first, last, base, origin) in strips.into_iter().flatten() {
        for row in first..=last {
            let y = origin + (layout.row_y(row) - base) * scale;
            let h = layout.row_height(row) * scale;
            if h <= 0.0 || y + h < header_h {
                continue;
            }
            let text = out.intern(&row_name(row));
            let font = out.intern("Calibri");
            out.push(DrawCommand::Text {
                x: header_w - TEXT_PADDING * scale,
                y: y + h / 2.0,
                text,
                font,
                size,
                color: options.header_foreground,
                align: TextAlign::Right,
                baseline: TextBaseline::Middle,
                bold: false,
                italic: false,
                underline: false,
            });
        }
    }
}

/// Как обрезать текст ячейки.
enum TextClip {
    /// Выпустить за границы ячейки в пустых соседей.
    None,
    /// Обрезать по границам ячейки.
    Rect,
}

/// Текст одной ячейки.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    book: &Workbook,
    cell: &Cell,
    format: crate::model::CellFormat,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    scale: f32,
    clip: &TextClip,
    out: &mut DisplayList,
) {
    let Some(text) = display_text(book, cell, format.num_fmt) else {
        return;
    };
    if text.is_empty() || w <= 0.0 || h <= 0.0 {
        return;
    }

    let font = book.styles().font(format.font).cloned().unwrap_or_default();
    let size = font.size * PX_PER_POINT * scale;
    let color = resolve_color(book.theme(), font.color).unwrap_or(Color::BLACK);
    let padding = TEXT_PADDING * scale;

    // Числа Excel не выпускает за ячейку и не обрезает: не помещается —
    // показывает решётки. Даты и деньги — тоже числа.
    let text = if matches!(cell.value, CellValue::Number(_))
        && estimate_width(&text, size) > w - padding * 2.0
    {
        HASHES.to_owned()
    } else {
        text
    };

    let (align, tx) = match horizontal(format.alignment.horizontal, &cell.value) {
        HorizontalAlign::Center | HorizontalAlign::CenterContinuous => {
            (TextAlign::Center, x + w / 2.0)
        }
        HorizontalAlign::Right => (TextAlign::Right, x + w - padding),
        _ => (TextAlign::Left, x + padding),
    };
    let (baseline, ty) = match format.alignment.vertical {
        VerticalAlign::Top => (TextBaseline::Top, y + padding),
        VerticalAlign::Center => (TextBaseline::Middle, y + h / 2.0),
        _ => (TextBaseline::Bottom, y + h - padding),
    };

    if matches!(clip, TextClip::Rect) {
        out.push(DrawCommand::PushClip { x, y, w, h });
    }
    let text_ref = out.intern(&text);
    let font_ref = out.intern(&font.name);
    out.push(DrawCommand::Text {
        x: tx,
        y: ty,
        text: text_ref,
        font: font_ref,
        size,
        color,
        align,
        baseline,
        bold: font.bold,
        italic: font.italic,
        underline: font.underline,
    });
    if matches!(clip, TextClip::Rect) {
        out.push(DrawCommand::PopClip);
    }
}

/// Что Excel показывает вместо числа, которое не помещается в столбец.
const HASHES: &str = "#####";

/// Ширина строки в пикселях — оценка по метрикам Calibri.
///
/// Цифры в Calibri моноширинные, и вся система ширин Excel построена на ширине
/// нуля: 7 пикселей при 11 pt. Для чисел с их разделителями оценка поэтому
/// точна, а произвольный текст так измерять нельзя — впрочем, его Excel и не
/// заменяет решётками, а пускает в пустого соседа.
///
/// TODO (Фаза 4): настоящие метрики из шрифта книги.
fn estimate_width(text: &str, size_px: f32) -> f32 {
    let k = size_px / (11.0 * PX_PER_POINT);
    let sum: f32 = text
        .chars()
        .map(|ch| match ch {
            '0'..='9' => 7.0,
            '.' | ',' | ' ' => 3.5,
            '-' | '+' | '/' | ':' => 4.0,
            _ => 7.5,
        })
        .sum();
    sum * k
}

/// Текст ячейки так, как его показывает Excel.
///
/// Числа и даты проводит через формат: в файле лежит серийный номер, а видно
/// «31.01.2024». Строки и коды ошибок берутся как есть, логические — словами,
/// как их пишет Excel.
#[must_use]
pub fn display_text(book: &Workbook, cell: &Cell, num_fmt: u32) -> Option<String> {
    match &cell.value {
        CellValue::Empty => cell.formula.as_ref().map(|_| String::new()),
        CellValue::Number(value) => {
            let code = book
                .styles()
                .format_code(num_fmt)
                .unwrap_or(numfmt::GENERAL);
            Some(numfmt::format(*value, code, book.date1904()))
        }
        CellValue::Bool(value) => Some(if *value { "TRUE" } else { "FALSE" }.to_owned()),
        _ => cell.value.text(book.shared_strings()).map(str::to_owned),
    }
}

/// Выравнивание по горизонтали: явное из формата, иначе по типу значения.
fn horizontal(align: HorizontalAlign, value: &CellValue) -> HorizontalAlign {
    if align != HorizontalAlign::General {
        return align;
    }
    match value {
        CellValue::Number(_) => HorizontalAlign::Right,
        CellValue::Bool(_) => HorizontalAlign::Center,
        _ => HorizontalAlign::Left,
    }
}

/// Цвет заливки ячейки; `None` — заливки нет.
fn fill_color(book: &Workbook, fill: u32) -> Option<Color> {
    let fill = book.styles().fill(fill)?;
    match fill.pattern {
        crate::model::FillPattern::Solid => resolve_color(book.theme(), fill.foreground),
        crate::model::FillPattern::None => None,
        // Узоры Excel рисует растром; в DisplayList растра нет, поэтому узор
        // показывается своим цветом. TODO (Фаза 5): растр узора.
        _ => resolve_color(book.theme(), fill.foreground)
            .or_else(|| resolve_color(book.theme(), fill.background)),
    }
}

/// Цвет из модели в цвет `DisplayList`.
///
/// `theme` — палитра книги: без неё цвета вида `theme="n"` разрешить нечем.
///
/// `None` — цвет не задан, индекс вне палитры или палитра такого цвета не
/// знает.
pub fn resolve_color(theme: &Theme, color: CellColor) -> Option<Color> {
    match color {
        CellColor::None => None,
        CellColor::Rgb(value) => Some(Color(argb_to_rgba(value))),
        CellColor::Theme(index) => theme_color(theme, index),
        CellColor::Indexed(index) => indexed_color(index).map(Color),
    }
}

/// Цвет темы: палитра уже разрешила индекс до `Color::Rgb`, остаётся
/// переставить каналы. Пустой слот и индекс вне палитры дают `None`.
fn theme_color(theme: &Theme, index: u32) -> Option<Color> {
    match theme.color(index)? {
        CellColor::Rgb(value) => Some(Color(argb_to_rgba(value))),
        // Палитра хранит только RGB — прочие варианты означают пустой слот.
        CellColor::None | CellColor::Theme(_) | CellColor::Indexed(_) => None,
    }
}

/// Переставить каналы: в файле цвет записан как `AARRGGBB`, а `DisplayList`
/// ждёт `RRGGBBAA`.
///
/// Без этого заливки не просто меняли оттенок: альфа уезжала в синий канал,
/// и жёлтая заливка `FFFFFF00` становилась прозрачной.
fn argb_to_rgba(value: u32) -> u32 {
    let alpha = value >> 24;
    let rgb = value & 0x00FF_FFFF;
    (rgb << 8) | alpha
}

/// Устаревшая палитра Excel (`indexed`): индексы 0…63 — цвета BIFF8,
/// 64 — системный цвет текста, 65 — системный фон.
fn indexed_color(index: u32) -> Option<u32> {
    const PALETTE: [u32; 64] = [
        0x00_0000, 0xFF_FFFF, 0xFF_0000, 0x00_FF00, 0x00_00FF, 0xFF_FF00, 0xFF_00FF, 0x00_FFFF,
        0x00_0000, 0xFF_FFFF, 0xFF_0000, 0x00_FF00, 0x00_00FF, 0xFF_FF00, 0xFF_00FF, 0x00_FFFF,
        0x80_0000, 0x00_8000, 0x00_0080, 0x80_8000, 0x80_0080, 0x00_8080, 0xC0_C0C0, 0x80_8080,
        0x99_99FF, 0x99_3366, 0xFF_FFCC, 0xCC_FFFF, 0x66_0066, 0xFF_8080, 0x00_66CC, 0xCC_CCFF,
        0x00_0080, 0xFF_00FF, 0xFF_FF00, 0x00_FFFF, 0x80_0080, 0x80_0000, 0x00_8080, 0x00_00FF,
        0x00_CCFF, 0xCC_FFFF, 0xCC_FFCC, 0xFF_FF99, 0x99_CCFF, 0xFF_99CC, 0xCC_99FF, 0xFF_CC99,
        0x33_66FF, 0x33_CCCC, 0x99_CC00, 0xFF_CC00, 0xFF_9900, 0xFF_6600, 0x66_6699, 0x96_9696,
        0x00_3366, 0x33_9966, 0x00_3300, 0x33_3300, 0x99_3300, 0x99_3366, 0x33_3399, 0x33_3333,
    ];
    match index {
        0..=63 => Some(PALETTE[index as usize] << 8 | 0xFF),
        // Системные цвета: текст и фон окна.
        64 => Some(0x0000_00FF),
        65 => Some(0xFFFF_FFFF),
        _ => None,
    }
}

/// Ячейки строки в пределах показанных столбцов.
fn visible_cells(sheet: &Sheet, row: u32, cols: (u32, u32)) -> impl Iterator<Item = (u32, &Cell)> {
    let (first, last) = cols;
    sheet
        .cells
        .cells_of_row(row)
        .iter()
        .filter(move |cell| cell.col >= first && cell.col <= last)
        .map(|cell| (cell.col, cell))
}

/// Свободна ли ячейка справа: туда Excel пускает текст.
fn next_is_free(sheet: &Sheet, row: u32, col: u32, last_col: u32) -> bool {
    if col >= last_col {
        return false;
    }
    let cells = sheet.cells.cells_of_row(row);
    let Ok(index) = cells.binary_search_by_key(&(col + 1), |cell| cell.col) else {
        return true;
    };
    matches!(cells[index].value, CellValue::Empty)
}

/// Прямоугольник одним `Rect` без рамки и скруглений.
fn fill_rect(out: &mut DisplayList, x: f32, y: f32, w: f32, h: f32, fill: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    out.push(DrawCommand::Rect {
        x,
        y,
        w,
        h,
        fill,
        stroke: Color::TRANSPARENT,
        stroke_w: 0.0,
        radius: [0.0; 4],
    });
}

#[cfg(test)]
// Координаты в тестах — точные литералы, представимые в `f32`.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::model::{Border, CellFormat, Fill, FillPattern, Font, StyleTable, WorksheetMeta};
    use crate::model::{SheetContent, Theme, WorksheetBuilder};
    use crate::SharedStrings;
    use crate::SheetState;
    use std::collections::BTreeMap;

    const PART: &str = "xl/worksheets/sheet1.xml";

    /// Книга с одним листом, собранным вызывающим.
    fn book_with(content: SheetContent, styles: StyleTable) -> Workbook {
        book_with_theme(content, styles, Theme::default())
    }

    /// Книга с темой: цвета `theme="n"` разрешаются через неё.
    fn book_with_theme(content: SheetContent, styles: StyleTable, theme: Theme) -> Workbook {
        let sheet = Sheet::new(
            WorksheetMeta {
                name: "Лист1".into(),
                part: PART.into(),
                state: SheetState::Visible,
            },
            content,
        );
        Workbook::new(vec![sheet], SharedStrings::default(), styles, theme, false)
    }

    /// Тема Excel: `dk1` записан раньше `lt1`, а `theme="1"` — это `dk1`.
    fn theme() -> Theme {
        crate::theme::parse(
            br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                  <a:themeElements>
                    <a:clrScheme>
                      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
                      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
                      <a:dk2><a:srgbClr val="1F497D"/></a:dk2>
                      <a:lt2><a:srgbClr val="EEECE1"/></a:lt2>
                      <a:accent1><a:srgbClr val="4F81BD"/></a:accent1>
                    </a:clrScheme>
                  </a:themeElements>
                </a:theme>"#,
            "xl/theme/theme1.xml",
        )
        .unwrap()
    }

    fn painted(book: &Workbook, viewport: Viewport, options: &PaintOptions) -> DisplayList {
        let mut dl = DisplayList::new();
        build(book, &book.sheets()[0], viewport, options, &mut dl);
        dl
    }

    /// Сколько команд каждого вида.
    fn count(dl: &DisplayList, want: fn(&DrawCommand) -> bool) -> usize {
        (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter(|cmd| want(cmd))
            .count()
    }

    fn numbers() -> SheetContent {
        let mut builder = WorksheetBuilder::new(PART);
        for row in 0..5 {
            for col in 0..3 {
                builder
                    .push(
                        row,
                        Cell::new(col, 0, CellValue::Number(f64::from(row * 3 + col))),
                    )
                    .unwrap();
            }
        }
        SheetContent {
            cells: builder.finish(),
            ..SheetContent::default()
        }
    }

    #[test]
    fn repeated_build_does_not_pile_up() {
        let book = book_with(numbers(), StyleTable::default());
        let mut dl = DisplayList::new();

        build(
            &book,
            &book.sheets()[0],
            Viewport::default(),
            &PaintOptions::default(),
            &mut dl,
        );
        let once = dl.len();
        build(
            &book,
            &book.sheets()[0],
            Viewport::default(),
            &PaintOptions::default(),
            &mut dl,
        );

        assert_eq!(dl.len(), once, "кадр собрался поверх предыдущего");
    }

    #[test]
    fn empty_sheet_still_fills_the_window() {
        let book = book_with(SheetContent::default(), StyleTable::default());
        let dl = painted(&book, Viewport::default(), &PaintOptions::default());

        assert!(matches!(dl.cmd(0), Some(DrawCommand::Clear)));
        match dl.cmd(1) {
            Some(DrawCommand::Rect { fill, w, h, .. }) => {
                assert_eq!(*fill, Color::WHITE);
                assert_eq!((*w, *h), (800.0, 600.0));
            }
            other => panic!("ожидался фон окна, а не {other:?}"),
        }
    }

    #[test]
    fn paints_only_visible_cells() {
        let book = book_with(numbers(), StyleTable::default());
        // Окно ровно на две строки и два столбца: 128×40 пикселей.
        let viewport = Viewport {
            width: 128.0,
            height: 40.0,
            ..Viewport::default()
        };
        let options = PaintOptions {
            show_headers: false,
            show_grid: false,
            ..PaintOptions::default()
        };
        let dl = painted(&book, viewport, &options);

        let texts: Vec<String> = (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Text { text, .. } => Some(dl.string(*text).to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["0", "1", "3", "4"]);

        // Прокрутка на строку вниз показывает следующую строку.
        let scrolled = painted(
            &book,
            Viewport {
                scroll_y: 20.0,
                ..viewport
            },
            &options,
        );
        let texts: Vec<String> = (0..scrolled.len())
            .filter_map(|i| scrolled.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Text { text, .. } => Some(scrolled.string(*text).to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["3", "4", "6", "7"]);
    }

    #[test]
    fn grid_and_headers_can_be_turned_off() {
        let book = book_with(numbers(), StyleTable::default());

        let with = painted(&book, Viewport::default(), &PaintOptions::default());
        let lines = count(&with, |cmd| matches!(cmd, DrawCommand::Line { .. }));
        assert!(lines > 0);

        let bare = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );
        assert_eq!(
            count(&bare, |cmd| matches!(cmd, DrawCommand::Line { .. })),
            0
        );
        // Без заголовков в текстах только значения ячеек — ни «A», ни «1».
        let texts: Vec<String> = (0..bare.len())
            .filter_map(|i| bare.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Text { text, .. } => Some(bare.string(*text).to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 15);
    }

    #[test]
    fn numbers_are_formatted_and_aligned_right() {
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    num_fmt: 10,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default()],
            vec![Fill::default()],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(0.1234)))
            .unwrap();
        let book = book_with(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
        );

        let dl = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );
        let first_text = (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .find(|cmd| matches!(cmd, DrawCommand::Text { .. }));
        match first_text {
            Some(DrawCommand::Text {
                text, align, bold, ..
            }) => {
                assert_eq!(dl.string(*text), "12.34%");
                // Число без явного выравнивания прижимается вправо.
                assert_eq!(*align, TextAlign::Right);
                assert!(!*bold);
            }
            other => panic!("ожидался текст, а не {other:?}"),
        }
    }

    #[test]
    fn merged_range_is_painted_once_and_hides_the_grid() {
        let mut content = numbers();
        content.merges.push(crate::Range::parse("A1:B2").unwrap());
        let book = book_with(content, StyleTable::default());

        let dl = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_headers: false,
                ..PaintOptions::default()
            },
        );

        // Значение первой ячейки нарисовано, значения B1 и A2 — нет.
        let texts: Vec<String> = (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Text { text, .. } => Some(dl.string(*text).to_owned()),
                _ => None,
            })
            .collect();
        assert!(!texts.contains(&"1".to_owned()), "B1 внутри объединения");
        assert!(!texts.contains(&"3".to_owned()), "A2 внутри объединения");
        assert!(texts.contains(&"2".to_owned()), "C1 вне объединения");

        // Фон объединения нарисован после сетки — поверх её линий.
        let mut seen_merge_rect = false;
        for i in 0..dl.len() {
            let Some(DrawCommand::Rect { x, y, w, h, .. }) = dl.cmd(i) else {
                continue;
            };
            if *x == 0.0 && *y == 0.0 && *w == 128.0 && *h == 40.0 {
                seen_merge_rect = true;
            }
        }
        assert!(seen_merge_rect, "фон объединения не нарисован");
    }

    #[test]
    fn frozen_pane_keeps_the_top_row_in_place() {
        let mut content = numbers();
        content.view.pane = Some(crate::Pane {
            cols: 0,
            rows: 1,
            state: crate::PaneState::Frozen,
            active: crate::PaneKind::BottomLeft,
            top_left: None,
        });
        let book = book_with(content, StyleTable::default());

        let viewport = Viewport {
            scroll_y: 20.0,
            width: 128.0,
            height: 60.0,
            ..Viewport::default()
        };
        let dl = painted(
            &book,
            viewport,
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );

        // Первая строка закреплена и остаётся на месте при прокрутке.
        let texts: Vec<(String, f32)> = (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Text { text, y, .. } => Some((dl.string(*text).to_owned(), *y)),
                _ => None,
            })
            .collect();
        let first = texts.iter().find(|(text, _)| text == "0").unwrap();
        assert_eq!(first.1, 20.0 - TEXT_PADDING, "закреплённая строка уехала");

        // Прокрутка отсчитывается от низа закреплённой части: сдвиг на строку
        // показывает третью строку листа, а не вторую.
        assert!(texts.iter().any(|(text, _)| text == "6"));
        assert!(
            !texts.iter().any(|(text, _)| text == "3"),
            "вторая строка должна была прокрутиться"
        );
    }

    #[test]
    fn solid_fill_becomes_a_rect() {
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    fill: 1,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default()],
            vec![
                Fill::default(),
                Fill {
                    pattern: FillPattern::Solid,
                    foreground: crate::model::Color::Rgb(0xFFFF_0000),
                    background: crate::model::Color::None,
                },
            ],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(1.0)))
            .unwrap();
        let book = book_with(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
        );
        let dl = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );

        // В файле цвет записан как `FFFF0000` — красный с непрозрачной альфой;
        // в DisplayList он уже `FF0000FF`.
        let filled = count(
            &dl,
            |cmd| matches!(cmd, DrawCommand::Rect { fill, .. } if *fill == Color(0xFF00_00FF)),
        );
        assert_eq!(filled, 1);
    }

    #[test]
    fn number_too_wide_for_the_column_becomes_hashes() {
        /// Лист из одной ячейки с числом и форматом «0.00».
        fn book(width: f32) -> Workbook {
            let styles = StyleTable::new(
                vec![
                    CellFormat::default(),
                    CellFormat {
                        num_fmt: 2,
                        ..CellFormat::default()
                    },
                ],
                vec![Font::default()],
                vec![Fill::default()],
                vec![Border::default()],
                BTreeMap::new(),
            );
            let mut builder = WorksheetBuilder::new(PART);
            builder
                .push(0, Cell::new(0, 1, CellValue::Number(45_002.0)))
                .unwrap();
            let mut content = SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            };
            content.dims.cols.push(crate::ColWidth {
                first: 0,
                last: 0,
                width,
                custom: true,
                hidden: false,
                best_fit: false,
            });
            book_with(content, styles)
        }

        fn texts(book: &Workbook) -> Vec<String> {
            let dl = painted(
                book,
                Viewport::default(),
                &PaintOptions {
                    show_grid: false,
                    show_headers: false,
                    ..PaintOptions::default()
                },
            );
            (0..dl.len())
                .filter_map(|i| dl.cmd(i))
                .filter_map(|cmd| match cmd {
                    DrawCommand::Text { text, .. } => Some(dl.string(*text).to_owned()),
                    _ => None,
                })
                .collect()
        }

        // «45002.00» — восемь цифр с точкой: в 33 пикселя не влезает.
        assert_eq!(texts(&book(4.0)), vec!["#####"]);
        // В столбце по умолчанию (64 пикселя) — влезает с запасом.
        assert_eq!(texts(&book(8.43)), vec!["45002.00"]);
    }

    #[test]
    fn underline_from_the_font_reaches_the_frame() {
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    font: 1,
                    ..CellFormat::default()
                },
            ],
            vec![
                Font::default(),
                Font {
                    underline: true,
                    ..Font::default()
                },
            ],
            vec![Fill::default()],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 1, CellValue::InlineString("link".into())))
            .unwrap();
        let book = book_with(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
        );

        let dl = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );
        assert_eq!(
            count(
                &dl,
                |cmd| matches!(cmd, DrawCommand::Text { underline, .. } if *underline)
            ),
            1
        );
    }

    #[test]
    fn colors_are_reordered_from_argb_to_rgba() {
        // Аргументы — то, что записано в файле: AARRGGBB.
        let none = Theme::default();
        let white = |value: u32| {
            resolve_color(&none, CellColor::Rgb(value))
                .unwrap()
                .to_css()
        };

        assert_eq!(white(0xFF_FF_FF_FF), "#ffffff");
        assert_eq!(white(0xFF_00_00_00), "#000000");
        // Голубой: без перестановки каналов получился бы пурпурный.
        assert_eq!(white(0xFF_00_FF_FF), "#00ffff");
        // Жёлтый: без перестановки альфа уехала бы в синий и заливка исчезла.
        assert_eq!(white(0xFF_FF_FF_00), "#ffff00");
        // Полупрозрачный красный.
        assert_eq!(white(0x80_FF_00_00), "rgba(255,0,0,0.502)");
    }

    #[test]
    fn indexed_palette_is_resolved() {
        let none = Theme::default();
        assert_eq!(
            resolve_color(&none, CellColor::Indexed(0)),
            Some(Color::BLACK)
        );
        assert_eq!(
            resolve_color(&none, CellColor::Indexed(2)),
            Some(Color(0xFF00_00FF))
        );
        assert_eq!(
            resolve_color(&none, CellColor::Indexed(64)),
            Some(Color::BLACK)
        );
        assert_eq!(
            resolve_color(&none, CellColor::Indexed(65)),
            Some(Color::WHITE)
        );
        assert_eq!(resolve_color(&none, CellColor::Indexed(200)), None);
        // Пустая палитра ничего не разрешает — выдумывать цвета нельзя.
        assert_eq!(resolve_color(&none, CellColor::Theme(4)), None);
        assert_eq!(resolve_color(&none, CellColor::None), None);
    }

    #[test]
    fn theme_colors_are_resolved_through_the_palette() {
        let theme = theme();

        assert_eq!(
            resolve_color(&theme, CellColor::Theme(1)),
            Some(Color::BLACK),
            "1 — dk1, текст по умолчанию, а не lt1"
        );
        assert_eq!(
            resolve_color(&theme, CellColor::Theme(0)),
            Some(Color::WHITE)
        );
        // accent1 = 4F81BD; в DisplayList каналы уже переставлены.
        assert_eq!(
            resolve_color(&theme, CellColor::Theme(4)),
            Some(Color(0x4F81_BDFF))
        );
        assert_eq!(resolve_color(&theme, CellColor::Theme(12)), None);
        // Слот, которого нет в теме, тоже не разрешается.
        assert_eq!(resolve_color(&theme, CellColor::Theme(9)), None);
    }

    #[test]
    fn default_theme_font_color_paints_black_text() {
        // Главная ловушка темы: `<color theme="1"/>` — цвет текста по
        // умолчанию. Если индексировать палитру в порядке XML (`dk1` первым),
        // `theme="1"` укажет на `lt1`, и текст станет белым — исчезнет.
        let styles = StyleTable::new(
            vec![CellFormat::default()],
            vec![Font {
                color: CellColor::Theme(1),
                ..Font::default()
            }],
            vec![Fill::default()],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 0, CellValue::InlineString("текст".into())))
            .unwrap();
        let book = book_with_theme(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
            theme(),
        );

        let dl = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );

        let text = (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .find_map(|cmd| match cmd {
                DrawCommand::Text { color, .. } => Some(*color),
                _ => None,
            });
        assert_eq!(text, Some(Color::BLACK));
    }

    #[test]
    fn solid_fill_with_theme_color_is_painted() {
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    fill: 1,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default()],
            vec![
                Fill::default(),
                Fill {
                    pattern: FillPattern::Solid,
                    foreground: CellColor::Theme(4),
                    background: CellColor::None,
                },
            ],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(1.0)))
            .unwrap();
        let book = book_with_theme(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
            theme(),
        );
        let dl = painted(
            &book,
            Viewport::default(),
            &PaintOptions {
                show_grid: false,
                show_headers: false,
                ..PaintOptions::default()
            },
        );

        // accent1 = 4F81BD, непрозрачный: в DisplayList это `4F81BDFF`.
        let filled = count(
            &dl,
            |cmd| matches!(cmd, DrawCommand::Rect { fill, .. } if *fill == Color(0x4F81_BDFF)),
        );
        assert_eq!(filled, 1);
    }
}
