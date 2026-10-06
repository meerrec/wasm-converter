//! Разбивка листа на страницы: срез страницы и ячейки, попавшие в него.
//!
//! Лист режется по строкам и по столбцам, и ни строка, ни столбец между
//! страницами не рвутся: как только низ очередной строки выходит за нижнюю
//! границу области содержимого, начинается новая полоса строк; как только
//! накопленная ширина столбцов перерастает её ширину — новая полоса столбцов.
//! Высоты строк и ширины столбцов берутся из раскладки абсолютными, поэтому
//! пустые строки внутри листа занимают на странице своё место: разрыва под
//! ними не подразумевается. Пустые строки сверху и пустые столбцы слева не
//! печатаются — первой странице задаёт верх первой строки с ячейкой, а полосы
//! начинаются с первого используемого столбца. Страницы без ячеек не создаются.
//!
//! Порядок страниц — как в Excel при «вниз, потом вправо»: полоса строк
//! исчерпывается целиком, прежде чем начнётся следующая полоса столбцов.
//!
//! Шапка ([`PageConfig::repeat_header_rows`] первых строк листа) и первые
//! столбцы ([`PageConfig::repeat_first_columns`]) повторяются на каждой
//! странице: шапка — сверху, столбцы — слева. Из потока они вырезаны, поэтому
//! дубля на первой странице нет, а их высота и ширина вычитаются из полезной
//! области полосы: иначе на шапку налезли бы строки, которых она лишила места.
//!
//! Ячейка объединения рисуется один раз — на своей левой верхней: остальные
//! пусты и отдельного текста не несут. Объединение, пересёкшее границу полосы,
//! рисуется целиком от своей левой верхней: разрыв объединений между полосами
//! не поддержан.
//!
//! Центрирование ([`PageConfig::center_horizontally`] и
//! [`PageConfig::center_vertically`]) сдвигает полосу набора страницы — вместе с
//! повторяемыми частями — к середине области содержимого. Считается оно здесь,
//! а применяет сдвиг painter: срез остаётся в координатах листа.

use std::ops::Range;

use doc_converter_xlsx::layout::SheetLayout;
use doc_converter_xlsx::{Cell, CellRef, Sheet, Workbook};

use crate::fonts::{Face, FaceSet};
use crate::layout::{PageGeometry, RectPx};
use crate::options::PageConfig;
use crate::styles::{self, CellStyle};

/// Запас при сравнении ширин, пиксели раскладки.
///
/// Пересчёт «точки → пиксели» округляет `f32`, и полоса, уложившаяся в область
/// содержимого ровно, может оказаться на доли пикселя шире её. 0,05 px на
/// печати — сотые доли миллиметра: незаметны, зато не дают лишней страницы.
const BAND_EPS_PX: f32 = 0.05;

/// Срез листа, который печатает одна страница.
///
/// Границы — полуинтервалы, как у [`Range`].
pub struct PageSlice {
    /// Строки листа, ячейки которых отнесены к странице: от первой попавшей
    /// до строки за последней (не включая её). Строки шапки сюда не входят.
    ///
    /// Рисование обходится сдвигами, а по границам срез опознают тесты.
    #[allow(dead_code)]
    pub rows: Range<u32>,
    /// Полоса столбцов листа; см. [`PageSlice::rows`].
    #[allow(dead_code)]
    pub cols: Range<u32>,
    /// Строки шапки, повторяемые на этой странице; пусто — шапки нет.
    #[allow(dead_code)]
    pub header_rows: Range<u32>,
    /// Первые столбцы, повторяемые слева; пусто — повтора нет.
    #[allow(dead_code)]
    pub repeat_cols: Range<u32>,
    /// Верх среза в пикселях раскладки; вычитается при записи операций
    /// ([`PageGeometry::with_page_top_px`]). У страницы с шапкой это верх
    /// полосы *минус* высота шапки: место шапки полосе уже не принадлежит.
    pub offset_y: f32,
    /// Левый край полосы столбцов в пикселях раскладки; вычитается при записи
    /// операций — сдвига по горизонтали у [`PageGeometry`] нет. Как и
    /// [`PageSlice::offset_y`], уменьшен на ширину повторяемых столбцов.
    pub offset_x: f32,
    /// Сдвиг содержимого вправо, пиксели раскладки: центрирование полосы в
    /// области содержимого ([`PageConfig::center_horizontally`]); ноль — без
    /// сдвига. Painter прибавляет его ко всем ячейкам страницы — и к ячейкам
    /// потока, и к повторяемым частям, — поэтому полоса сдвигается целиком.
    pub center_x_px: f32,
    /// Сдвиг содержимого вниз, пиксели раскладки; см. [`PageSlice::center_x_px`].
    pub center_y_px: f32,
}

/// Ячейка, готовая к отрисовке: ссылка на модель, её прямоугольник и стиль.
///
/// Прямоугольник — в координатах листа; сдвиг среза вычитает тот, кто рисует.
/// Исключение — ячейки шапки и повторяемых столбцов: они лежат на каждой
/// странице у её края, поэтому их прямоугольник заранее сдвинут на срез и
/// после вычитания оказывается в начале координат страницы.
pub struct PaintedCell<'a> {
    pub cell: &'a Cell,
    pub rect: RectPx,
    pub style: CellStyle,
}

/// Страница: срез листа и ячейки, в него попавшие.
pub struct SheetPage<'a> {
    pub slice: PageSlice,
    pub cells: Vec<PaintedCell<'a>>,
}

/// Лист, разбитый на страницы, и начертания, которые на них встретились.
pub struct SheetPagination<'a> {
    pub faces: FaceSet,
    pub pages: Vec<SheetPage<'a>>,
}

/// Ниже этого масштаба «уместить на N страниц» не опускается: 10% — предел
/// масштаба Excel, а мельче лист уже нечитаем, сколько бы страниц ни просили.
const MIN_FIT_SCALE: f32 = 0.1;

/// Масштаб печати: [`PageConfig::scale`], а при `fit_to_width`/`fit_to_height` —
/// ужимающий лист так, чтобы он занял не больше N страниц.
///
/// «Уместить на N страниц» перебивает ручной масштаб, а не умножается на него
/// (в Excel это взаимоисключающие настройки), и лист не увеличивает: то, что и
/// так помещается в N страниц, печатается в 100%. Заданы обе границы — берётся
/// меньший масштаб, как у Excel «уместить на N страниц в ширину и M в высоту».
#[must_use]
pub fn print_scale(cfg: &PageConfig, sheet: &Sheet, layout: &SheetLayout) -> f32 {
    let pages_wide = cfg.fit_to_width.filter(|pages| *pages > 0);
    let pages_tall = cfg.fit_to_height.filter(|pages| *pages > 0);
    if pages_wide.is_none() && pages_tall.is_none() {
        // Тот же запасной вариант, что у `PageGeometry::new`: нулевой масштаб
        // не напечатал бы ничего.
        return if cfg.scale > 0.0 { cfg.scale } else { 1.0 };
    }
    let Some(used) = sheet.cells.used_range() else {
        return 1.0;
    };
    // Область содержимого в пикселях раскладки — по геометрии при 100%, чтобы
    // не зависеть ни от `cfg.scale`, ни от собственного результата.
    let unit = PageGeometry::new(&PageConfig {
        scale: 1.0,
        ..cfg.clone()
    });
    let (content_width_px, content_height_px) = unit.content_px();
    let mut fit = 1.0_f32;
    if let Some(pages_wide) = pages_wide {
        let span_px = layout.column_x(used.last.col + 1) - layout.column_x(used.first.col);
        if span_px > 0.0 {
            // Число страниц — счётчик, много меньше 2^24: точности f32 хватает.
            #[allow(clippy::cast_precision_loss)]
            let by_width = pages_wide as f32 * content_width_px / span_px;
            fit = fit.min(by_width);
        }
    }
    if let Some(pages_tall) = pages_tall {
        let span_px = layout.row_y(used.last.row + 1) - layout.row_y(used.first.row);
        if span_px > 0.0 {
            #[allow(clippy::cast_precision_loss)]
            let by_height = pages_tall as f32 * content_height_px / span_px;
            fit = fit.min(by_height).min(1.0);
            fit = fit_height_scale(cfg, sheet, layout, used.last.row, pages_tall, fit);
        }
    }
    fit.min(1.0)
}

