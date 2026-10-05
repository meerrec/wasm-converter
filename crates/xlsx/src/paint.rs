//! Лист в команды рисования: сетка, заливки, текст, заголовки.
//!
//! Координаты `DisplayList` — физические пиксели canvas: масштаб (зум, умноженный
//! на DPR) применяется здесь, а раскладка считается без него. Порядок слоёв
//! повторяет Excel: заливки, поверх них сетка, поверх всего текст, а над текстом —
//! картинки: изображения в Excel — отдельный слой объектов, плавающий над
//! ячейками. Объединённые ячейки рисуются после сетки — иначе она просвечивала бы
//! сквозь них.
//!
//! Закреплённые области разбивают окно на четыре квадранта: закреплённые строки
//! и столбцы не прокручиваются, остальное — да. Каждый квадрант рисуется под
//! своим `PushClip`, поэтому содержимое не выползает за его границы.

use std::cell::RefCell;

use doc_converter_render::display_list::{
    Color, DisplayList, DrawCommand, LineStyle, TextAlign, TextBaseline,
};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_ID};
use doc_converter_render::text_measure::measure_text;
use doc_converter_render::viewport::Viewport;

use crate::cellref::{column_name, row_name, CellRef, Range};
use crate::conditional::{EffectiveStyle, RuleIndex, Visual};
use crate::drawing::ImageAnchor;
use crate::layout::{SheetLayout, PX_PER_POINT};
use crate::model::{
    Border, BorderSide, BorderStyle, Cell, CellValue, Color as CellColor, HorizontalAlign, Sheet,
    Theme, VerticalAlign,
};
use crate::numfmt;
use crate::Workbook;

/// Ширина полосы заголовков строк, пиксели раскладки.
pub const ROW_HEADER_WIDTH: f32 = 44.0;
/// Высота полосы заголовков столбцов.
pub const COL_HEADER_HEIGHT: f32 = 20.0;
/// Поля текста внутри ячейки.
const TEXT_PADDING: f32 = 3.0;
/// Индекс слота `hlink` в палитре темы (`SpreadsheetML`): на него ссылается
/// встроенный стиль Excel «Hyperlink».
const HLINK_THEME_INDEX: u32 = 10;

thread_local! {
    /// Метрики текста — один реестр на поток. Шрифт по умолчанию (Carlito,
    /// метрически совместимый с Calibri) зарегистрирован в `new`, и кадры
    /// переиспользуют тёплый LRU-кэш глифов.
    ///
    /// Все ячейки меряются шрифтом по умолчанию: байтов шрифта книги в пакете
    /// нет, а политика «что делать при промахе» зафиксирована в ADR-0005 —
    /// метрики обязаны быть детерминированными на любой машине.
    static FONTS: RefCell<FontRegistry> = RefCell::new(FontRegistry::new(4096));
}

/// Что рисовать поверх содержимого.
///
/// Цвета — `Option`: `None` означает «взять у оформления» (светлого или тёмного,
/// см. [`PaintOptions::dark`]), явный цвет его перекрывает.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintOptions {
    /// Показывать сетку.
    pub show_grid: bool,
    /// Показывать заголовки строк и столбцов.
    pub show_headers: bool,
    /// Тёмное оформление: фон окна, сетка и заголовки темнеют, палитра темы
    /// инвертируется (`lt1`↔`dk1`, `lt2`↔`dk2`), а текст, неразличимый на своём
    /// фоне, поднимается до читаемого контраста.
    ///
    /// Явные RGB-цвета ячеек не инвертируются: инверсия меняла бы оттенок и
    /// ломала осмысленные пары «текст на заливке» (чёрный текст на светлой
    /// заливке стал бы белым на тёмной). Допущение: Excel тёмного оформления
    /// листа не документирует; правило задаёт просмотрщик.
    pub dark: bool,
    /// Фон листа; `None` — фон оформления.
    pub background: Option<Color>,
    /// Цвет сетки; `None` — цвет оформления.
    pub grid: Option<Color>,
    /// Фон заголовков; `None` — фон оформления.
    pub header_background: Option<Color>,
    /// Цвет текста заголовков; `None` — цвет оформления.
    pub header_foreground: Option<Color>,
    /// Линия между заголовками и листом; `None` — цвет оформления.
    pub header_line: Option<Color>,
}

/// Цвета оформления: светлые — как у Excel, тёмные — просмотрщика.
mod dl_color {
    use super::Color;

    pub const WHITE: Color = Color::WHITE;
    pub const GRID: Color = Color(0xD9_D9_D9_FF);
    pub const HEADER_BG: Color = Color(0xF5_F5_F5_FF);
    pub const HEADER_FG: Color = Color(0x44_44_44_FF);
    pub const HEADER_LINE: Color = Color(0xC0_C0_C0_FF);

    pub const DARK_BACKGROUND: Color = Color(0x1E_1E_1E_FF);
    pub const DARK_GRID: Color = Color(0x3A_3A_3A_FF);
    pub const DARK_HEADER_BG: Color = Color(0x2A_2A_2A_FF);
    pub const DARK_HEADER_FG: Color = Color(0xC8_C8_C8_FF);
    pub const DARK_HEADER_LINE: Color = Color(0x50_50_50_FF);
    /// Текст, которому цвет не задан в файле: соответствует `lt1` палитры.
    pub const DARK_TEXT: Color = Color(0xE8_E8_E8_FF);
}

impl PaintOptions {
    /// Фон листа: заданный цвет или фон оформления.
    fn effective_background(&self) -> Color {
        self.background.unwrap_or(if self.dark {
            dl_color::DARK_BACKGROUND
        } else {
            dl_color::WHITE
        })
    }

    /// Цвет сетки: заданный или из оформления.
    fn effective_grid(&self) -> Color {
        self.grid.unwrap_or(if self.dark {
            dl_color::DARK_GRID
        } else {
            dl_color::GRID
        })
    }

    /// Фон заголовков: заданный или из оформления.
    fn effective_header_background(&self) -> Color {
        self.header_background.unwrap_or(if self.dark {
            dl_color::DARK_HEADER_BG
        } else {
            dl_color::HEADER_BG
        })
    }

    /// Цвет текста заголовков: заданный или из оформления.
    fn effective_header_foreground(&self) -> Color {
        self.header_foreground.unwrap_or(if self.dark {
            dl_color::DARK_HEADER_FG
        } else {
            dl_color::HEADER_FG
        })
    }

    /// Линия между заголовками и листом: заданная или из оформления.
    fn effective_header_line(&self) -> Color {
        self.header_line.unwrap_or(if self.dark {
            dl_color::DARK_HEADER_LINE
        } else {
            dl_color::HEADER_LINE
        })
    }

    /// Цвет текста, которому цвет не задан в файле: чёрный в светлом
    /// оформлении, почти белый — в тёмном.
    fn default_text(&self) -> Color {
        if self.dark {
            dl_color::DARK_TEXT
        } else {
            Color::BLACK
        }
    }
}

impl Default for PaintOptions {
    fn default() -> Self {
        Self {
            show_grid: true,
            show_headers: true,
            dark: false,
            background: None,
            grid: None,
            header_background: None,
            header_foreground: None,
            header_line: None,
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
            view_left: layout.column_x(frozen_cols) + viewport.x.max(0.0),
            view_top: layout.row_y(frozen_rows) + viewport.y.max(0.0),
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
/// листа, и возвращать «никуда» было бы неудобно выделению. Маппинг «точка →
/// ячейка» остаётся раскладочным адаптером: `DisplayList` не несёт идентичности
/// ячеек, а геометрия попадания по командам кадра (картинки, текст) живёт в
/// `render::hit_test` (ADR-0003).
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
    /// Правый и нижний края видимой части квадранта в координатах раскладки:
    /// у закреплённой части — граница закрепления, у прокручиваемой — край окна.
    /// Закреплённый квадрант не должен рисовать то, что лежит под прокручиваемым.
    pane_right: f32,
    pane_bottom: f32,
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

    // Условное форматирование: индекс строится раз на кадр, в нём только
    // правила, меняющие оформление.
    let rules = RuleIndex::new(book, sheet);

    // Список команд переиспользуется между кадрами: без очистки следующий
    // кадр лёг бы поверх предыдущего, и картинка задвоилась бы.
    out.clear();
    out.push(DrawCommand::Clear);
    fill_rect(
        out,
        0.0,
        0.0,
        viewport.w,
        viewport.h,
        options.effective_background(),
    );

    let scroll_w = (viewport.w - header_w - frozen_w).max(0.0) / scale;
    let scroll_h = (viewport.h - header_h - frozen_h).max(0.0) / scale;
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

        // Закреплённая часть кончается на границе закрепления, прокручиваемая —
        // на краю окна; у не закреплённых осей обе границы совпадают.
        let pane_right = if cols.0 < frozen_cols {
            layout.column_x(frozen_cols)
        } else {
            view_left + scroll_w
        };
        let pane_bottom = if rows.0 < frozen_rows {
            layout.row_y(frozen_rows)
        } else {
            view_top + scroll_h
        };

        let region = Region {
            rows,
            cols,
            screen_x,
            screen_y,
            layout_x,
            layout_y,
            pane_right,
            pane_bottom,
            clip: (
                screen_x,
                screen_y,
                (viewport.w - screen_x).max(0.0),
                (viewport.h - screen_y).max(0.0),
            ),
        };
        draw_region(book, sheet, &rules, &layout, scale, &region, options, out);
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

/// Фон, заливки, сетка, границы и текст одного квадранта.
#[allow(clippy::too_many_arguments)]
fn draw_region(
    book: &Workbook,
    sheet: &Sheet,
    rules: &RuleIndex<'_>,
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
            let style = rules.style_at(book, cell.at(row));
            let Some(color) = fill_color(book, &style, options) else {
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
        draw_grid(layout, scale, region, options.effective_grid(), out);
    }

    // Объединённые ячейки: заливка закрывает сетку изнутри диапазона.
    for range in visible_merges(sheet, region) {
        let (x, y, w, h) = range_rect(layout, scale, region, range);
        let anchor = sheet.cells.cell(range.first);
        let style = anchor.map(|_| rules.style_at(book, range.first));
        let color = style
            .as_ref()
            .and_then(|style| fill_color(book, style, options))
            .unwrap_or(options.effective_background());
        fill_rect(out, x, y, w, h, color);
    }

    // Полосы данных и значки — поверх сетки и заливок, но под границами и
    // текстом: изображение правила не должно закрывать ни рамку ячейки, ни её
    // содержимое.
    if rules.has_visuals() {
        draw_visuals(book, sheet, rules, layout, scale, region, out);
    }

    // Границы — поверх сетки и заливок: в Excel рамка замещает сетку на своём
    // ребре.
    draw_borders(book, sheet, rules, layout, scale, region, options, out);

    // Текст — поверх рамок: он выпускается в пустых соседей и перечёркивался бы
    // их границами.
    for range in visible_merges(sheet, region) {
        let (x, y, w, h) = range_rect(layout, scale, region, range);
        if let Some(cell) = sheet.cells.cell(range.first) {
            let style = rules.style_at(book, range.first);
            // Значок с `showValue="0"` заменяет значение, а не дополняет его.
            if style.hides_value() {
                continue;
            }
            let is_link = sheet.hyperlink_at(range.first).is_some();
            draw_text(
                book,
                cell,
                &style,
                x,
                y,
                w,
                h,
                scale,
                &TextClip::Rect,
                is_link,
                options,
                out,
            );
        }
    }

    for row in region.rows.0..=region.rows.1 {
        for (col, cell) in visible_cells(sheet, row, region.cols) {
            if sheet.merges.covering(cell.at(row)).is_some() {
                continue;
            }
            let style = rules.style_at(book, cell.at(row));
            // Значок с `showValue="0"` заменяет значение, а не дополняет его.
            if style.hides_value() {
                continue;
            }
            let x = region.screen_x(layout.column_x(col), scale);
            let y = region.screen_y(layout.row_y(row), scale);
            let w = layout.column_width(col) * scale;
            let h = layout.row_height(row) * scale;

            // Excel пускает текст в соседние ячейки, пока те пусты, и обрезает
            // его, как только рядом есть содержимое. Числа он не выпускает
            // никогда: не помещается — показывает «#####» (см. `draw_text`).
            let align = horizontal(style.base().alignment.horizontal, &cell.value);
            let clip = if align == HorizontalAlign::Right
                || !next_is_free(sheet, row, col, region.cols.1)
            {
                TextClip::Rect
            } else {
                TextClip::None
            };
            let is_link = sheet.hyperlink_at(cell.at(row)).is_some();
            draw_text(
                book, cell, &style, x, y, w, h, scale, &clip, is_link, options, out,
            );
        }
    }

    // Картинки — над текстом: в Excel объект лежит на отдельном слое поверх
    // ячеек и закрывает их содержимое. Выше текста, но ниже `PopClip`: клип
    // квадранта обрезает картинку так же, как остальное содержимое.
    draw_images(sheet, layout, scale, region, out);

    out.push(DrawCommand::PopClip);
}

/// Прямоугольник картинки в координатах раскладки: `(x, y, w, h)`.
///
/// `from` — верхний левый угол, `to` — нижний правый (в OOXML он исключающий,
/// то есть задаёт границу, а не последний пиксель). Повреждённый якорь может
/// дать отрицательный размер — рисование такие пропускает.
fn anchor_rect(layout: &SheetLayout, anchor: ImageAnchor) -> (f32, f32, f32, f32) {
    let (from, to) = match anchor {
        ImageAnchor::OneCell { from, ext } => {
            let x = layout.column_x(from.col) + from.col_off;
            let y = layout.row_y(from.row) + from.row_off;
            return (x, y, ext.cx, ext.cy);
        }
        ImageAnchor::TwoCell { from, to } => (from, to),
    };
    let x = layout.column_x(from.col) + from.col_off;
    let y = layout.row_y(from.row) + from.row_off;
    let x1 = layout.column_x(to.col) + to.col_off;
    let y1 = layout.row_y(to.row) + to.row_off;
    (x, y, x1 - x, y1 - y)
}

/// Картинки одного квадранта — в порядке наложения.
///
/// Порядок `sheet.images` — порядок документа: первые лежат ниже, и painter
/// закрашивает их следующими командами. Картинка без `image_id` не рисуется:
/// media не разрешилась, регистрировать нечего.
///
/// Видимая часть — пересечение прямоугольника с квадрантом. Это отсекает и
/// картинки за окном, и картинку, пересекающую границу закрепления: в каждом
/// квадранте рисуется только его часть — закреплённая остаётся на месте,
/// прокручиваемая уезжает.
fn draw_images(
    sheet: &Sheet,
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    out: &mut DisplayList,
) {
    for image in &sheet.images {
        let Some(bitmap_id) = image.image_id else {
            continue;
        };
        let (x, y, w, h) = anchor_rect(layout, image.anchor);
        let x0 = x.max(region.layout_x);
        let y0 = y.max(region.layout_y);
        let x1 = (x + w).min(region.pane_right);
        let y1 = (y + h).min(region.pane_bottom);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        out.push(DrawCommand::Image {
            x: region.screen_x(x0, scale),
            y: region.screen_y(y0, scale),
            w: (x1 - x0) * scale,
            h: (y1 - y0) * scale,
            bitmap_id,
        });
    }
}

/// Высота полосы данных от высоты строки; поля сверху и снизу.
const BAR_HEIGHT_RATIO: f32 = 0.7;
/// Размер значка от высоты строки.
const ICON_HEIGHT_RATIO: f32 = 0.7;

/// Полосы данных и значки видимых ячеек.
///
/// Объединённые ячейки рисуются по своему диапазону один раз — как текст.
fn draw_visuals(
    book: &Workbook,
    sheet: &Sheet,
    rules: &RuleIndex<'_>,
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    out: &mut DisplayList,
) {
    for range in visible_merges(sheet, region) {
        let (x, y, w, h) = range_rect(layout, scale, region, range);
        draw_visual(book, rules, range.first, (x, y, w, h), scale, out);
    }
    for row in region.rows.0..=region.rows.1 {
        for (col, cell) in visible_cells(sheet, row, region.cols) {
            if sheet.merges.covering(cell.at(row)).is_some() {
                continue;
            }
            let x = region.screen_x(layout.column_x(col), scale);
            let y = region.screen_y(layout.row_y(row), scale);
            let w = layout.column_width(col) * scale;
            let h = layout.row_height(row) * scale;
            draw_visual(book, rules, cell.at(row), (x, y, w, h), scale, out);
        }
    }
}

/// Полоса данных или значок одной ячейки в её прямоугольнике.
fn draw_visual(
    book: &Workbook,
    rules: &RuleIndex<'_>,
    at: CellRef,
    (x, y, w, h): (f32, f32, f32, f32),
    scale: f32,
    out: &mut DisplayList,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let style = rules.style_at(book, at);
    let Some(visual) = style.visual() else {
        return;
    };
    match visual {
        // Цвет шкалы уже лёг фоном ячейки (`fill_color`).
        Visual::ColorScale(_) => {}
        Visual::DataBar {
            start, end, color, ..
        } => {
            // Полоса оставляет поля по краям ячейки, как в Excel, и рисуется
            // под текстом: значение ячейки читается поверх неё.
            let pad = scale;
            let inner = (w - pad * 2.0).max(0.0);
            let bar_h = h * BAR_HEIGHT_RATIO;
            let width = (end - start) * inner;
            if width > 0.0 {
                fill_rect(
                    out,
                    x + pad + start * inner,
                    y + (h - bar_h) / 2.0,
                    width,
                    bar_h,
                    color,
                );
            }
        }
        Visual::Icon {
            glyph,
            color,
            show_value,
        } => {
            let font = style.font(book.styles()).name;
            let size = (h * ICON_HEIGHT_RATIO).min(w * 0.6).max(1.0);
            let (tx, align) = if show_value {
                (x + scale, TextAlign::Left)
            } else {
                // Значение скрыто — значок стоит по центру ячейки.
                (x + w / 2.0, TextAlign::Center)
            };
            let text = out.intern(glyph);
            let font_ref = out.intern(&font);
            out.push(DrawCommand::Text {
                x: tx,
                y: y + h / 2.0,
                text,
                font: font_ref,
                size,
                color,
                align,
                baseline: TextBaseline::Middle,
                bold: false,
                italic: false,
                underline: false,
            });
        }
    }
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

/// Видимые в квадранте объединения.
fn visible_merges<'a>(sheet: &'a Sheet, region: &'a Region) -> impl Iterator<Item = Range> + 'a {
    sheet.merges.ranges().filter(move |range| {
        range.last.row >= region.rows.0
            && range.first.row <= region.rows.1
            && range.last.col >= region.cols.0
            && range.first.col <= region.cols.1
    })
}

/// Границы квадранта: у каждой видимой ячейки и по контуру объединений.
///
/// Скрытые строки и столбцы рамок не получают вовсе, а объединённая ячейка —
/// рамку по контуру диапазона: её стороны хранит якорь.
#[allow(clippy::too_many_arguments)]
fn draw_borders(
    book: &Workbook,
    sheet: &Sheet,
    rules: &RuleIndex<'_>,
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    options: &PaintOptions,
    out: &mut DisplayList,
) {
    for row in region.rows.0..=region.rows.1 {
        for (col, cell) in visible_cells(sheet, row, region.cols) {
            if sheet.merges.covering(cell.at(row)).is_some()
                || layout.column_width(col) <= 0.0
                || layout.row_height(row) <= 0.0
            {
                continue;
            }
            let border = rules.style_at(book, cell.at(row)).border(book.styles());
            draw_border(
                book,
                sheet,
                rules,
                layout,
                scale,
                region,
                (row, row),
                (col, col),
                &border,
                options,
                out,
            );
        }
    }
    for range in visible_merges(sheet, region) {
        // Рамку показывает только якорь объединения: пустой диапазон рисовать
        // нечем.
        if sheet.cells.cell(range.first).is_none() {
            continue;
        }
        let border = rules.style_at(book, range.first).border(book.styles());
        draw_border(
            book,
            sheet,
            rules,
            layout,
            scale,
            region,
            (range.first.row, range.last.row),
            (range.first.col, range.last.col),
            &border,
            options,
            out,
        );
    }
}

/// Экранная геометрия диапазона: левый верхний угол, ширина и высота.
fn range_rect(
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    range: Range,
) -> (f32, f32, f32, f32) {
    let x = region.screen_x(layout.column_x(range.first.col), scale);
    let y = region.screen_y(layout.row_y(range.first.row), scale);
    let w = (layout.column_x(range.last.col + 1) - layout.column_x(range.first.col)) * scale;
    let h = (layout.row_y(range.last.row + 1) - layout.row_y(range.first.row)) * scale;
    (x, y, w, h)
}

/// Линия рамки: рисунок, толщина в пикселях раскладки и вес в конфликте.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BorderLine {
    style: LineStyle,
    /// Толщина в пикселях раскладки — как и координаты, её умножает масштаб.
    width: f32,
    /// Кто тяжелее, тот и рисуется на общем ребре соседей.
    weight: u8,
}

/// Перевод `ST_BorderStyle` в линию кадра.
///
/// Толщины повторяют ширины Excel 2013 в пунктах — 0,75 pt у тонких, 1,75 pt у
/// средних, 2,5 pt у толстых и двойных (таблица соответствия Excel и
/// `LibreOffice`: `sc/qa/unit/data/README.cellborders`), — переведённые по 96 dpi
/// и округлённые до целых пикселей раскладки. Волосяная линия тоньше пикселя и
/// по определению остаётся полупрозрачной на экране с DPR 1.
///
/// `LineStyle` знает штрих и точки, поэтому пунктирные варианты `ST_BorderStyle`
/// сводятся к ним, а `Double` получает полную толщину — painter сам делит её на
/// две линии и зазор.
///
/// Порядок весов — приоритет линий Excel 5.0/7.0 (KB 98152): Double > Thick >
/// Medium > Thin > Dashed > Dotted > Hair. Современный Excel решает конфликт по
/// «свежести» последней применённой границы, но в файле она не сохраняется —
/// брать нечего, кроме старого правила.
fn border_line(style: BorderStyle) -> Option<BorderLine> {
    let line = |style, width, weight| {
        Some(BorderLine {
            style,
            width,
            weight,
        })
    };
    match style {
        BorderStyle::None => None,
        BorderStyle::Hair => line(LineStyle::Solid, 0.5, 1),
        BorderStyle::Dotted => line(LineStyle::Dotted, 1.0, 2),
        BorderStyle::Dashed
        | BorderStyle::DashDot
        | BorderStyle::DashDotDot
        | BorderStyle::SlantDashDot => line(LineStyle::Dashed, 1.0, 3),
        BorderStyle::Thin => line(LineStyle::Solid, 1.0, 4),
        BorderStyle::MediumDashed | BorderStyle::MediumDashDot | BorderStyle::MediumDashDotDot => {
            line(LineStyle::Dashed, 2.0, 5)
        }
        BorderStyle::Medium => line(LineStyle::Solid, 2.0, 6),
        BorderStyle::Thick => line(LineStyle::Solid, 3.0, 7),
        BorderStyle::Double => line(LineStyle::Double, 3.0, 8),
    }
}

/// Сторона-победитель на общем ребре: своя или соседа.
///
/// Сначала решает вес, при полном равенстве — положение: выигрывает ячейка,
/// которая левее (для вертикального ребра) или выше (для горизонтального).
/// Так же разрешает конфликт CSS 2.1 §17.6.2.1 (`border-collapse`); это
/// единственный симметричный вариант — у обоих соседей ребро получает одного и
/// того же победителя. `LibreOffice` на полном равенстве отдаёт ребро соседу
/// только потому, что сравнивает по очереди, и сам помечает это
/// (`ScHasPriority`, `sc/source/core/data/attrib.cxx`, `// FIXME: What is this?`).
fn resolve_side(own: BorderSide, other: BorderSide, own_left_or_top: bool) -> BorderSide {
    let weight = |side: BorderSide| border_line(side.style).map_or(0, |line| line.weight);
    let (own_w, other_w) = (weight(own), weight(other));
    if own_w > other_w {
        own
    } else if other_w > own_w {
        other
    } else if own_left_or_top {
        own
    } else {
        other
    }
}

/// Сторона прямоугольника: какую из его ячеек делить с соседом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    /// Сторона соседа, обращённая к нам.
    const fn facing(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Сторона ячейки `at`; несуществующая ячейка даёт пустую сторону.
///
/// Ячейка внутри объединения своей рамки не имеет — её показывает якорь, он же
/// рисуется по контуру диапазона.
fn side_at(
    book: &Workbook,
    sheet: &Sheet,
    rules: &RuleIndex<'_>,
    at: CellRef,
    side: Edge,
) -> BorderSide {
    let at = sheet.merges.covering(at).map_or(at, |range| range.first);
    let border = rules.style_at(book, at).border(book.styles());
    match side {
        Edge::Top => border.top,
        Edge::Bottom => border.bottom,
        Edge::Left => border.left,
        Edge::Right => border.right,
    }
}

/// Сторона соседа за ребром ячейки `(row, col)`.
///
/// Скрытые строки и столбцы пропускаются: их рамки в кадр не попадают вовсе, и
/// ребро с ними делят следующие видимые ячейки. За пределами раскладки соседа
/// нет — сторона пустая.
#[allow(clippy::too_many_arguments)]
fn neighbor_side(
    book: &Workbook,
    sheet: &Sheet,
    rules: &RuleIndex<'_>,
    layout: &SheetLayout,
    row: u32,
    col: u32,
    edge: Edge,
) -> BorderSide {
    let (mut row, mut col) = (row, col);
    match edge {
        Edge::Top => loop {
            let Some(up) = row.checked_sub(1) else {
                return BorderSide::default();
            };
            row = up;
            if layout.row_height(row) > 0.0 {
                break;
            }
        },
        Edge::Bottom => loop {
            row += 1;
            if row >= layout.rows() {
                return BorderSide::default();
            }
            if layout.row_height(row) > 0.0 {
                break;
            }
        },
        Edge::Left => loop {
            let Some(left) = col.checked_sub(1) else {
                return BorderSide::default();
            };
            col = left;
            if layout.column_width(col) > 0.0 {
                break;
            }
        },
        Edge::Right => loop {
            col += 1;
            if col >= layout.cols() {
                return BorderSide::default();
            }
            if layout.column_width(col) > 0.0 {
                break;
            }
        },
    }
    side_at(book, sheet, rules, CellRef::new(row, col), edge.facing())
}

/// Нарисовать рамку прямоугольника: четыре стороны, каждая — с соседом за
/// ребром.
#[allow(clippy::too_many_arguments)]
fn draw_border(
    book: &Workbook,
    sheet: &Sheet,
    rules: &RuleIndex<'_>,
    layout: &SheetLayout,
    scale: f32,
    region: &Region,
    rows: (u32, u32),
    cols: (u32, u32),
    border: &Border,
    options: &PaintOptions,
    out: &mut DisplayList,
) {
    let x0 = region.screen_x(layout.column_x(cols.0), scale);
    let x1 = region.screen_x(layout.column_x(cols.1 + 1), scale);
    let y0 = region.screen_y(layout.row_y(rows.0), scale);
    let y1 = region.screen_y(layout.row_y(rows.1 + 1), scale);
    // Полностью скрытый прямоугольник (например, объединение из одних скрытых
    // столбцов) не виден ни одной точкой.
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    // На общем ребре сторону выбирает `resolve_side`; при равенстве выигрывает
    // левая или верхняя ячейка — за неё и отвечает последний аргумент.
    let top = resolve_side(
        border.top,
        neighbor_side(book, sheet, rules, layout, rows.0, cols.0, Edge::Top),
        false,
    );
    let bottom = resolve_side(
        border.bottom,
        neighbor_side(book, sheet, rules, layout, rows.1, cols.0, Edge::Bottom),
        true,
    );
    let left = resolve_side(
        border.left,
        neighbor_side(book, sheet, rules, layout, rows.0, cols.0, Edge::Left),
        false,
    );
    let right = resolve_side(
        border.right,
        neighbor_side(book, sheet, rules, layout, rows.0, cols.1, Edge::Right),
        true,
    );

    let theme = book.theme();
    push_border_line(out, (x0, y0), (x1, y0), top, theme, options, scale);
    push_border_line(out, (x0, y1), (x1, y1), bottom, theme, options, scale);
    push_border_line(out, (x0, y0), (x0, y1), left, theme, options, scale);
    push_border_line(out, (x1, y0), (x1, y1), right, theme, options, scale);
}

/// Одна сторона рамки в кадр; сторона без стиля пропускается.
fn push_border_line(
    out: &mut DisplayList,
    from: (f32, f32),
    to: (f32, f32),
    side: BorderSide,
    theme: &Theme,
    options: &PaintOptions,
    scale: f32,
) {
    let Some(line) = border_line(side.style) else {
        return;
    };
    // Цвет не задан или не разрешился — автоцвет: чёрный в светлом оформлении
    // (как текст по умолчанию), светлый в тёмном. Явный цвет автор оставил
    // себе, и тёмное оформление его не перекрашивает — рамка может оказаться
    // мало заметной на тёмном фоне, как и явный цвет текста.
    let stroke =
        resolve_themed(theme, side.color, options.dark).unwrap_or_else(|| options.default_text());
    out.push(DrawCommand::Line {
        x1: from.0,
        y1: from.1,
        x2: to.0,
        y2: to.1,
        stroke,
        stroke_w: line.width * scale,
        style: line.style,
    });
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
        headers.viewport.w,
        headers.header_h,
        headers.options.effective_header_background(),
    );
    fill_rect(
        out,
        0.0,
        0.0,
        headers.header_w,
        headers.viewport.h,
        headers.options.effective_header_background(),
    );

    draw_column_headers(headers, out);
    draw_row_headers(headers, out);

    let line = headers.options.effective_header_line();
    out.push(DrawCommand::Line {
        x1: 0.0,
        y1: headers.header_h,
        x2: headers.viewport.w,
        y2: headers.header_h,
        stroke: line,
        stroke_w: 1.0,
        style: LineStyle::Solid,
    });
    out.push(DrawCommand::Line {
        x1: headers.header_w,
        y1: 0.0,
        x2: headers.header_w,
        y2: headers.viewport.h,
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
    let scroll_w = (viewport.w - header_w - frozen_w).max(0.0) / scale;
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
            let visible_w = (x + w - visible_x).min(viewport.w - visible_x);
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
                color: options.effective_header_foreground(),
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
    let scroll_h = (viewport.h - header_h - frozen_h).max(0.0) / scale;
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
                color: options.effective_header_foreground(),
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

/// Текст одной ячейки; `is_link` — на ячейке лежит гиперссылка.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    book: &Workbook,
    cell: &Cell,
    style: &EffectiveStyle<'_>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    scale: f32,
    clip: &TextClip,
    is_link: bool,
    options: &PaintOptions,
    out: &mut DisplayList,
) {
    let code = style.format_code(book.styles());
    let Some(text) = display_text_with(book, cell, code) else {
        return;
    };
    if text.is_empty() || w <= 0.0 || h <= 0.0 {
        return;
    }

    let mut font = style.font(book.styles());
    let size = font.size * PX_PER_POINT * scale;
    let mut color = resolve_themed(book.theme(), font.color, options.dark)
        .unwrap_or_else(|| options.default_text());
    let padding = TEXT_PADDING * scale;

    // Ссылка получает оформление по умолчанию — подчёркивание и цвет `hlink`
    // из темы, как у встроенного стиля Excel «Hyperlink». В файле это
    // оформление несёт шрифт ячейки: стиль «Hyperlink» Excel прикладывает при
    // вставке ссылки, а не при отрисовке. Но книга, собранная другой
    // программой, может объявить `<hyperlink>` без стиля, и тогда ссылка не
    // отличалась бы от обычного текста.
    //
    // Допущение: поведение Excel на таком файле не проверялось — правило задаёт
    // просмотрщик, чтобы ссылки были видны.
    //
    // Приоритет: ячейка со своим шрифтом (`font != 0`) уже оформлена автором —
    // её подчёркивание и цвет не трогаем; у ячейки со шрифтом книги (`font == 0`)
    // достраиваем.
    if is_link && style.base().font == 0 {
        font.underline = true;
        // Пустой слот `hlink` в теме — не повод потерять цвет: остаётся цвет
        // шрифта.
        if let Some(link) = resolve_themed(
            book.theme(),
            CellColor::Theme(HLINK_THEME_INDEX),
            options.dark,
        ) {
            color = link;
        }
    }

    // В тёмном оформлении цвет текста проверяется на читаемость у своего фона:
    // явный тёмный текст на тёмном фоне (или светлый на светлой заливке)
    // поднимается до контраста. Явные цвета при этом не инвертируются — только
    // сдвигаются по яркости.
    if options.dark {
        let background =
            fill_color(book, style, options).unwrap_or_else(|| options.effective_background());
        color = ensure_contrast(color, background);
    }

    // Числа Excel не выпускает за ячейку и не обрезает: не помещается —
    // показывает решётки. Даты и деньги — тоже числа.
    // Числа Excel не выпускает за ячейку и не обрезает: не помещается —
    // показывает решётки. Ширина — настоящие метрики шрифта по умолчанию, а
    // не оценочная таблица: `FontRegistry` подключён в путь рисования
    // (ADR-0002, ADR-0005).
    let text = if matches!(cell.value, CellValue::Number(_))
        && FONTS.with(|fonts| {
            let mut fonts = fonts.borrow_mut();
            measure_text(&mut fonts, DEFAULT_FONT_ID, size, &text)
        }) > w - padding * 2.0
    {
        HASHES.to_owned()
    } else {
        text
    };

    let (align, tx) = match horizontal(style.base().alignment.horizontal, &cell.value) {
        HorizontalAlign::Center | HorizontalAlign::CenterContinuous => {
            (TextAlign::Center, x + w / 2.0)
        }
        HorizontalAlign::Right => (TextAlign::Right, x + w - padding),
        _ => (TextAlign::Left, x + padding),
    };
    let (baseline, ty) = match style.base().alignment.vertical {
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

/// Текст ячейки так, как его показывает Excel.
///
/// Числа и даты проводит через формат: в файле лежит серийный номер, а видно
/// «31.01.2024». Строки и коды ошибок берутся как есть, логические — словами,
/// как их пишет Excel.
#[must_use]
pub fn display_text(book: &Workbook, cell: &Cell, num_fmt: u32) -> Option<String> {
    display_text_with(book, cell, book.styles().format_code(num_fmt))
}

/// Текст ячейки по уже разрешённому коду формата.
///
/// Нужен условному форматированию: правило может задать свой формат числа,
/// и он перекрывает базовый (см. [`EffectiveStyle::format_code`]). `None` —
/// формат не задан, число показывается в общем формате.
fn display_text_with(book: &Workbook, cell: &Cell, code: Option<&str>) -> Option<String> {
    match &cell.value {
        CellValue::Empty => cell.formula.as_ref().map(|_| String::new()),
        CellValue::Number(value) => {
            let code = code.unwrap_or(numfmt::GENERAL);
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
///
/// Заливку берёт из эффективного стиля: сработавшее правило могло задать свою.
fn fill_color(
    book: &Workbook,
    style: &EffectiveStyle<'_>,
    options: &PaintOptions,
) -> Option<Color> {
    // Цветовая шкала задаёт фон сама и перекрывает заливку формата.
    if let Some(background) = style.background() {
        return Some(background);
    }
    let fill = style.fill(book.styles())?;
    match fill.pattern {
        crate::model::FillPattern::Solid => {
            resolve_themed(book.theme(), fill.foreground, options.dark)
        }
        crate::model::FillPattern::None => None,
        // Узоры Excel рисует растром; в DisplayList растра нет, поэтому узор
        // показывается своим цветом. TODO (Фаза 5): растр узора.
        _ => resolve_themed(book.theme(), fill.foreground, options.dark)
            .or_else(|| resolve_themed(book.theme(), fill.background, options.dark)),
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

/// Цвет из модели с учётом тёмного оформления.
///
/// В тёмном оформлении индексы темы идут через [`dark_theme_index`], остальное
/// разрешается как есть ([`resolve_color`]): явные RGB-цвета не инвертируются.
fn resolve_themed(theme: &Theme, color: CellColor, dark: bool) -> Option<Color> {
    if dark {
        if let CellColor::Theme(index) = color {
            return theme_color(theme, dark_theme_index(index));
        }
    }
    resolve_color(theme, color)
}

/// Индекс цвета темы в тёмном оформлении: светлые и тёмные слоты меняются
/// местами (`lt1`↔`dk1`, `lt2`↔`dk2`).
///
/// Акценты (4–9) и цвета ссылок (10, 11) остаются: парных светлых и тёмных
/// вариантов у них нет, а сами они — средние тона, различимые на обоих фонах.
/// Ссылку при необходимости дотягивает [`ensure_contrast`]. Допущение: Excel
/// тёмного оформления листа не документирует; правило задаёт просмотрщик.
fn dark_theme_index(index: u32) -> u32 {
    match index {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        other => other,
    }
}

/// Минимальный контраст обычного текста с фоном по WCAG 2.1 (уровень AA).
const MIN_TEXT_CONTRAST: f32 = 4.5;

/// Относительная яркость sRGB по WCAG 2.1.
fn luminance(color: Color) -> f32 {
    fn linear(channel: u32) -> f32 {
        let value = f32::from(u8::try_from(channel).unwrap_or_default()) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    // Цвет в `DisplayList` — `RRGGBBAA`.
    0.2126 * linear((color.0 >> 24) & 0xFF)
        + 0.7152 * linear((color.0 >> 16) & 0xFF)
        + 0.0722 * linear((color.0 >> 8) & 0xFF)
}

/// Контраст двух цветов по WCAG 2.1: от 1 (неразличимы) до 21.
fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (one, two) = (luminance(a), luminance(b));
    let (hi, lo) = if one >= two { (one, two) } else { (two, one) };
    (hi + 0.05) / (lo + 0.05)
}

/// Подмешать `to` к `from` на `step/10`; целочисленно, чтобы результат не
/// зависел от округления `f32`.
fn mix(from: Color, to: Color, step: u32) -> Color {
    let channel = |shift: u32| -> u32 {
        let from = i64::from((from.0 >> shift) & 0xFF);
        let to = i64::from((to.0 >> shift) & 0xFF);
        let value = from + (to - from) * i64::from(step) / 10;
        u32::try_from(value.clamp(0, 255)).unwrap_or_default()
    };
    Color((channel(24) << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0))
}

/// Поднять контраст текста с фоном до [`MIN_TEXT_CONTRAST`].
///
/// Цвет подмешивается к белому или чёрному — смотря какой полюс дальше от
/// фона, — поэтому оттенок сохраняется: тёмно-синий текст на тёмном фоне
/// становится светло-синим, а не белым. Неразличимый цвет на среднем фоне
/// (контраста не даёт ни один полюс) остаётся ближайшим к читаемому.
fn ensure_contrast(foreground: Color, background: Color) -> Color {
    if contrast_ratio(foreground, background) >= MIN_TEXT_CONTRAST {
        return foreground;
    }
    let target =
        if contrast_ratio(Color::WHITE, background) >= contrast_ratio(Color::BLACK, background) {
            Color::WHITE
        } else {
            Color::BLACK
        };
    (1..=10)
        .map(|step| mix(foreground, target, step))
        .find(|candidate| contrast_ratio(*candidate, background) >= MIN_TEXT_CONTRAST)
        .unwrap_or(target)
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
    use crate::dims::{ColWidth, RowHeight};
    use crate::drawing::{ImageExtent, ImageMarker, SheetImage};
    use crate::model::{Border, CellFormat, Fill, FillPattern, Font, StyleTable, WorksheetMeta};
    use crate::model::{SheetContent, Theme, WorksheetBuilder};
    use crate::sheet_meta::{Hyperlink, HyperlinkTarget};
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
                      <a:hlink><a:srgbClr val="0000FF"/></a:hlink>
                      <a:folHlink><a:srgbClr val="800080"/></a:folHlink>
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

    /// Кадр только с содержимым: сетка и заголовки не мешают искать текст.
    fn content_only() -> PaintOptions {
        PaintOptions {
            show_grid: false,
            show_headers: false,
            ..PaintOptions::default()
        }
    }

    /// Текстовые команды кадра: содержимое, подчёркивание и цвет.
    fn text_commands(dl: &DisplayList) -> Vec<(String, bool, Color)> {
        (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Text {
                    text,
                    underline,
                    color,
                    ..
                } => Some((dl.string(*text).to_owned(), *underline, *color)),
                _ => None,
            })
            .collect()
    }

    /// Лист из ячеек `(строка, столбец, формат, текст)` и гиперссылок `ref`.
    fn content_with(cells: &[(u32, u32, u32, &str)], links: &[&str]) -> SheetContent {
        let mut builder = WorksheetBuilder::new(PART);
        for (row, col, style, text) in cells {
            builder
                .push(
                    *row,
                    Cell::new(*col, *style, CellValue::InlineString((*text).into())),
                )
                .unwrap();
        }
        SheetContent {
            cells: builder.finish(),
            hyperlinks: links
                .iter()
                .map(|range| Hyperlink {
                    range: Range::parse_ref(range).unwrap(),
                    target: HyperlinkTarget::External("https://example.com/".into()),
                    display: None,
                    tooltip: None,
                })
                .collect(),
            ..SheetContent::default()
        }
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
            w: 128.0,
            h: 40.0,
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
                y: 20.0,
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
            y: 20.0,
            w: 128.0,
            h: 60.0,
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
    fn hyperlink_cell_without_its_own_font_gets_underline_and_theme_color() {
        let content = content_with(&[(0, 0, 0, "ссылка"), (0, 1, 0, "текст")], &["A1"]);
        let book = book_with_theme(content, StyleTable::default(), theme());

        let dl = painted(&book, Viewport::default(), &content_only());
        let texts = text_commands(&dl);

        let link = texts
            .iter()
            .find(|(text, ..)| text == "ссылка")
            .expect("ссылка в кадре");
        assert!(link.1, "ссылка подчёркнута");
        // hlink темы — синий 0000FF; в DisplayList каналы уже RRGGBBAA.
        assert_eq!(link.2, Color(0x0000_FFFF));

        let plain = texts
            .iter()
            .find(|(text, ..)| text == "текст")
            .expect("сосед в кадре");
        assert!(!plain.1, "ячейка без ссылки не подчёркнута");
        assert_eq!(plain.2, Color::BLACK);
    }

    #[test]
    fn hyperlink_range_styles_every_covered_cell() {
        // `ref` гиперссылки — диапазон: оформление получает каждая накрытая
        // ячейка, а не только левая верхняя.
        let content = content_with(
            &[(0, 0, 0, "раз"), (0, 1, 0, "мимо"), (1, 0, 0, "два")],
            &["A1:A2"],
        );
        let book = book_with_theme(content, StyleTable::default(), theme());
        let texts = text_commands(&painted(&book, Viewport::default(), &content_only()));

        for covered in ["раз", "два"] {
            let (_, underline, color) = texts
                .iter()
                .find(|(text, ..)| text == covered)
                .unwrap_or_else(|| panic!("{covered}: текста нет в кадре"));
            assert!(
                *underline && *color == Color(0x0000_FFFF),
                "{covered}: ячейка накрыта ссылкой"
            );
        }
        let (_, underline, _) = texts
            .iter()
            .find(|(text, ..)| text == "мимо")
            .expect("текст в кадре");
        assert!(!*underline, "соседняя ячейка не тронута");
    }

    #[test]
    fn empty_theme_hlink_slot_does_not_break_the_link() {
        // Тема без слота `hlink`: цвет брать неоткуда — остаётся цвет шрифта,
        // но подчёркивание никуда не девается.
        let content = content_with(&[(0, 0, 0, "ссылка")], &["A1"]);
        let book = book_with_theme(content, StyleTable::default(), Theme::default());
        let texts = text_commands(&painted(&book, Viewport::default(), &content_only()));

        let (_, underline, color) = texts.first().expect("ссылка в кадре");
        assert!(*underline, "ссылка подчёркнута и без цвета темы");
        assert_eq!(*color, Color::BLACK);
    }

    #[test]
    fn link_with_its_own_font_keeps_the_author_style() {
        // Автор дал ячейке свой шрифт — красный и без подчёркивания; ссылка не
        // повод переписать его.
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
                    color: CellColor::Rgb(0xFF_FF_00_00),
                    ..Font::default()
                },
            ],
            vec![Fill::default()],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let content = content_with(&[(0, 0, 1, "ссылка")], &["A1"]);
        let book = book_with_theme(content, styles, theme());
        let texts = text_commands(&painted(&book, Viewport::default(), &content_only()));

        let (_, underline, color) = texts.first().expect("ссылка в кадре");
        assert!(!underline, "подчёркивание автора не включают");
        assert_eq!(*color, Color(0xFF00_00FF), "цвет автора не перекрашивают");
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

    /// Тёмное оформление того же кадра.
    fn dark_options(options: &PaintOptions) -> PaintOptions {
        PaintOptions {
            dark: true,
            ..*options
        }
    }

    /// Заливка первой прямоугольной команды — фон окна.
    fn window_background(dl: &DisplayList) -> Color {
        (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .find_map(|cmd| match cmd {
                DrawCommand::Rect { fill, .. } => Some(*fill),
                _ => None,
            })
            .expect("фон окна в кадре")
    }

    /// Цвет первой текстовой команды.
    fn first_text_color(dl: &DisplayList) -> Color {
        text_commands(dl)
            .first()
            .map(|(_, _, color)| *color)
            .expect("текст в кадре")
    }

    /// Кадр содержит прямоугольник такой заливки.
    fn has_rect(dl: &DisplayList, fill: Color) -> bool {
        (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .any(|cmd| matches!(cmd, DrawCommand::Rect { fill: f, .. } if *f == fill))
    }

    /// Книга из одной ячейки в стиле 1: шрифт и заливка заданы вызывающим.
    fn styled_book(font: Font, fill: Fill) -> Workbook {
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    font: 1,
                    fill: 1,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default(), font],
            vec![Fill::default(), fill],
            vec![Border::default()],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(1, 1, CellValue::InlineString("текст".into())))
            .unwrap();
        book_with_theme(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
            theme(),
        )
    }

    #[test]
    fn dark_option_repaints_chrome_and_theme_text() {
        // `theme="1"` — dk1: в светлом оформлении чёрный текст.
        let book = styled_book(
            Font {
                color: CellColor::Theme(1),
                ..Font::default()
            },
            Fill::default(),
        );
        let viewport = Viewport::default();
        let light = painted(&book, viewport, &content_only());
        let dark = painted(&book, viewport, &dark_options(&content_only()));

        assert_eq!(window_background(&light), Color::WHITE);
        assert_eq!(window_background(&dark), dl_color::DARK_BACKGROUND);
        assert_eq!(first_text_color(&light), Color::BLACK, "dk1 — чёрный");
        assert_eq!(
            first_text_color(&dark),
            Color::WHITE,
            "в тёмном оформлении dk1 и lt1 меняются местами"
        );
        assert!(
            contrast_ratio(first_text_color(&dark), window_background(&dark)) >= MIN_TEXT_CONTRAST,
            "текст темы обязан читаться на тёмном фоне"
        );
    }

    #[test]
    fn dark_off_paints_the_light_frame() {
        let book = styled_book(Font::default(), Fill::default());
        let viewport = Viewport::default();
        let default = painted(&book, viewport, &PaintOptions::default());
        let off = painted(
            &book,
            viewport,
            &PaintOptions {
                dark: false,
                ..PaintOptions::default()
            },
        );

        assert_eq!(
            default.to_bytes(),
            off.to_bytes(),
            "выключенная опция не меняет ни одной команды"
        );
        assert_eq!(window_background(&default), Color::WHITE);
    }

    #[test]
    fn dark_option_repaints_grid_and_headers() {
        let book = book_with(numbers(), StyleTable::default());
        let viewport = Viewport::default();
        let light = painted(&book, viewport, &PaintOptions::default());
        let dark = painted(&book, viewport, &dark_options(&PaintOptions::default()));
        let strokes =
            |dl: &DisplayList| -> Vec<Color> { lines(dl).into_iter().map(|line| line.4).collect() };
        let texts = |dl: &DisplayList| -> Vec<Color> {
            text_commands(dl)
                .into_iter()
                .map(|(_, _, color)| color)
                .collect()
        };

        assert!(strokes(&light).contains(&dl_color::GRID));
        assert!(strokes(&light).contains(&dl_color::HEADER_LINE));
        assert!(has_rect(&light, dl_color::HEADER_BG));
        assert!(texts(&light).contains(&dl_color::HEADER_FG));

        assert!(strokes(&dark).contains(&dl_color::DARK_GRID));
        assert!(strokes(&dark).contains(&dl_color::DARK_HEADER_LINE));
        assert!(has_rect(&dark, dl_color::DARK_HEADER_BG));
        assert!(texts(&dark).contains(&dl_color::DARK_HEADER_FG));
        assert!(
            contrast_ratio(dl_color::DARK_HEADER_FG, dl_color::DARK_HEADER_BG) >= MIN_TEXT_CONTRAST
        );
    }

    #[test]
    fn explicit_dark_text_on_light_fill_stays_dark_in_dark_theme() {
        let yellow = Color(0xFFFF_00FF);
        let book = styled_book(
            Font {
                color: CellColor::Rgb(0xFF_00_00_00),
                ..Font::default()
            },
            Fill {
                pattern: FillPattern::Solid,
                foreground: CellColor::Rgb(0xFF_FF_FF_00),
                background: CellColor::None,
            },
        );

        let dark = painted(&book, Viewport::default(), &dark_options(&content_only()));
        assert!(has_rect(&dark, yellow), "заливка автора остаётся светлой");
        assert_eq!(
            first_text_color(&dark),
            Color::BLACK,
            "цвет автора на своей заливке не перекрашивается"
        );
        assert!(contrast_ratio(Color::BLACK, yellow) >= MIN_TEXT_CONTRAST);
    }

    #[test]
    fn explicit_dark_text_without_fill_is_lifted_in_dark_theme() {
        let book = styled_book(
            Font {
                color: CellColor::Rgb(0xFF_00_00_00),
                ..Font::default()
            },
            Fill::default(),
        );
        let viewport = Viewport::default();
        let light = painted(&book, viewport, &content_only());
        let dark = painted(&book, viewport, &dark_options(&content_only()));

        assert_eq!(
            first_text_color(&light),
            Color::BLACK,
            "светлое оформление цвет автора не трогает"
        );
        let lifted = first_text_color(&dark);
        assert_ne!(lifted, Color::BLACK, "на тёмном фоне чёрный не виден");
        assert!(
            contrast_ratio(lifted, dl_color::DARK_BACKGROUND) >= MIN_TEXT_CONTRAST,
            "поднятый цвет обязан читаться"
        );
    }

    #[test]
    fn dark_theme_swaps_light_and_dark_fill_slots() {
        // Заливка `theme="2"` — lt2, текст `theme="1"` — dk1.
        let book = styled_book(
            Font {
                color: CellColor::Theme(1),
                ..Font::default()
            },
            Fill {
                pattern: FillPattern::Solid,
                foreground: CellColor::Theme(2),
                background: CellColor::None,
            },
        );
        let viewport = Viewport::default();
        let light = painted(&book, viewport, &content_only());
        let dark = painted(&book, viewport, &dark_options(&content_only()));

        let lt2 = Color(0xEEEC_E1FF);
        let dk2 = Color(0x1F49_7DFF);
        assert!(has_rect(&light, lt2), "в светлом lt2 остаётся собой");
        assert!(has_rect(&dark, dk2), "в тёмном lt2 меняется на dk2");
        assert_eq!(first_text_color(&dark), Color::WHITE);
        assert!(contrast_ratio(Color::WHITE, dk2) >= MIN_TEXT_CONTRAST);
    }

    #[test]
    fn automatic_border_follows_dark_theme_but_explicit_color_does_not() {
        // Рамка 1 — без цвета (автоцвет), рамка 2 — явный красный.
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    border: 1,
                    ..CellFormat::default()
                },
                CellFormat {
                    border: 2,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default()],
            vec![Fill::default()],
            vec![
                Border::default(),
                Border {
                    top: side(BorderStyle::Thin, CellColor::None),
                    ..Border::default()
                },
                Border {
                    top: side(BorderStyle::Thin, CellColor::Rgb(0xFFFF_0000)),
                    ..Border::default()
                },
            ],
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(1.0)))
            .unwrap();
        builder
            .push(0, Cell::new(1, 2, CellValue::Number(2.0)))
            .unwrap();
        let book = book_with_theme(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
            theme(),
        );

        let strokes =
            |dl: &DisplayList| -> Vec<Color> { lines(dl).into_iter().map(|line| line.4).collect() };
        let light = painted(&book, Viewport::default(), &borders_only());
        let dark = painted(&book, Viewport::default(), &dark_options(&borders_only()));

        assert!(
            strokes(&light).contains(&Color::BLACK),
            "автоцвет в светлом — чёрный, как текст по умолчанию"
        );
        assert!(
            strokes(&dark).contains(&dl_color::DARK_TEXT),
            "автоцвет в тёмном — светлый"
        );
        assert!(
            strokes(&dark).contains(&RED),
            "явный цвет рамки тёмное оформление не перекрашивает"
        );
    }

    #[test]
    fn dark_frame_text_stays_readable() {
        // Гиперссылка без своего шрифта: `hlink` темы (0000FF) на тёмном фоне
        // неразличим сам по себе — цвет обязан поднять `ensure_contrast`.
        let book = book_with_theme(
            content_with(&[(0, 0, 0, "ссылка")], &["A1"]),
            StyleTable::default(),
            theme(),
        );
        let viewport = Viewport::default();
        let dark = painted(&book, viewport, &dark_options(&content_only()));
        let (_, underline, link) = text_commands(&dark)
            .first()
            .cloned()
            .expect("ссылка в кадре");
        assert!(underline);
        assert!(
            contrast_ratio(link, dl_color::DARK_BACKGROUND) >= MIN_TEXT_CONTRAST,
            "ссылка не читается: {link:?}"
        );

        // Весь текст листа с числами — без явных цветов — тоже читается.
        let numbers = book_with(numbers(), StyleTable::default());
        let dark = painted(&numbers, viewport, &dark_options(&content_only()));
        let background = window_background(&dark);
        for (text, _, color) in text_commands(&dark) {
            assert!(
                contrast_ratio(color, background) >= MIN_TEXT_CONTRAST,
                "«{text}» не читается: {color:?} на {background:?}"
            );
        }
    }

    /// Цвет в раскладке `DisplayList` (`RRGGBBAA`).
    const RED: Color = Color(0xFF00_00FF);
    const BLUE: Color = Color(0x0000_FFFF);

    /// Сторона рамки.
    fn side(style: BorderStyle, color: CellColor) -> BorderSide {
        BorderSide { style, color }
    }

    /// Линии кадра: `(x1, y1, x2, y2, цвет, толщина, рисунок)`.
    fn lines(dl: &DisplayList) -> Vec<(f32, f32, f32, f32, Color, f32, LineStyle)> {
        (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke,
                    stroke_w,
                    style,
                } => Some((*x1, *y1, *x2, *y2, *stroke, *stroke_w, *style)),
                _ => None,
            })
            .collect()
    }

    /// Без сетки и заголовков: в кадре остаются только рамки.
    fn borders_only() -> PaintOptions {
        PaintOptions {
            show_grid: false,
            show_headers: false,
            ..PaintOptions::default()
        }
    }

    /// Таблица стилей с рамками `borders`: формат 1 — рамка 1, формат 2 — рамка 2.
    fn border_styles(borders: Vec<Border>) -> StyleTable {
        StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    border: 1,
                    ..CellFormat::default()
                },
                CellFormat {
                    border: 2,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default()],
            vec![Fill::default()],
            borders,
            BTreeMap::new(),
        )
    }

    #[test]
    fn border_styles_map_to_lines() {
        let cases = [
            (BorderStyle::None, None),
            (BorderStyle::Hair, Some((LineStyle::Solid, 0.5))),
            (BorderStyle::Dotted, Some((LineStyle::Dotted, 1.0))),
            (BorderStyle::Dashed, Some((LineStyle::Dashed, 1.0))),
            (BorderStyle::DashDot, Some((LineStyle::Dashed, 1.0))),
            (BorderStyle::DashDotDot, Some((LineStyle::Dashed, 1.0))),
            (BorderStyle::SlantDashDot, Some((LineStyle::Dashed, 1.0))),
            (BorderStyle::Thin, Some((LineStyle::Solid, 1.0))),
            (BorderStyle::MediumDashed, Some((LineStyle::Dashed, 2.0))),
            (BorderStyle::MediumDashDot, Some((LineStyle::Dashed, 2.0))),
            (
                BorderStyle::MediumDashDotDot,
                Some((LineStyle::Dashed, 2.0)),
            ),
            (BorderStyle::Medium, Some((LineStyle::Solid, 2.0))),
            (BorderStyle::Thick, Some((LineStyle::Solid, 3.0))),
            (BorderStyle::Double, Some((LineStyle::Double, 3.0))),
        ];
        for (style, want) in cases {
            let got = border_line(style).map(|line| (line.style, line.width));
            assert_eq!(got, want, "{style:?}");
        }
    }

    #[test]
    fn border_weights_follow_excel_precedence() {
        // Порядок Excel 5.0/7.0: чем позже в списке, тем тяжелее сторона.
        let order = [
            BorderStyle::None,
            BorderStyle::Hair,
            BorderStyle::Dotted,
            BorderStyle::Dashed,
            BorderStyle::DashDot,
            BorderStyle::DashDotDot,
            BorderStyle::SlantDashDot,
            BorderStyle::Thin,
            BorderStyle::MediumDashed,
            BorderStyle::MediumDashDot,
            BorderStyle::MediumDashDotDot,
            BorderStyle::Medium,
            BorderStyle::Thick,
            BorderStyle::Double,
        ];
        let weights: Vec<u8> = order
            .iter()
            .map(|style| border_line(*style).map_or(0, |line| line.weight))
            .collect();
        assert!(
            weights.windows(2).all(|pair| pair[0] <= pair[1]),
            "веса не по возрастанию: {weights:?}"
        );
        // Вес не совпадает с толщиной: штрих тонкой линии проигрывает сплошной
        // той же толщины, а средний пунктир — тонкой сплошной не проигрывает.
        assert!(weights[3] < weights[7], "штрих легче сплошной");
        assert!(weights[7] < weights[8], "средний пунктир тяжелее тонкой");
        assert!(weights[12] < weights[13], "двойная тяжелее толстой");
    }

    #[test]
    fn heavier_side_wins_the_shared_edge() {
        let thin_red = Border {
            right: side(BorderStyle::Thin, CellColor::Rgb(0xFFFF_0000)),
            ..Border::default()
        };
        let medium_blue = Border {
            left: side(BorderStyle::Medium, CellColor::Rgb(0xFF00_00FF)),
            ..Border::default()
        };

        let borders = vec![Border::default(), thin_red, medium_blue];
        let book = two_cells(border_styles(borders), PART);
        let dl = painted(&book, Viewport::default(), &borders_only());
        assert_edge(&dl, (BLUE, 2.0));

        // Побеждает не «правая» сторона, а тяжёлая: поменяем местами стили —
        // победитель тот же.
        let borders = vec![
            Border::default(),
            Border {
                right: side(BorderStyle::Medium, CellColor::Rgb(0xFF00_00FF)),
                ..Border::default()
            },
            Border {
                left: side(BorderStyle::Thin, CellColor::Rgb(0xFFFF_0000)),
                ..Border::default()
            },
        ];
        let book = two_cells(border_styles(borders), PART);
        let dl = painted(&book, Viewport::default(), &borders_only());
        assert_edge(&dl, (BLUE, 2.0));
    }

    #[test]
    fn equal_weight_borders_are_won_by_the_left_cell() {
        // Обе стороны тонкие, но разных цветов: по CSS 2.1 ребро берёт левая
        // ячейка. Оба соседа обязаны нарисовать одного победителя — иначе
        // цвет ребра зависел бы от порядка обхода.
        let borders = vec![
            Border::default(),
            Border {
                right: side(BorderStyle::Thin, CellColor::Rgb(0xFFFF_0000)),
                ..Border::default()
            },
            Border {
                left: side(BorderStyle::Thin, CellColor::Rgb(0xFF00_00FF)),
                ..Border::default()
            },
        ];
        let book = two_cells(border_styles(borders), PART);
        let dl = painted(&book, Viewport::default(), &borders_only());
        assert_edge(&dl, (RED, 1.0));
    }

    /// Две ячейки одной строки: A1 — формат 1, B1 — формат 2.
    fn two_cells(styles: StyleTable, part: &str) -> Workbook {
        let mut builder = WorksheetBuilder::new(part);
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(1.0)))
            .unwrap();
        builder
            .push(0, Cell::new(1, 2, CellValue::Number(2.0)))
            .unwrap();
        book_with(
            SheetContent {
                cells: builder.finish(),
                ..SheetContent::default()
            },
            styles,
        )
    }

    /// Все линии на ребре между A1 и B1: цвет и толщина.
    fn assert_edge(dl: &DisplayList, want: (Color, f32)) {
        let edge: Vec<(Color, f32)> = lines(dl)
            .into_iter()
            .filter(|line| line.0 == 64.0 && line.2 == 64.0)
            .map(|line| (line.4, line.5))
            .collect();
        assert!(!edge.is_empty(), "ребро между A1 и B1 не нарисовано");
        assert!(
            edge.iter().all(|got| *got == want),
            "на ребре {edge:?}, ожидалось {want:?}"
        );
    }

    #[test]
    fn hidden_tracks_do_not_paint_borders() {
        // A1 — тонкая красная справа; B1 лежит в скрытом столбце и закрыт
        // толстой синей слева; строка 1 тоже скрыта и держит толстую рамку.
        let borders = vec![
            Border::default(),
            Border {
                top: side(BorderStyle::Thin, CellColor::Rgb(0xFFFF_0000)),
                right: side(BorderStyle::Thin, CellColor::Rgb(0xFFFF_0000)),
                ..Border::default()
            },
            Border {
                top: side(BorderStyle::Thick, CellColor::Rgb(0xFF00_00FF)),
                bottom: side(BorderStyle::Thick, CellColor::Rgb(0xFF00_00FF)),
                left: side(BorderStyle::Thick, CellColor::Rgb(0xFF00_00FF)),
                right: side(BorderStyle::Thick, CellColor::Rgb(0xFF00_00FF)),
                ..Border::default()
            },
        ];
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(1.0)))
            .unwrap();
        builder
            .push(0, Cell::new(1, 2, CellValue::Number(2.0)))
            .unwrap();
        builder
            .push(1, Cell::new(0, 2, CellValue::Number(3.0)))
            .unwrap();

        let mut content = SheetContent {
            cells: builder.finish(),
            ..SheetContent::default()
        };
        content.dims.cols.push(ColWidth {
            first: 1,
            last: 1,
            width: 8.43,
            custom: true,
            hidden: true,
            best_fit: false,
        });
        content.dims.rows.push(RowHeight {
            row: 1,
            height: 15.0,
            custom: true,
            hidden: true,
            outline_level: 0,
        });
        content.dims.rows.sort();

        let book = book_with(content, border_styles(borders));
        let dl = painted(&book, Viewport::default(), &borders_only());

        // Скрытые ячейки не рисуют ничего и не участвуют в конфликте: ребро
        // остаётся за тонкой красной A1, а не за толстой синей B1.
        let lines = lines(&dl);
        assert_eq!(
            lines.len(),
            2,
            "скрытые строки и столбцы дали линии: {lines:?}"
        );
        assert!(lines.iter().all(|line| line.4 == RED && line.5 == 1.0));
    }