/// Ужать масштаб «уместить по высоте» настолько, чтобы полос по высоте стало не
/// больше `pages`.
///
/// Аналитической оценки мало: строки не рвутся (при `avoid_row_break`), остаток
/// полосы пропадает, и страниц выходит на одну больше. Подбор ищет наибольший
/// масштаб, укладывающийся в `pages`, двоичным поиском: `low` всегда уклады-
/// вается, `high` — никогда.
fn fit_height_scale(
    cfg: &PageConfig,
    sheet: &Sheet,
    layout: &SheetLayout,
    last_row: u32,
    pages: u32,
    candidate: f32,
) -> f32 {
    // Ниже 10% не подбираем, а то, что и так ужато сильнее (например, по
    // ширине), не трогаем: подбор только ужимает.
    if candidate <= MIN_FIT_SCALE {
        return candidate;
    }
    // Страниц — счётчик, много меньше 2^32: разрядности usize хватает везде.
    let target = usize::try_from(pages).unwrap_or(usize::MAX);
    let bands_at = |scale: f32| {
        let page = PageGeometry::new(&PageConfig {
            scale,
            ..cfg.clone()
        });
        let header_end = repeat_count(cfg.repeat_header_rows, last_row);
        row_bands(
            sheet,
            layout,
            &page,
            cfg,
            header_end,
            layout.row_y(header_end),
        )
        .len()
    };
    if bands_at(candidate) <= target {
        return candidate;
    }
    let mut low = MIN_FIT_SCALE;
    if bands_at(low) > target {
        return low; // и в 10% не уложились: ужимать больше некуда
    }
    let mut high = candidate;
    while low < high {
        // `f32::midpoint` появился только в Rust 1.85, а MSRV workspace — 1.82.
        #[allow(clippy::manual_midpoint)]
        let mid = (low + high) / 2.0;
        // Разница перестала быть различимой во f32 — дальше искать нечем.
        if mid <= low || mid >= high {
            break;
        }
        if bands_at(mid) <= target {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

/// Разбить ячейки листа на страницы по строкам и столбцам и собрать нужные
/// начертания.
///
/// Геометрия должна быть уже с итоговым масштабом ([`print_scale`]): от него
/// зависят и полосы, и то, сколько строк и столбцов в них помещается.
///
/// Ячейки шапки и повторяемых столбцов попадают на каждую страницу; чтобы
/// painter поставил их к краю страницы тем же вычетом среза, их прямоугольники
/// сдвинуты на срез заранее (см. [`PaintedCell`]).
pub fn paginate<'a>(
    book: &Workbook,
    sheet: &'a Sheet,
    layout: &SheetLayout,
    page: &PageGeometry,
    cfg: &PageConfig,
) -> SheetPagination<'a> {
    let Some(used) = sheet.cells.used_range() else {
        return SheetPagination {
            faces: FaceSet::default(),
            pages: vec![empty_page()],
        };
    };
    let header_rows = 0..repeat_count(cfg.repeat_header_rows, used.last.row);
    let repeat_cols = 0..repeat_count(cfg.repeat_first_columns, used.last.col);
    // Верх шапки и левый край повторяемых столбцов — начало листа: отсчёт
    // повторяемых частей идёт от первой строки и первого столбца.
    let header_h_px = layout.row_y(header_rows.end);
    let repeat_w_px = layout.column_x(repeat_cols.end);

    let mut bands_x = column_bands(repeat_cols.end, used.last.col, layout, page, repeat_w_px);
    let mut bands_y = row_bands(sheet, layout, page, cfg, header_rows.end, header_h_px);
    // Повторяемая часть может занять лист целиком: полос потока тогда нет, но
    // страница с одной шапкой остаться должна — пустой срез её и описывает.
    if bands_x.is_empty() {
        bands_x.push(repeat_cols.end..repeat_cols.end);
    }
    if bands_y.is_empty() {
        bands_y.push(RowBand {
            rows: header_rows.end..header_rows.end,
            offset_y: header_h_px,
        });
    }
    // Сдвиги полос — то, что painter вычтет из прямоугольников ячеек потока.
    let shift_x: Vec<f32> = bands_x
        .iter()
        .map(|band| layout.column_x(band.start) - repeat_w_px)
        .collect();
    let shift_y: Vec<f32> = bands_y
        .iter()
        .map(|band| band.offset_y - header_h_px)
        .collect();

    let mut faces = FaceSet::default();
    // Корзины по числу полос: индекс — «полоса столбцов * полос строк + полоса
    // строк», поэтому обход корзин по порядку и есть порядок страниц.
    let mut buckets: Vec<Vec<PaintedCell<'a>>> = (0..bands_x.len() * bands_y.len())
        .map(|_| Vec::new())
        .collect();
    for (row, row_cells) in sheet.cells.rows() {
        // Ячейка шапки идёт во все полосы строк, ячейка потока — в свою. При
        // разрыве внутри строки (A4) полос у неё две: строка продолжается на
        // следующей странице, и ячейка нужна обеим.
        let in_header = header_rows.contains(&row);
        let row_band = bands_y.partition_point(|band| band.rows.end <= row)
            ..bands_y.partition_point(|band| band.rows.start <= row);
        for cell in row_cells {
            let at = cell.at(row);
            let Some(rect) = cell_rect(sheet, layout, at) else {
                continue;
            };
            if rect.w <= 0.0 || rect.h <= 0.0 {
                continue;
            }
            let in_repeat = repeat_cols.contains(&at.col);
            let col_band = bands_x.partition_point(|band| band.end <= at.col);
            let rows = if in_header {
                0..bands_y.len()
            } else {
                row_band.clone()
            };
            let cols = if in_repeat {
                0..bands_x.len()
            } else {
                col_band..col_band + 1
            };
            let style = styles::resolve(book, cell);
            faces.insert(Face::of(style.bold, style.italic));
            for bx in cols {
                for by in rows.clone() {
                    let mut pinned = rect;
                    if in_repeat {
                        pinned.x += shift_x[bx];
                    }
                    if in_header {
                        pinned.y += shift_y[by];
                    }
                    buckets[bx * bands_y.len() + by].push(PaintedCell {
                        cell,
                        rect: pinned,
                        style: style.clone(),
                    });
                }
            }
        }
    }
    let mut pages = Vec::with_capacity(buckets.len());
    for (index, cells) in buckets.into_iter().enumerate() {
        if cells.is_empty() {
            continue;
        }
        let (col_band, row_band) = (index / bands_y.len(), index % bands_y.len());
        pages.push(SheetPage {
            slice: page_slice(
                &bands_y[row_band],
                &bands_x[col_band],
                layout,
                page,
                cfg,
                &header_rows,
                &repeat_cols,
            ),
            cells,
        });
    }
    if pages.is_empty() {
        pages.push(empty_page());
    }
    SheetPagination { faces, pages }
}

/// Сколько первых строк (столбцов) листа повторять.
///
/// Запрошено может быть больше, чем есть на листе: лишнее отбрасывается —
/// шапка за конец используемого диапазона не выходит и на границе не паникует.
fn repeat_count(requested: usize, last: u32) -> u32 {
    // Счёт строк много меньше u32::MAX, приведение насыщающее.
    let requested = u32::try_from(requested).unwrap_or(u32::MAX);
    requested.min(last.saturating_add(1))
}

/// Сдвиг полосы к середине области содержимого: половина пустого места.
///
/// Полоса шире области не сдвигается: центрировать её нечем, а отрицательный
/// сдвиг увёл бы содержимое за край области — в поле.
fn center_shift(content_px: f32, band_px: f32) -> f32 {
    ((content_px - band_px) / 2.0).max(0.0)
}

/// Страница без ячеек: пустой лист или лист, где все ячейки скрыты.
///
/// Центрировать нечего: оба сдвига — ноль, сколько бы ни просили настройки.
fn empty_page() -> SheetPage<'static> {
    SheetPage {
        slice: PageSlice {
            rows: 0..0,
            cols: 0..0,
            header_rows: 0..0,
            repeat_cols: 0..0,
            offset_y: 0.0,
            offset_x: 0.0,
            center_x_px: 0.0,
            center_y_px: 0.0,
        },
        cells: Vec::new(),
    }
}

/// Полоса строк: срез строк и его верх в пикселях раскладки.
struct RowBand {
    rows: Range<u32>,
    offset_y: f32,
}

/// Срез страницы: полосы строк и столбцов, повторяемые части, их сдвиг и
/// центрирование.
fn page_slice(
    rows: &RowBand,
    cols: &Range<u32>,
    layout: &SheetLayout,
    page: &PageGeometry,
    cfg: &PageConfig,
    header_rows: &Range<u32>,
    repeat_cols: &Range<u32>,
) -> PageSlice {
    let mut slice = PageSlice {
        rows: rows.rows.clone(),
        cols: cols.clone(),
        header_rows: header_rows.clone(),
        repeat_cols: repeat_cols.clone(),
        offset_y: rows.offset_y - layout.row_y(header_rows.end),
        offset_x: layout.column_x(cols.start) - layout.column_x(repeat_cols.end),
        center_x_px: 0.0,
        center_y_px: 0.0,
    };
    // Центрируется полоса вместе с повторяемыми частями: на странице они
    // печатаются одним блоком, и делить пополам нужно его целиком.
    if cfg.center_horizontally {
        let band_width_px = layout.column_x(cols.end) - layout.column_x(cols.start);
        let (content_width_px, _) = page.content_px();
        slice.center_x_px = center_shift(
            content_width_px,
            layout.column_x(repeat_cols.end) + band_width_px,
        );
    }
    if cfg.center_vertically {
        let band_height_px = layout.row_y(rows.rows.end) - rows.offset_y;
        let (_, content_height_px) = page.content_px();
        slice.center_y_px = center_shift(
            content_height_px,
            layout.row_y(header_rows.end) + band_height_px,
        );
    }
    slice
}