    #[test]
    fn merged_cell_border_follows_the_range_outline() {
        let thick = side(BorderStyle::Thick, CellColor::Rgb(0xFFFF_0000));
        let borders = vec![
            Border::default(),
            Border {
                top: thick,
                bottom: thick,
                left: thick,
                right: thick,
                ..Border::default()
            },
        ];
        let styles = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    border: 1,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default()],
            vec![Fill::default()],
            borders,
            BTreeMap::new(),
        );
        let mut builder = WorksheetBuilder::new(PART);
        // Обе ячейки объединения несут одну и ту же рамку: рисуется контур
        // диапазона, а не рамка каждой клетки.
        builder
            .push(0, Cell::new(0, 1, CellValue::Number(1.0)))
            .unwrap();
        builder
            .push(0, Cell::new(1, 1, CellValue::Number(2.0)))
            .unwrap();
        let mut content = SheetContent {
            cells: builder.finish(),
            ..SheetContent::default()
        };
        content.merges.push(crate::Range::parse("A1:B2").unwrap());
        let book = book_with(content, styles);

        let dl = painted(&book, Viewport::default(), &borders_only());
        let lines = lines(&dl);
        assert_eq!(lines.len(), 4, "контур объединения: {lines:?}");
        assert!(lines.iter().all(|line| line.4 == RED && line.5 == 3.0));
        // Внутренние рёбра диапазона (x = 64 и y = 20) не нарисованы.
        assert!(lines
            .iter()
            .all(|line| line.0 != 64.0 && line.2 != 64.0 && line.1 != 20.0 && line.3 != 20.0));
    }

    /// Книга из общего каталога фикстур (`test-fixtures/xlsx`).
    fn open_fixture(name: &str) -> Workbook {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/xlsx")
            .join(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("{} не читается: {e}", path.display()));
        crate::open(bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn border_styles_fixture_paints_every_style() {
        let book = open_fixture("styles-border-styles.xlsx");
        let dl = painted(&book, Viewport::default(), &borders_only());
        let lines = lines(&dl);
        assert!(
            lines.iter().all(|line| line.1 == line.3),
            "у клеток нет боковых рамок, а в кадре вертикаль: {lines:?}"
        );

        // exceljs кладёт стили по строкам сверху вниз: thin, medium, thick,
        // dashed, dotted, double, hair. Верхняя линия каждой клетки чёрная,
        // нижняя синяя, а общее ребро соседей берёт более тяжёлая сторона.
        // Отсюда линии кадра: (y, сколько раз, цвет, рисунок, толщина).
        let expected: [(f32, usize, Color, LineStyle, f32); 8] = [
            (0.0, 1, Color::BLACK, LineStyle::Solid, 1.0), // thin, верх A1
            (20.0, 2, Color::BLACK, LineStyle::Solid, 2.0), // medium побеждает thin
            (40.0, 2, Color::BLACK, LineStyle::Solid, 3.0), // thick побеждает medium
            (60.0, 2, BLUE, LineStyle::Solid, 3.0),        // thick побеждает dashed
            (80.0, 2, BLUE, LineStyle::Dashed, 1.0),       // dashed побеждает dotted
            (100.0, 2, Color::BLACK, LineStyle::Double, 3.0), // double побеждает dotted
            (120.0, 2, BLUE, LineStyle::Double, 3.0),      // double побеждает hair
            (140.0, 1, BLUE, LineStyle::Solid, 0.5),       // hair, низ A7
        ];
        let mut total = 0;
        for (y, count, color, style, width) in expected {
            let at_y: Vec<_> = lines.iter().filter(|line| line.1 == y).collect();
            assert_eq!(at_y.len(), count, "линий на y = {y}: {lines:?}");
            assert!(
                at_y.iter()
                    .all(|line| line.4 == color && line.6 == style && line.5 == width),
                "на y = {y} ожидались {color:?} {style:?} {width}, а не {at_y:?}"
            );
            total += count;
        }
        assert_eq!(lines.len(), total, "лишние линии: {lines:?}");
    }

    #[test]
    fn border_box_fixture_paints_the_cell_grid() {
        let book = open_fixture("styles-border-box.xlsx");
        let dl = painted(&book, Viewport::default(), &borders_only());
        let lines = lines(&dl);
        assert!(!lines.is_empty());

        // A1:E5 — тонкая чёрная рамка у каждой из 25 ячеек, поэтому рёбра
        // ложатся сеткой: сторона клетки — отрезок в одну клетку (64 px в
        // ширину, 20 px в высоту).
        let columns = [
            (0.0, 64.0),
            (64.0, 128.0),
            (128.0, 192.0),
            (192.0, 256.0),
            (256.0, 320.0),
        ];
        let rows = [
            (0.0, 20.0),
            (20.0, 40.0),
            (40.0, 60.0),
            (60.0, 80.0),
            (80.0, 100.0),
        ];
        let (mut horizontal_ys, mut vertical_xs) = (Vec::new(), Vec::new());
        let (mut horizontal, mut vertical) = (0usize, 0usize);
        for line in &lines {
            assert_eq!(
                (line.4, line.5, line.6),
                (Color::BLACK, 1.0, LineStyle::Solid),
                "ребро не тонкое чёрное: {line:?}"
            );
            if line.1 == line.3 {
                assert!(
                    columns.contains(&(line.0, line.2)),
                    "горизонталь не в клетку: {line:?}"
                );
                horizontal_ys.push(line.1);
                horizontal += 1;
            } else {
                assert_eq!(line.0, line.2, "линия не по осям: {line:?}");
                assert!(
                    rows.contains(&(line.1, line.3)),
                    "вертикаль не в клетку: {line:?}"
                );
                vertical_xs.push(line.0);
                vertical += 1;
            }
        }
        // Каждая из 25 клеток рисует свои четыре стороны — по 50 линий на
        // направление; внутренние рёбра попадают в кадр дважды.
        assert_eq!((horizontal, vertical), (50, 50));
        horizontal_ys.sort_by(f32::total_cmp);
        horizontal_ys.dedup();
        vertical_xs.sort_by(f32::total_cmp);
        vertical_xs.dedup();
        assert_eq!(horizontal_ys, [0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        assert_eq!(vertical_xs, [0.0, 64.0, 128.0, 192.0, 256.0, 320.0]);
    }

    /// Команды картинок кадра: `(x, y, w, h, bitmap_id)`.
    fn image_commands(dl: &DisplayList) -> Vec<(f32, f32, f32, f32, u32)> {
        (0..dl.len())
            .filter_map(|i| dl.cmd(i))
            .filter_map(|cmd| match cmd {
                DrawCommand::Image {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                } => Some((*x, *y, *w, *h, *bitmap_id)),
                _ => None,
            })
            .collect()
    }

    /// Близость координат: EMU переводится в пиксели с дробным остатком.
    fn near(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() < 0.01
    }

    /// Лист с ячейкой A1 и заданными картинками.
    fn images_content(images: Vec<SheetImage>) -> SheetContent {
        let mut builder = WorksheetBuilder::new(PART);
        builder
            .push(0, Cell::new(0, 0, CellValue::Number(1.0)))
            .unwrap();
        SheetContent {
            cells: builder.finish(),
            images,
            ..SheetContent::default()
        }
    }

    /// Картинка без media: в кадре важны только id и якорь.
    fn sheet_image(anchor: ImageAnchor, image_id: Option<u32>) -> SheetImage {
        SheetImage {
            name: None,
            media: None,
            image_id,
            edit_as: None,
            anchor,
        }
    }

    /// One-cell якорь с нулевыми смещениями.
    fn one_cell(col: u32, row: u32, cx: f32, cy: f32) -> ImageAnchor {
        ImageAnchor::OneCell {
            from: ImageMarker {
                col,
                row,
                col_off: 0.0,
                row_off: 0.0,
            },
            ext: ImageExtent { cx, cy },
        }
    }

    #[test]
    fn png_fixture_paints_both_anchor_kinds() {
        let book = open_fixture("images-png.xlsx");
        let dl = painted(&book, Viewport::default(), &content_only());

        let images = image_commands(&dl);
        assert_eq!(images.len(), 2, "кадр: {images:?}");
        // One-cell B2: столбец 1 (64) и строка 1 (20) плюс 96×72 пикселя.
        assert_eq!(images[0], (64.0, 20.0, 96.0, 72.0, 0));
        // Two-cell E3:H8 — 4 столбца по 64 и 6 строк по 20; обе картинки
        // ссылаются на одну media, поэтому id у них общий.
        assert_eq!(images[1], (256.0, 40.0, 256.0, 120.0, 0));

        // С заголовками картинка сдвигается вместе с листом: 44 пикселя
        // полосы строк и 20 — столбцов.
        let with_headers = painted(&book, Viewport::default(), &PaintOptions::default());
        assert_eq!(
            image_commands(&with_headers)[0],
            (108.0, 40.0, 96.0, 72.0, 0)
        );
    }

    #[test]
    fn jpeg_fixture_paints_offsets_inside_cells() {
        let book = open_fixture("images-jpeg.xlsx");
        let dl = painted(&book, Viewport::default(), &content_only());

        let images = image_commands(&dl);
        assert_eq!(images.len(), 2, "кадр: {images:?}");
        // One-cell C4: 2 столбца и 3 строки плюс 320000 и 90000 EMU смещения.
        let (x, y, w, h, id) = images[0];
        assert!(near(x, 2.0 * 64.0 + 320_000.0 / 9525.0), "x = {x}");
        assert!(near(y, 3.0 * 20.0 + 90_000.0 / 9525.0), "y = {y}");
        assert_eq!((w, h, id), (120.0, 80.0, 0));
        // Two-cell A1:D5 — правый нижний угол тоже со смещением 320000 EMU.
        let (x, y, w, h, id) = images[1];
        assert_eq!((x, y, id), (0.0, 0.0, 0));
        assert!(near(w, 3.0 * 64.0 + 320_000.0 / 9525.0), "w = {w}");
        assert!(near(h, 4.0 * 20.0), "h = {h}");
    }

    #[test]
    fn over_data_fixture_keeps_overlay_order_and_skips_the_far_image() {
        let book = open_fixture("images-over-data.xlsx");
        let dl = painted(&book, Viewport::default(), &content_only());

        // Третья картинка приколота к столбцу 16382 — это далеко за окном.
        let images = image_commands(&dl);
        assert_eq!(images.len(), 2, "кадр: {images:?}");
        // Two-cell B2:E6 лежит поверх данных: 3 столбца по 64, 4 строки по 20.
        assert_eq!(images[0], (64.0, 20.0, 192.0, 80.0, 0));
        // One-cell F2 с размером 140×100 накрывает первую: команды идут в
        // порядке документа, то есть в порядке наложения.
        assert_eq!(images[1], (320.0, 20.0, 140.0, 100.0, 1));
    }

    #[test]
    fn images_without_media_or_size_are_skipped() {
        let content = images_content(vec![
            // media не разрешилась: рисовать нечем.
            sheet_image(one_cell(0, 0, 40.0, 30.0), None),
            // Нулевые и отрицательные размеры не дают видимой части.
            sheet_image(one_cell(0, 0, 0.0, 30.0), Some(0)),
            sheet_image(one_cell(0, 0, 40.0, 0.0), Some(0)),
            sheet_image(one_cell(0, 0, -40.0, -30.0), Some(0)),
            sheet_image(one_cell(0, 0, 40.0, 30.0), Some(3)),
        ]);
        let book = book_with(content, StyleTable::default());
        let dl = painted(&book, Viewport::default(), &content_only());

        assert_eq!(image_commands(&dl), [(0.0, 0.0, 40.0, 30.0, 3)]);
    }

    #[test]
    fn images_outside_the_window_are_not_painted_and_scrolling_clips_them() {
        let content = images_content(vec![sheet_image(one_cell(0, 0, 40.0, 30.0), Some(0))]);
        let book = book_with(content, StyleTable::default());

        // Прокрутка на ширину картинки уводит её из окна целиком.
        let viewport = Viewport {
            x: 40.0,
            ..Viewport::default()
        };
        let dl = painted(&book, viewport, &content_only());
        assert!(
            image_commands(&dl).is_empty(),
            "кадр: {:?}",
            image_commands(&dl)
        );

        // На половине ширины в кадре остаётся только видимая часть.
        let viewport = Viewport {
            x: 20.0,
            ..Viewport::default()
        };
        let dl = painted(&book, viewport, &content_only());
        assert_eq!(image_commands(&dl), [(0.0, 0.0, 20.0, 30.0, 0)]);
    }

    #[test]
    fn frozen_column_splits_the_image_between_quadrants() {
        // Закреплён столбец A, а картинка лежит на A1:B1 — ровно на границе.
        let mut content = images_content(vec![sheet_image(
            ImageAnchor::TwoCell {
                from: ImageMarker {
                    col: 0,
                    row: 0,
                    col_off: 0.0,
                    row_off: 0.0,
                },
                to: ImageMarker {
                    col: 2,
                    row: 1,
                    col_off: 0.0,
                    row_off: 0.0,
                },
            },
            Some(0),
        )]);
        content.view.pane = Some(crate::Pane {
            cols: 1,
            rows: 0,
            state: crate::PaneState::Frozen,
            active: crate::PaneKind::BottomRight,
            top_left: None,
        });
        let book = book_with(content, StyleTable::default());

        // Каждый квадрант рисует свою половину: закреплённая часть кончается на
        // 64 пикселях, прокручиваемая начинается там же — картинка не двоится.
        let dl = painted(&book, Viewport::default(), &content_only());
        assert_eq!(
            image_commands(&dl),
            [(0.0, 0.0, 64.0, 20.0, 0), (64.0, 0.0, 64.0, 20.0, 0)]
        );
    }

    #[test]
    fn zoom_scales_image_rectangles_and_positions() {
        let book = open_fixture("images-png.xlsx");
        let viewport = Viewport {
            w: 1600.0,
            h: 1200.0,
            scale: 2.0,
            ..Viewport::default()
        };
        let dl = painted(&book, viewport, &content_only());

        let images = image_commands(&dl);
        assert_eq!(images.len(), 2, "кадр: {images:?}");
        assert_eq!(images[0], (128.0, 40.0, 192.0, 144.0, 0));
        assert_eq!(images[1], (512.0, 80.0, 512.0, 240.0, 0));
    }

    #[test]
    fn images_are_painted_above_text() {
        let book = open_fixture("images-over-data.xlsx");
        let dl = painted(&book, Viewport::default(), &content_only());

        let last_text = (0..dl.len())
            .rfind(|i| matches!(dl.cmd(*i), Some(DrawCommand::Text { .. })))
            .expect("текст в кадре");
        let first_image = (0..dl.len())
            .find(|i| matches!(dl.cmd(*i), Some(DrawCommand::Image { .. })))
            .expect("картинки в кадре");
        assert!(
            first_image > last_text,
            "картинка {first_image} не перекрывает текст {last_text}"
        );
    }

    #[test]
    fn overlapping_images_follow_the_document_order() {
        let content = images_content(vec![
            sheet_image(one_cell(0, 0, 40.0, 30.0), Some(2)),
            sheet_image(one_cell(0, 0, 40.0, 30.0), Some(0)),
            sheet_image(one_cell(0, 0, 40.0, 30.0), Some(1)),
        ]);
        let book = book_with(content, StyleTable::default());
        let dl = painted(&book, Viewport::default(), &content_only());

        // Команды идут в порядке списка: последняя ложится поверх остальных.
        let ids: Vec<_> = image_commands(&dl)
            .into_iter()
            .map(|(_, _, _, _, id)| id)
            .collect();
        assert_eq!(ids, [2, 0, 1]);
    }
}