/// Строка потока: номер и границы в пикселях раскладки.
struct FlowRow {
    row: u32,
    top: f32,
    bottom: f32,
}

/// Полосы строк: строка через границу не переносится — она начинает новую.
///
/// Строки шапки в поток не входят, а её высота уменьшает место, которое полоса
/// может занять: на странице шапка стоит сверху и отнимает его у строк.
///
/// Строка, не поместившаяся целиком, начинает следующую полосу — как у Excel.
/// При [`PageConfig::avoid_row_break`] = `false` разрыв проходит внутри строки:
/// полоса доигрывает её верх, а с той же позиции листа строка продолжается на
/// следующей странице (поэтому полосы перекрываются строкой, а неполная полоса
/// уезжает в нижнее поле). Строка выше целой страницы не дробится и в этом
/// режиме: она занимает полосу целиком и обрезается краем листа.
///
/// [`PageConfig::orphan_rows`] и [`PageConfig::widow_rows`] сдвигают разрыв
/// между строками: первый — к началу блока, если внизу остаётся слишком мало
/// его строк, второй — вверх, если следующая страница началась бы слишком
/// коротким хвостом блока. На разрыв внутри строки они не влияют: границей
/// распоряжается уже `avoid_row_break`.
fn row_bands(
    sheet: &Sheet,
    layout: &SheetLayout,
    page: &PageGeometry,
    cfg: &PageConfig,
    header_end: u32,
    header_h_px: f32,
) -> Vec<RowBand> {
    let (_, content_h_px) = page.content_px();
    let max_h_px = (content_h_px - header_h_px).max(0.0);
    let flow: Vec<FlowRow> = sheet
        .cells
        .rows()
        .filter(|(row, _)| *row >= header_end)
        .map(|(row, _)| FlowRow {
            row,
            top: layout.row_y(row),
            bottom: layout.row_y(row + 1),
        })
        .collect();
    let blocks = block_starts(&flow);
    let mut bands: Vec<RowBand> = Vec::new();
    let mut start = 0;
    let mut offset = flow.first().map_or(0.0, |row| row.top);
    while start < flow.len() {
        let limit = offset + max_h_px;
        let mut end = start;
        while end < flow.len() && flow[end].bottom <= limit {
            end += 1;
        }
        // Разрыв внутри строки: она видна и здесь, и на следующей полосе,
        // которая начнётся с той же позиции листа. Строка выше страницы
        // (нулевая высота полосы — вырожденная геометрия) не дробится.
        let split = end < flow.len()
            && !cfg.avoid_row_break
            && max_h_px > 0.0
            && flow[end].top < limit
            && flow[end].bottom - flow[end].top <= max_h_px;
        let (next_start, next_offset) = if split {
            end += 1;
            (end - 1, limit)
        } else {
            if end == start {
                // Строка выше страницы: она одна во всей полосе.
                end = start + 1;
            }
            end = break_with_blocks(&flow, &blocks, start, end, cfg);
            (end, flow.get(end).map_or(limit, |row| row.top))
        };
        bands.push(RowBand {
            rows: flow[start].row..flow[end - 1].row + 1,
            offset_y: offset,
        });
        start = next_start;
        offset = next_offset;
    }
    bands
}

/// Начала блоков в потоке: индексы, с которых начинается новый блок.
///
/// Блок — отрезок подряд идущих строк листа: пустая строка его разрывает, а
/// с ней и разрывы страниц бывают уместны. Блок и есть «абзац» для правил
/// сирот и вдов ([`PageConfig::orphan_rows`], [`PageConfig::widow_rows`]).
fn block_starts(flow: &[FlowRow]) -> Vec<usize> {
    let mut starts = Vec::new();
    for (index, row) in flow.iter().enumerate() {
        if index == 0 || flow[index - 1].row + 1 != row.row {
            starts.push(index);
        }
    }
    starts
}

/// Сдвинуть разрыв между строками `start..end` по правилам блоков.
///
/// Возвращает конец полосы: разрыв либо остаётся на месте, либо уезжает к
/// началу блока (сирота) или вверх (вдова). Сирота важнее вдовы: сдвиг к началу
/// блока сразу делает хвост целым, а подъём ради вдовы не имеет права оставить
/// внизу меньше сиротского минимума. Блоку, которому тесно на обе границы, как
/// в Word, места на странице не находится — он уходит на следующую целиком.
fn break_with_blocks(
    flow: &[FlowRow],
    blocks: &[usize],
    start: usize,
    end: usize,
    cfg: &PageConfig,
) -> usize {
    if end >= flow.len() {
        return end; // разрыва нет: конец листа
    }
    let block = blocks.partition_point(|first| *first <= end) - 1;
    let block_start = blocks[block];
    let block_end = blocks.get(block + 1).copied().unwrap_or(flow.len());
    // Сирота: внизу страницы мало строк блока, который на ней не кончается, —
    // блок уходит на следующую страницу целиком.
    if cfg.orphan_rows > 0
        && block_start < end
        && end - block_start < cfg.orphan_rows
        && block_start > start
    {
        return block_start;
    }
    // Вдова: хвост блока на следующей странице короче `widow_rows` — строки
    // подтягиваются с этой страницы, но не все: страница не остаётся пустой.
    if cfg.widow_rows > 0 && block_start < end && block_end - end < cfg.widow_rows {
        // Блок может быть короче запрошенного хвоста: `saturating_sub` не даёт
        // разрыву уехать за начало потока; тогда вдова неисполнима, и блок
        // уходит на следующую страницу целиком — но лишь если страница с ним
        // не опустеет.
        let kept = block_end.saturating_sub(cfg.widow_rows);
        if kept <= block_start {
            return if block_start > start {
                block_start
            } else {
                end
            };
        }
        // Подъём ради вдовы мог уронить сироту ниже её минимума: блок, которому
        // мало места на обе границы, уходит на следующую страницу целиком.
        if cfg.orphan_rows > 0 && kept - block_start < cfg.orphan_rows && block_start > start {
            return block_start;
        }
        return kept.max(start + 1);
    }
    end
}

/// Полосы столбцов, покрывающие неповторяемую часть используемого диапазона.
///
/// Столбец между полосами не рвётся: полоса набирает их, пока накопленная
/// ширина помещается в область содержимого. Столбец шире страницы остаётся в
/// полосе один — как строка выше страницы в своей. Ширина повторяемых столбцов
/// из области содержимого вычтена.
fn column_bands(
    first: u32,
    last: u32,
    layout: &SheetLayout,
    page: &PageGeometry,
    repeat_w_px: f32,
) -> Vec<Range<u32>> {
    if first > last {
        return Vec::new();
    }
    let (max_w_px, _) = page.content_px();
    let max_w_px = (max_w_px - repeat_w_px).max(0.0);
    let mut bands = Vec::new();
    let mut start = first;
    let mut left_px = layout.column_x(first);
    for col in first..=last {
        let right_px = layout.column_x(col + 1);
        if col > start && right_px - left_px > max_w_px + BAND_EPS_PX {
            bands.push(start..col);
            start = col;
            left_px = layout.column_x(col);
        }
    }
    bands.push(start..last + 1);
    bands
}

/// Прямоугольник ячейки в пикселях раскладки; `None` — ячейку закрывает
/// объединение, её левая верхняя рисуется отдельно.
fn cell_rect(sheet: &Sheet, layout: &SheetLayout, at: CellRef) -> Option<RectPx> {
    let (first, last) = match sheet.merges.covering(at) {
        Some(range) if range.first != at => return None,
        Some(range) => (range.first, range.last),
        None => (at, at),
    };
    let x = layout.column_x(first.col);
    let y = layout.row_y(first.row);
    Some(RectPx::new(
        x,
        y,
        layout.column_x(last.col + 1) - x,
        layout.row_y(last.row + 1) - y,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;

    use doc_converter_xlsx::layout::SheetLayout;
    use doc_converter_xlsx::{
        Cell, CellValue, RowHeight, SharedStrings, SheetContent, SheetState, StyleTable, Theme,
        Workbook, WorksheetBuilder, WorksheetMeta,
    };

    use super::*;
    use crate::options::PageConfig;

    /// Открыть книгу из общего набора фикстур.
    fn open(name: &str) -> Workbook {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/xlsx")
            .join(name);
        let bytes = fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name}: {err}"))
    }

    /// Геометрия страницы так, как её собирает painter: масштаб уже с учётом
    /// [`print_scale`].
    fn geometry(cfg: &PageConfig, sheet: &Sheet, layout: &SheetLayout) -> PageGeometry {
        let mut cfg = cfg.clone();
        cfg.scale = print_scale(&cfg, sheet, layout);
        PageGeometry::new(&cfg)
    }

    /// Срезы столбцов страниц в порядке нумерации.
    fn bands_of(pagination: &SheetPagination<'_>) -> Vec<Range<u32>> {
        pagination
            .pages
            .iter()
            .map(|page| page.slice.cols.clone())
            .collect()
    }

    #[test]
    fn empty_sheet_gets_one_empty_page() {
        let book = open("content-empty-sheet.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig::default();
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        assert_eq!(pagination.pages.len(), 1);
        let sheet_page = &pagination.pages[0];
        assert!(sheet_page.cells.is_empty(), "у пустого листа нет ячеек");
        assert_eq!(sheet_page.slice.rows, 0..0);
        assert_eq!(sheet_page.slice.cols, 0..0);
        // По битам, а не с допуском: начало пустой страницы — ровно ноль.
        assert_eq!(sheet_page.slice.offset_y.to_bits(), 0);
        assert_eq!(sheet_page.slice.offset_x.to_bits(), 0);
        assert_eq!(pagination.faces, FaceSet::default());
    }

    #[test]
    fn slices_tile_the_sheet_rows() {
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig::default();
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        assert!(
            pagination.pages.len() >= 10,
            "лист не разбит: страниц {}",
            pagination.pages.len()
        );
        let first = sheet
            .cells
            .rows()
            .next()
            .map(|(row, _)| row)
            .expect("строки есть");
        let last = sheet
            .cells
            .rows()
            .last()
            .map(|(row, _)| row)
            .expect("строки есть");
        assert_eq!(pagination.pages[0].slice.rows.start, first);
        assert_eq!(
            pagination
                .pages
                .last()
                .expect("страницы есть")
                .slice
                .rows
                .end,
            last + 1
        );

        for pair in pagination.pages.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            // Фикстура плотная: непустые строки идут подряд, поэтому конец
            // среза обязан совпасть с началом следующего.
            assert_eq!(
                a.slice.rows.end, b.slice.rows.start,
                "дыра или нахлёст среза"
            );
            assert!(
                a.slice.offset_y < b.slice.offset_y,
                "верх страницы не растёт"
            );
            // Верх среза — та же величина, что и начало строки в раскладке.
            assert_eq!(
                a.slice.offset_y.to_bits(),
                layout.row_y(a.slice.rows.start).to_bits()
            );
        }
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            assert!(!sheet_page.cells.is_empty(), "страница {index} без ячеек");
        }
    }

    #[test]
    fn every_eligible_cell_lands_exactly_once() {
        // Ширины листа: узкий (одна полоса), 20 столбцов, 100 столбцов,
        // столбцы своей ширины и ужатый «уместить на две страницы».
        let cases = [
            ("scale-ten-pages.xlsx", PageConfig::default()),
            ("content-dense.xlsx", PageConfig::default()),
            ("size-20x100x1.xlsx", PageConfig::default()),
            ("layout-column-widths.xlsx", PageConfig::default()),
            (
                "size-30x30x2.xlsx",
                PageConfig {
                    fit_to_width: Some(2),
                    ..PageConfig::default()
                },
            ),
        ];
        for (name, cfg) in cases {
            let book = open(name);
            let sheet = &book.sheets()[0];
            let layout = SheetLayout::new(sheet);
            let page = geometry(&cfg, sheet, &layout);
            let pagination = paginate(&book, sheet, &layout, &page, &cfg);

            // Отбор — тот же, что у пагинации: пригодное к отрисовке и не
            // скрытое объединением. Ячейки правее области содержимого больше не
            // выбрасываются: их несёт своя полоса столбцов.
            let mut expected: Vec<*const Cell> = sheet
                .cells
                .rows()
                .flat_map(|(row, cells)| cells.iter().map(move |cell| (row, cell)))
                .filter(|(row, cell)| {
                    cell_rect(sheet, &layout, cell.at(*row))
                        .is_some_and(|rect| rect.w > 0.0 && rect.h > 0.0)
                })
                .map(|(_, cell)| std::ptr::from_ref(cell))
                .collect();
            let mut drawn: Vec<*const Cell> = pagination
                .pages
                .iter()
                .flat_map(|sheet_page| {
                    sheet_page
                        .cells
                        .iter()
                        .map(|painted| std::ptr::from_ref(painted.cell))
                })
                .collect();

            let unique: HashSet<*const Cell> = drawn.iter().copied().collect();
            assert_eq!(
                drawn.len(),
                unique.len(),
                "{name}: ячейка нарисована больше одного раза"
            );
            drawn.sort_unstable();
            expected.sort_unstable();
            assert_eq!(
                drawn, expected,
                "{name}: набор отрисованных ячеек разошёлся с отбором"
            );
        }
    }

    #[test]
    fn painted_cells_stay_inside_their_slice() {
        for name in ["scale-ten-pages.xlsx", "content-dense.xlsx"] {
            let book = open(name);
            let sheet = &book.sheets()[0];
            let layout = SheetLayout::new(sheet);
            let cfg = PageConfig::default();
            let page = geometry(&cfg, sheet, &layout);
            let pagination = paginate(&book, sheet, &layout, &page, &cfg);

            for (index, sheet_page) in pagination.pages.iter().enumerate() {
                for painted in &sheet_page.cells {
                    let row = layout.row_at(painted.rect.y);
                    let col = layout.column_at(painted.rect.x);
                    assert!(
                        sheet_page.slice.rows.contains(&row),
                        "{name}, страница {index}: строка {row} вне среза {:?}",
                        sheet_page.slice.rows
                    );
                    assert!(
                        sheet_page.slice.cols.contains(&col),
                        "{name}, страница {index}: столбец {col} вне среза {:?}",
                        sheet_page.slice.cols
                    );
                    assert!(
                        painted.rect.x >= sheet_page.slice.offset_x,
                        "{name}, страница {index}: ячейка левее своей полосы"
                    );
                }
            }
        }
    }

    #[test]
    fn thirty_columns_split_into_three_bands() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig::default();
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        // 30 столбцов по 64 px — 1920 px; в область содержимого A4 (180 мм ≈
        // 680,3 px) их укладывается по десять, значит полос ровно три, а строки
        // (30 × 20 px) помещаются в одну.
        assert_eq!(bands_of(&pagination), vec![0..10, 10..20, 20..30]);
        let (content_px, _) = page.content_px();
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            assert!(!sheet_page.cells.is_empty(), "страница {index} без ячеек");
            assert_eq!(sheet_page.slice.rows, 0..30);
            let width_px = layout.column_x(sheet_page.slice.cols.end)
                - layout.column_x(sheet_page.slice.cols.start);
            assert!(
                width_px <= content_px + BAND_EPS_PX,
                "полоса {index} шире области содержимого: {width_px} против {content_px}"
            );
            // Первый столбец полосы прижат к левому краю: сдвиг `offset_x` —
            // это то, что painter вычтет из координат ячеек.
            let left_px = sheet_page
                .cells
                .iter()
                .fold(f32::INFINITY, |acc, painted| acc.min(painted.rect.x));
            assert_eq!(
                left_px.to_bits(),
                sheet_page.slice.offset_x.to_bits(),
                "страница {index}: содержимое не прижато к левому краю полосы"
            );
        }
    }

    #[test]
    fn fit_to_width_shrinks_the_sheet_to_given_page_count() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            fit_to_width: Some(2),
            ..PageConfig::default()
        };
        let scale = print_scale(&cfg, sheet, &layout);
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        assert!(scale < 1.0, "лист не ужат: масштаб {scale}");
        // 1920 px ширины делятся ровно на две страницы по 15 столбцов, а по
        // высоте лист и в половинном масштабе помещается целиком.
        assert_eq!(bands_of(&pagination), vec![0..15, 15..30]);
        assert!(pagination
            .pages
            .iter()
            .all(|sheet_page| sheet_page.slice.rows == (0..30)));
    }

    #[test]
    fn fit_to_width_overrides_manual_scale() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);

        // В Excel «уместить на N страниц» и ручной масштаб — взаимоисключающие
        // настройки; раз в API выставлены обе, побеждает «уместить».
        let small = PageConfig {
            scale: 0.2,
            fit_to_width: Some(2),
            ..PageConfig::default()
        };
        let large = PageConfig {
            scale: 3.0,
            fit_to_width: Some(2),
            ..PageConfig::default()
        };
        let fitted = print_scale(&small, sheet, &layout);
        assert_eq!(
            fitted.to_bits(),
            print_scale(&large, sheet, &layout).to_bits(),
            "ручной масштаб не должен влиять на `fit_to_width`"
        );
        assert_ne!(
            fitted.to_bits(),
            0.2_f32.to_bits(),
            "масштаб не пересчитан под `fit_to_width`"
        );

        // Без `fit_to_width` масштаб берётся из настроек как есть.
        let manual = PageConfig {
            scale: 0.2,
            ..PageConfig::default()
        };
        assert_eq!(
            print_scale(&manual, sheet, &layout).to_bits(),
            0.2_f32.to_bits()
        );
    }

    #[test]
    fn fit_to_width_does_not_enlarge() {
        // Шесть столбцов отчёта и так помещаются в страницу: «уместить на одну»
        // не растягивает их на весь лист.
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            fit_to_width: Some(1),
            ..PageConfig::default()
        };

        assert_eq!(
            print_scale(&cfg, sheet, &layout).to_bits(),
            1.0_f32.to_bits()
        );
    }

    #[test]
    fn zero_fit_to_width_means_no_fit() {
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            scale: 0.5,
            fit_to_width: Some(0),
            ..PageConfig::default()
        };

        assert_eq!(
            print_scale(&cfg, sheet, &layout).to_bits(),
            0.5_f32.to_bits()
        );
    }

    #[test]
    fn page_numbering_goes_down_then_over() {
        let book = open("content-dense.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig::default();
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        // Полосы столбцов идут подряд, и внутри каждой сначала исчерпываются
        // полосы строк — «вниз, потом вправо», как в Excel по умолчанию.
        let mut runs: Vec<(Range<u32>, Vec<Range<u32>>)> = Vec::new();
        for sheet_page in &pagination.pages {
            match runs.last_mut() {
                Some((cols, rows)) if *cols == sheet_page.slice.cols => {
                    rows.push(sheet_page.slice.rows.clone());
                }
                _ => runs.push((
                    sheet_page.slice.cols.clone(),
                    vec![sheet_page.slice.rows.clone()],
                )),
            }
        }
        assert!(runs.len() > 1, "фикстура должна делиться и по столбцам");
        let rows = &runs[0].1;
        assert!(rows.windows(2).all(|pair| pair[0].end <= pair[1].start));
        for (cols, band_rows) in &runs {
            assert_eq!(
                band_rows, rows,
                "полоса столбцов {cols:?} разрезана по строкам иначе"
            );
        }
    }

    /// Указатели ячеек листа внутри среза строк и столбцов.
    fn cells_in(sheet: &Sheet, rows: Range<u32>, cols: Range<u32>) -> Vec<*const Cell> {
        sheet
            .cells
            .rows()
            .filter(|(row, _)| rows.contains(row))
            .flat_map(|(row, cells)| cells.iter().map(move |cell| (row, cell)))
            .filter(|(row, cell)| cols.contains(&cell.at(*row).col))
            .map(|(_, cell)| std::ptr::from_ref(cell))
            .collect()
    }

    /// A3, `DoD`: шапка есть на каждой странице и ровно один раз.
    #[test]
    fn header_repeats_on_every_page() {
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            repeat_header_rows: 1,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let header = cells_in(sheet, 0..1, 0..u32::MAX);
        assert_eq!(header.len(), 6, "шапка фикстуры — шесть ячеек");
        // Шапка в строку не добавила страниц: 600 строк тела и без неё дают 13.
        assert_eq!(pagination.pages.len(), 13);
        assert_eq!(pagination.pages[0].slice.offset_y.to_bits(), 0);
        assert_eq!(
            pagination.pages[0].slice.rows.start, 1,
            "шапка вне потока строк"
        );

        let header_h = layout.row_y(1);
        let mut flow: HashSet<*const Cell> = HashSet::new();
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            assert_eq!(sheet_page.slice.header_rows, 0..1);
            assert!(
                sheet_page.slice.rows.start >= 1,
                "страница {index}: поток заходит в шапку"
            );
            let mut on_page = 0;
            for painted in &sheet_page.cells {
                let at = std::ptr::from_ref(painted.cell);
                let local_y = painted.rect.y - sheet_page.slice.offset_y;
                if header.contains(&at) {
                    on_page += 1;
                    assert!(local_y < header_h, "страница {index}: шапка не у верха");
                } else {
                    assert!(
                        local_y >= header_h,
                        "страница {index}: строка потока залезла под шапку"
                    );
                    assert!(
                        flow.insert(at),
                        "страница {index}: ячейка потока нарисована дважды"
                    );
                }
            }
            assert_eq!(
                on_page,
                header.len(),
                "страница {index}: шапка не в полном составе"
            );
            // Первая строка потока прижата к шапке: сдвиг среза — ровно её верх
            // минус высота шапки, без зазора и нахлёста.
            let top = sheet_page
                .cells
                .iter()
                .filter(|painted| !header.contains(&std::ptr::from_ref(painted.cell)))
                .map(|painted| painted.rect.y - sheet_page.slice.offset_y)
                .fold(f32::INFINITY, f32::min);
            assert_eq!(
                top.to_bits(),
                header_h.to_bits(),
                "страница {index}: поток не прижат к шапке"
            );
        }
    }

    /// A3: высота шапки вычитается из полезной высоты полосы.
    #[test]
    fn header_height_shrinks_the_row_band() {
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let header: HashSet<*const Cell> = cells_in(sheet, 0..1, 0..u32::MAX).into_iter().collect();

        let rows_per_page = |cfg: &PageConfig| {
            let page = geometry(cfg, sheet, &layout);
            let pagination = paginate(&book, sheet, &layout, &page, cfg);
            pagination
                .pages
                .iter()
                .map(|sheet_page| {
                    sheet_page
                        .cells
                        .iter()
                        .filter(|painted| !header.contains(&std::ptr::from_ref(painted.cell)))
                        .map(|painted| layout.row_at(painted.rect.y))
                        .collect::<HashSet<u32>>()
                        .len()
                })
                .max()
                .expect("страницы есть")
        };

        // Строка фикстуры — 20 px, область содержимого A4 — 971,3 px: без шапки
        // в полосу влезает 48 строк, шапка в строку отнимает одну.
        assert_eq!(rows_per_page(&PageConfig::default()), 48);
        assert_eq!(
            rows_per_page(&PageConfig {
                repeat_header_rows: 1,
                ..PageConfig::default()
            }),
            47
        );
    }

    /// A3: первые столбцы есть на каждой странице и не повторяются в потоке.
    #[test]
    fn first_columns_repeat_on_every_page() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            repeat_first_columns: 2,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let pinned = cells_in(sheet, 0..u32::MAX, 0..2);
        assert_eq!(pinned.len(), 60, "два первых столбца — 60 ячеек");
        assert!(pagination.pages.len() > 1, "фикстура делится по столбцам");
        assert_eq!(
            pagination.pages[0].slice.cols.start, 2,
            "первые столбцы вне потока"
        );

        let repeat_w = layout.column_x(2);
        let mut flow: HashSet<*const Cell> = HashSet::new();
        let mut pinned_seen: HashSet<*const Cell> = HashSet::new();
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            assert_eq!(sheet_page.slice.repeat_cols, 0..2);
            assert!(
                sheet_page.slice.cols.start >= 2,
                "страница {index}: поток заходит на повторяемые столбцы"
            );
            let mut on_page = 0;
            for painted in &sheet_page.cells {
                let at = std::ptr::from_ref(painted.cell);
                let local_x = painted.rect.x - sheet_page.slice.offset_x;
                if pinned.contains(&at) {
                    on_page += 1;
                    pinned_seen.insert(at);
                    assert!(
                        local_x < repeat_w,
                        "страница {index}: повторяемый столбец не у левого края"
                    );
                } else {
                    assert!(
                        local_x >= repeat_w,
                        "страница {index}: столбец потока залез на повторяемые"
                    );
                    assert!(
                        flow.insert(at),
                        "страница {index}: ячейка потока нарисована дважды"
                    );
                }
            }
            assert_eq!(
                on_page,
                pinned.len(),
                "страница {index}: первые столбцы не в полном составе"
            );
            // Столбец потока прижат к повторяемым: сдвиг полосы — ровно её
            // левый край минус ширина повтора.
            let left = sheet_page
                .cells
                .iter()
                .filter(|painted| !pinned.contains(&std::ptr::from_ref(painted.cell)))
                .map(|painted| painted.rect.x - sheet_page.slice.offset_x)
                .fold(f32::INFINITY, f32::min);
            assert_eq!(
                left.to_bits(),
                repeat_w.to_bits(),
                "страница {index}: поток не прижат к первым столбцам"
            );
        }
        assert_eq!(
            pinned_seen.len(),
            pinned.len(),
            "часть повторяемых столбцов не нарисована"
        );
    }

    /// A3: повторов больше, чем строк и столбцов листа, — не паника, а лист,
    /// целиком ушедший в повторяемую часть.
    #[test]
    fn repeat_larger_than_sheet_does_not_panic() {
        let book = open("content-dense.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            repeat_header_rows: 10_000,
            repeat_first_columns: 10_000,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let total: usize = sheet.cells.rows().map(|(_, row)| row.len()).sum();
        assert_eq!(
            pagination.pages.len(),
            1,
            "весь лист — одна повторяемая часть"
        );
        let sheet_page = &pagination.pages[0];
        assert_eq!(sheet_page.slice.header_rows, 0..100);
        assert_eq!(sheet_page.slice.repeat_cols, 0..20);
        assert_eq!(sheet_page.slice.rows, 100..100);
        assert_eq!(sheet_page.slice.cols, 20..20);
        assert_eq!(
            sheet_page.cells.len(),
            total,
            "ячейки потерялись или удвоились"
        );
        // Полос потока нет — срез начинается с начала листа.
        assert_eq!(sheet_page.slice.offset_y.to_bits(), 0);
        assert_eq!(sheet_page.slice.offset_x.to_bits(), 0);
    }

    /// A3: шапка и первые столбцы вместе сходятся в левом верхнем углу.
    #[test]
    fn header_and_first_columns_meet_at_the_corner() {
        let book = open("content-dense.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            repeat_header_rows: 1,
            repeat_first_columns: 1,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        // 99 строк тела по 47 в полосе и 19 столбцов по девять — девять страниц.
        assert_eq!(pagination.pages.len(), 9);
        // Угловая ячейка — пересечение шапки и первого столбца; она пересекается
        // с обеими повторяемыми частями и не должна ни пропасть, ни задвоиться.
        let corner_cells = cells_in(sheet, 0..1, 0..1);
        assert_eq!(corner_cells.len(), 1);
        let header_h = layout.row_y(1);
        let repeat_w = layout.column_x(1);
        let mut corners = 0;
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            assert_eq!(sheet_page.slice.header_rows, 0..1);
            assert_eq!(sheet_page.slice.repeat_cols, 0..1);
            let corner: Vec<_> = sheet_page
                .cells
                .iter()
                .filter(|painted| corner_cells.contains(&std::ptr::from_ref(painted.cell)))
                .collect();
            assert_eq!(corner.len(), 1, "страница {index}: угловая ячейка не одна");
            let painted = corner[0];
            assert_eq!(
                (painted.rect.x - sheet_page.slice.offset_x).to_bits(),
                0,
                "страница {index}: угол не прижат влево"
            );
            assert_eq!(
                (painted.rect.y - sheet_page.slice.offset_y).to_bits(),
                0,
                "страница {index}: угол не прижат вверх"
            );
            corners += 1;
            for painted in &sheet_page.cells {
                let local_x = painted.rect.x - sheet_page.slice.offset_x;
                let local_y = painted.rect.y - sheet_page.slice.offset_y;
                let in_corner =
                    local_x < repeat_w - BAND_EPS_PX && local_y < header_h - BAND_EPS_PX;
                assert!(
                    !in_corner || corner_cells.contains(&std::ptr::from_ref(painted.cell)),
                    "страница {index}: в угол залезла чужая ячейка"
                );
            }
        }
        assert_eq!(
            corners,
            pagination.pages.len(),
            "угол потерян на части страниц"
        );
    }

    /// Выключенное центрирование не сдвигает страницы: оба сдвига — ровно ноль.
    #[test]
    fn centering_off_keeps_slices_in_place() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig::default();
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        assert!(pagination.pages.len() > 1, "нужны страницы с полосами");
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            assert_eq!(
                sheet_page.slice.center_x_px.to_bits(),
                0,
                "страница {index}: сдвиг по горизонтали без настройки"
            );
            assert_eq!(
                sheet_page.slice.center_y_px.to_bits(),
                0,
                "страница {index}: сдвиг по вертикали без настройки"
            );
        }
    }

    /// Горизонтальное центрирование ставит каждую полосу столбцов посередине
    /// области содержимого: 30 столбцов по 64 px — три полосы по 640 px, и
    /// каждая делит остаток области (≈680,31 px) пополам.
    #[test]
    fn center_horizontally_centers_every_column_band() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            center_horizontally: true,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let (content_w_px, _) = page.content_px();
        assert_eq!(bands_of(&pagination), vec![0..10, 10..20, 20..30]);
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            let band_w_px = layout.column_x(sheet_page.slice.cols.end)
                - layout.column_x(sheet_page.slice.cols.start);
            let expected = (content_w_px - band_w_px) / 2.0;
            assert!(
                (sheet_page.slice.center_x_px - expected).abs() < 0.01,
                "страница {index}: сдвиг {} против ожидаемого {expected}",
                sheet_page.slice.center_x_px
            );
            assert!(sheet_page.slice.center_x_px > 0.0);
            assert_eq!(
                sheet_page.slice.center_y_px.to_bits(),
                0,
                "страница {index}: вертикаль не просили"
            );
        }
        // 640 px в области 180 мм (680,31 px) — сдвиг 20,16 px.
        assert!((pagination.pages[0].slice.center_x_px - 20.157).abs() < 0.01);
    }

    /// Повторяемые столбцы входят в ширину полосы: центрируется полоса вместе
    /// с повтором, поэтому последняя короткая полоса сдвигается сильнее полной.
    #[test]
    fn center_horizontally_counts_repeated_columns() {
        let book = open("size-30x30x2.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            center_horizontally: true,
            repeat_first_columns: 1,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let (content_w_px, _) = page.content_px();
        let repeat_w_px = layout.column_x(1);
        assert!(repeat_w_px > 0.0);
        assert!(pagination.pages.len() > 1);
        let mut shifts = Vec::new();
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            let band_w_px = layout.column_x(sheet_page.slice.cols.end)
                - layout.column_x(sheet_page.slice.cols.start);
            let expected = (content_w_px - repeat_w_px - band_w_px) / 2.0;
            assert!(
                (sheet_page.slice.center_x_px - expected).abs() < 0.01,
                "страница {index}: сдвиг {} против ожидаемого {expected}",
                sheet_page.slice.center_x_px
            );
            shifts.push(sheet_page.slice.center_x_px);
        }
        // Полосы разной ширины (последняя короче) — сдвиги различаются: ширину
        // задаёт полоса, а не рисунок непустых ячеек.
        assert!(
            shifts
                .windows(2)
                .any(|pair| (pair[0] - pair[1]).abs() > 1.0),
            "сдвиги совпали: {shifts:?}"
        );
    }

    /// Вертикальное центрирование ставит полосу строк посередине: короткая
    /// последняя полоса уезжает к середине сильнее полной.
    #[test]
    fn center_vertically_centers_every_row_band() {
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            center_vertically: true,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let (_, content_h_px) = page.content_px();
        assert!(pagination.pages.len() > 1);
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            // Без разрыва внутри строки верх полосы — верх её первой строки.
            let band_h_px =
                layout.row_y(sheet_page.slice.rows.end) - layout.row_y(sheet_page.slice.rows.start);
            let expected = (content_h_px - band_h_px) / 2.0;
            assert!(
                (sheet_page.slice.center_y_px - expected).abs() < 0.01,
                "страница {index}: сдвиг {} против ожидаемого {expected}",
                sheet_page.slice.center_y_px
            );
            assert!(sheet_page.slice.center_y_px >= 0.0);
        }
        let first = pagination.pages.first().expect("страницы есть");
        let last = pagination.pages.last().expect("страницы есть");
        assert!(
            last.slice.center_y_px > first.slice.center_y_px,
            "короткая полоса не уехала к середине сильнее полной"
        );
    }

    /// Шапка входит в центрируемую высоту: полоса строк вместе с повторяемой
    /// шапкой ставится посередине области содержимого.
    #[test]
    fn center_vertically_counts_repeated_header() {
        let book = open("scale-ten-pages.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            center_vertically: true,
            repeat_header_rows: 1,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let (_, content_h_px) = page.content_px();
        let header_h_px = layout.row_y(1);
        assert!(header_h_px > 0.0);
        for (index, sheet_page) in pagination.pages.iter().enumerate() {
            let band_h_px =
                layout.row_y(sheet_page.slice.rows.end) - layout.row_y(sheet_page.slice.rows.start);
            let expected = (content_h_px - header_h_px - band_h_px) / 2.0;
            assert!(
                (sheet_page.slice.center_y_px - expected).abs() < 0.01,
                "страница {index}: сдвиг {} против ожидаемого {expected}",
                sheet_page.slice.center_y_px
            );
        }
    }

    /// Полоса шире области содержимого не сдвигается влево: центрировать её
    /// нечем, а отрицательный сдвиг увёл бы содержимое в поле.
    #[test]
    fn center_shift_never_moves_content_off_the_page() {
        assert_eq!(center_shift(100.0, 160.0).to_bits(), 0);
        assert_eq!(center_shift(100.0, 100.0).to_bits(), 0);
        assert!((center_shift(100.0, 60.0) - 20.0).abs() < f32::EPSILON);
    }

    /// Пустой лист центрировать нечем: единственная страница остаётся без сдвига.
    #[test]
    fn empty_sheet_is_not_centered() {
        let book = open("content-empty-sheet.xlsx");
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            center_horizontally: true,
            center_vertically: true,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        assert_eq!(pagination.pages.len(), 1);
        let slice = &pagination.pages[0].slice;
        assert_eq!(slice.center_x_px.to_bits(), 0);
        assert_eq!(slice.center_y_px.to_bits(), 0);
    }

    // ── A4: запреты разрыва и умещение по высоте ────────────────────────────

    /// Часть пакета с содержимым листа: синтетическим листам важен её вид.
    const PART: &str = "xl/worksheets/sheet1.xml";

    /// Лист: ячейки в столбце 0 строк из `filled`, каждая строка до последней
    /// заполненной — со своей высотой (`heights[row]`, пропуск — 15 pt).
    fn sheet_rows(filled: &[u32], heights: &[f32]) -> Sheet {
        let last = filled.iter().copied().max().expect("строки есть");
        let mut builder = WorksheetBuilder::new(PART);
        for row in filled {
            builder
                .push(*row, Cell::new(0, 0, CellValue::Number(1.0)))
                .expect("ячейка вставляется");
        }
        let mut content = SheetContent {
            cells: builder.finish(),
            ..SheetContent::default()
        };
        for row in 0..=last {
            content.dims.rows.push(RowHeight {
                row,
                height: heights.get(row as usize).copied().unwrap_or(15.0),
                custom: true,
                hidden: false,
                outline_level: 0,
            });
        }
        Sheet::new(
            WorksheetMeta {
                name: "Лист1".into(),
                part: PART.into(),
                state: SheetState::Visible,
            },
            content,
        )
    }

    /// Лист из `count` непустых строк одинаковой высоты.
    fn uniform_rows(count: u32, height_pt: f32) -> Sheet {
        let filled: Vec<u32> = (0..count).collect();
        sheet_rows(&filled, &vec![height_pt; count as usize])
    }

    /// Полосы строк листа при настройках `cfg`; `header_end` — первая строка
    /// потока: шапку вырезает вызывающий, как это делает [`paginate`].
    fn bands_with(
        sheet: &Sheet,
        layout: &SheetLayout,
        cfg: &PageConfig,
        header_end: u32,
    ) -> Vec<RowBand> {
        let page = geometry(cfg, sheet, layout);
        row_bands(
            sheet,
            layout,
            &page,
            cfg,
            header_end,
            layout.row_y(header_end),
        )
    }

    /// Срезы строк полос — то же, что видно в [`PageSlice::rows`].
    fn band_rows(bands: &[RowBand]) -> Vec<Range<u32>> {
        bands.iter().map(|band| band.rows.clone()).collect()
    }

    /// A4: строка, не поместившаяся целиком, начинает следующую страницу, —
    /// как в Excel. В режиме `avoid_row_break = false` разрыв проходит по
    /// нижней границе области и строка видна на обеих страницах.
    #[test]
    fn avoid_row_break_moves_partial_row_whole_to_next_page() {
        // 48 строк по 20 px занимают 960 px из 971,3; следующая (100 px) не
        // влезает целиком, хотя её верх ещё в области содержимого.
        let mut heights = vec![15.0; 48];
        heights.push(75.0);
        let filled: Vec<u32> = (0..49).collect();
        let sheet = sheet_rows(&filled, &heights);
        let layout = SheetLayout::new(&sheet);

        let plain = bands_with(&sheet, &layout, &PageConfig::default(), 0);
        assert_eq!(
            band_rows(&plain),
            vec![0..48, 48..49],
            "неполная строка не перенесена целиком"
        );

        let split = PageConfig {
            avoid_row_break: false,
            ..PageConfig::default()
        };
        let bands = bands_with(&sheet, &layout, &split, 0);
        assert_eq!(band_rows(&bands), vec![0..49, 48..49]);
        let (_, content_h_px) = geometry(&split, &sheet, &layout).content_px();
        assert_eq!(
            bands[1].offset_y.to_bits(),
            content_h_px.to_bits(),
            "продолжение строки не прижато к нижней границе области"
        );
    }

    /// A4: строка выше целой страницы не дробится и в режиме разрыва.
    #[test]
    fn row_taller_than_page_is_never_split() {
        // 800 pt — это 1066,7 px, больше области содержимого A4 (971,3 px).
        let sheet = sheet_rows(&[0, 1], &[800.0, 15.0]);
        let layout = SheetLayout::new(&sheet);
        let split = PageConfig {
            avoid_row_break: false,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &split, 0)),
            vec![0..1, 1..2]
        );
    }

    /// A4: `orphan_rows` не оставляет внизу страницы слишком короткий хвост
    /// блока — хвост уезжает на следующую страницу вслед за блоком.
    #[test]
    fn orphan_rows_moves_short_block_tail_to_next_page() {
        // Блок A — строки 0..=6, пустая строка 7, блок B — 8..=11; по 100 px.
        let filled: Vec<u32> = (0..=6).chain(8..=11).collect();
        let sheet = sheet_rows(&filled, &[75.0; 12]);
        let layout = SheetLayout::new(&sheet);

        // По умолчанию внизу остаётся строка 8 — одна строка блока B.
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &PageConfig::default(), 0)),
            vec![0..9, 9..12]
        );

        // Минимум 1 — столько и осталось: разрыв не сдвигается.
        let one = PageConfig {
            orphan_rows: 1,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &one, 0)),
            vec![0..9, 9..12]
        );

        // Минимум 2 — остатка мало, блок B уходит на следующую страницу целиком.
        let two = PageConfig {
            orphan_rows: 2,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &two, 0)),
            vec![0..7, 8..12]
        );
    }

    /// A4: `widow_rows` подтягивает строки с предыдущей страницы, но не все:
    /// страница не остаётся пустой.
    #[test]
    fn widow_rows_pulls_rows_from_previous_page() {
        let sheet = uniform_rows(30, 37.5); // 30 строк по 50 px
        let layout = SheetLayout::new(&sheet);

        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &PageConfig::default(), 0)),
            vec![0..19, 19..30]
        );

        // Хвост 11 строк — минимум 11 разрыв не сдвигает.
        let eleven = PageConfig {
            widow_rows: 11,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &eleven, 0)),
            vec![0..19, 19..30]
        );

        // Минимум 12 — хвоста мало, разрыв поднимается на строку вверх.
        let twelve = PageConfig {
            widow_rows: 12,
            ..PageConfig::default()
        };
        let bands = bands_with(&sheet, &layout, &twelve, 0);
        assert_eq!(band_rows(&bands), vec![0..18, 18..30]);
        assert_eq!(
            bands[1].rows.end - bands[1].rows.start,
            12,
            "на следующей странице не ровно `widow_rows` строк"
        );
    }

    /// A4: сдвиг ради вдовы не оставляет внизу меньше сиротского минимума —
    /// блок, которому тесно на обе границы, уходит на следующую страницу.
    #[test]
    fn widow_shift_respects_orphan_minimum() {
        // Блок A — строки 0..=4, пустая строка 5, блок B — 6..=12; по 100 px.
        let filled: Vec<u32> = (0..=4).chain(6..=12).collect();
        let sheet = sheet_rows(&filled, &[75.0; 13]);
        let layout = SheetLayout::new(&sheet);

        // Внизу ровно 3 строки блока — сирота молчит, но вдова подняла бы
        // разрыв так, что внизу осталось бы 2: блок уходит целиком.
        let tight = PageConfig {
            orphan_rows: 3,
            widow_rows: 5,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &tight, 0)),
            vec![0..5, 6..13]
        );

        // С сиротой 2 сдвиг вдовы состоятелен: внизу 2 строки, сверху — ровно 5.
        let fits = PageConfig {
            orphan_rows: 2,
            widow_rows: 5,
            ..PageConfig::default()
        };
        let bands = bands_with(&sheet, &layout, &fits, 0);
        assert_eq!(band_rows(&bands), vec![0..8, 8..13]);
        assert_eq!(bands[1].rows.end - bands[1].rows.start, 5);
    }

    /// A4: вдова длиннее потока не роняет счёт (`usize` не уходит в минус):
    /// блок уходит на следующую страницу целиком.
    #[test]
    fn widow_longer_than_flow_does_not_panic() {
        // Поток — 20 строк по 50 px: блок A (0..=7), пустая строка 8, блок B
        // (9..=20); `widow_rows` больше всего потока.
        let filled: Vec<u32> = (0..=7).chain(9..=20).collect();
        let sheet = sheet_rows(&filled, &[37.5; 21]);
        let layout = SheetLayout::new(&sheet);
        let cfg = PageConfig {
            widow_rows: 25,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &cfg, 0)),
            vec![0..8, 9..21]
        );
    }

    /// A4: правила блоков доезжают до страниц [`paginate`], а не только до
    /// полос: ячейки не теряются и не дублируются.
    #[test]
    fn paginate_honors_widow_rows() {
        let book = Workbook::new(
            vec![uniform_rows(30, 37.5)],
            SharedStrings::default(),
            StyleTable::default(),
            Theme::default(),
            false,
        );
        let sheet = &book.sheets()[0];
        let layout = SheetLayout::new(sheet);
        let cfg = PageConfig {
            widow_rows: 12,
            ..PageConfig::default()
        };
        let page = geometry(&cfg, sheet, &layout);
        let pagination = paginate(&book, sheet, &layout, &page, &cfg);

        let slices: Vec<Range<u32>> = pagination
            .pages
            .iter()
            .map(|sheet_page| sheet_page.slice.rows.clone())
            .collect();
        assert_eq!(slices, vec![0..18, 18..30]);

        let drawn: Vec<*const Cell> = pagination
            .pages
            .iter()
            .flat_map(|sheet_page| {
                sheet_page
                    .cells
                    .iter()
                    .map(|painted| std::ptr::from_ref(painted.cell))
            })
            .collect();
        let unique: HashSet<*const Cell> = drawn.iter().copied().collect();
        assert_eq!(drawn.len(), 30, "часть ячеек не дошла до страниц");
        assert_eq!(unique.len(), 30, "ячейка нарисована дважды");
    }

    /// A4: `fit_to_height` ужимает лист под N страниц и только настолько.
    #[test]
    fn fit_to_height_shrinks_to_given_page_count() {
        let sheet = uniform_rows(200, 75.0); // по 100 px
        let layout = SheetLayout::new(&sheet);
        assert_eq!(
            bands_with(&sheet, &layout, &PageConfig::default(), 0).len(),
            23,
            "до ужатия лист занимает не 23 полосы"
        );

        let cfg = PageConfig {
            fit_to_height: Some(10),
            ..PageConfig::default()
        };
        let scale = print_scale(&cfg, &sheet, &layout);
        assert!(scale < 1.0, "лист не ужат: масштаб {scale}");
        let page = PageGeometry::new(&PageConfig {
            scale,
            ..cfg.clone()
        });
        assert!(
            row_bands(&sheet, &layout, &page, &cfg, 0, 0.0).len() <= 10,
            "полос больше десяти при масштабе {scale}"
        );

        // Масштаб наибольший из подходящих: чуть больше — и полос снова больше.
        let bigger = PageGeometry::new(&PageConfig {
            scale: scale * 1.01,
            ..cfg.clone()
        });
        assert!(row_bands(&sheet, &layout, &bigger, &cfg, 0, 0.0).len() > 10);
    }

    /// A4: `fit_to_height` не растягивает лист: помещающийся и так печатается
    /// в 100%.
    #[test]
    fn fit_to_height_does_not_enlarge() {
        let sheet = uniform_rows(10, 75.0); // 1000 px: две полосы при 100%
        let layout = SheetLayout::new(&sheet);
        let cfg = PageConfig {
            fit_to_height: Some(5),
            ..PageConfig::default()
        };
        assert_eq!(
            print_scale(&cfg, &sheet, &layout).to_bits(),
            1.0_f32.to_bits()
        );
    }

    /// A4: «уместить по высоте» перебивает ручной масштаб, как в Excel.
    #[test]
    fn fit_to_height_overrides_manual_scale() {
        let sheet = uniform_rows(200, 75.0);
        let layout = SheetLayout::new(&sheet);
        let fitted = |scale: f32| {
            print_scale(
                &PageConfig {
                    scale,
                    fit_to_height: Some(10),
                    ..PageConfig::default()
                },
                &sheet,
                &layout,
            )
        };
        assert_eq!(fitted(0.2).to_bits(), fitted(3.0).to_bits());
        assert_ne!(fitted(0.2).to_bits(), 0.2_f32.to_bits());

        let manual = PageConfig {
            scale: 0.2,
            ..PageConfig::default()
        };
        assert_eq!(
            print_scale(&manual, &sheet, &layout).to_bits(),
            0.2_f32.to_bits()
        );
    }

    /// A4: повторяемая шапка отнимает место у полос — подбор масштаба её
    /// учитывает.
    #[test]
    fn fit_to_height_counts_repeated_header() {
        let sheet = uniform_rows(200, 75.0);
        let layout = SheetLayout::new(&sheet);
        let plain = print_scale(
            &PageConfig {
                fit_to_height: Some(10),
                ..PageConfig::default()
            },
            &sheet,
            &layout,
        );
        let cfg = PageConfig {
            repeat_header_rows: 1,
            fit_to_height: Some(10),
            ..PageConfig::default()
        };
        let with_header = print_scale(&cfg, &sheet, &layout);
        assert!(
            with_header < plain,
            "шапка не отняла место: {with_header} против {plain}"
        );

        let page = PageGeometry::new(&PageConfig {
            scale: with_header,
            ..cfg.clone()
        });
        let bands = row_bands(&sheet, &layout, &page, &cfg, 1, layout.row_y(1));
        assert!(
            bands.len() <= 10,
            "с шапкой полос {} — больше десяти",
            bands.len()
        );
    }

    /// A4: место, отнятое шапкой, уводит неполную строку на следующую страницу
    /// целиком — при конфликте приоритет за `avoid_row_break`.
    #[test]
    fn avoid_row_break_counts_repeated_header() {
        // 41 строка по 100 px: строка 0 — шапка. На первой странице 8 строк
        // потока (девятая не влезает), дальше шапка отнимает место и у каждой.
        let sheet = uniform_rows(41, 75.0);
        let layout = SheetLayout::new(&sheet);
        let cfg = PageConfig {
            repeat_header_rows: 1,
            ..PageConfig::default()
        };
        let bands = bands_with(&sheet, &layout, &cfg, 1);
        assert_eq!(bands[0].rows.start, 1, "шапка попала в поток");
        assert_eq!(
            band_rows(&bands)[0],
            1..9,
            "первая страница: 8 строк потока"
        );
        assert_eq!(
            band_rows(&bands)[1],
            9..17,
            "вторая страница: шапка не отняла место"
        );
    }

    /// A4: шапка уменьшает полосу, но вдова всё равно добирает свой минимум на
    /// следующей странице.
    #[test]
    fn widow_rows_counts_repeated_header() {
        // Строка 0 — шапка, строки 1..=39 — поток по 100 px. С шапкой на
        // страницу входит 8 строк потока, без вдовы последняя — 7.
        let sheet = uniform_rows(40, 75.0);
        let layout = SheetLayout::new(&sheet);
        let plain = PageConfig {
            repeat_header_rows: 1,
            ..PageConfig::default()
        };
        let bands = bands_with(&sheet, &layout, &plain, 1);
        assert!(
            bands.iter().all(|band| band.rows.start >= 1),
            "шапка в потоке"
        );
        let last = bands.last().expect("полосы есть");
        assert_eq!(last.rows.end - last.rows.start, 7);

        let cfg = PageConfig {
            repeat_header_rows: 1,
            widow_rows: 8,
            ..PageConfig::default()
        };
        let bands = bands_with(&sheet, &layout, &cfg, 1);
        assert_eq!(bands.len(), 5, "вдова изменила число страниц");
        let last = bands.last().expect("полосы есть");
        assert_eq!(
            last.rows.end - last.rows.start,
            8,
            "вдова не добрала строки на шапке"
        );
    }

    /// A4: шапка выше листа не включает разрыв строки: при конфликте приоритет
    /// за `avoid_row_break` — дробить строку негде, вся полоса занята шапкой.
    #[test]
    fn header_taller_than_page_keeps_rows_whole() {
        // Шапка (строка 0) — 1000 px, больше области содержимого A4 (971,3 px).
        let sheet = sheet_rows(&[0, 1, 2, 3], &[750.0, 15.0, 15.0, 15.0]);
        let layout = SheetLayout::new(&sheet);
        let split = PageConfig {
            avoid_row_break: false,
            ..PageConfig::default()
        };
        assert_eq!(
            band_rows(&bands_with(&sheet, &layout, &split, 1)),
            vec![1..2, 2..3, 3..4],
            "строка потока раздроблена при нулевой полосе"
        );
    }
}
