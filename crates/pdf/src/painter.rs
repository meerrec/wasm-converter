//! Сборка страниц PDF: обход ячеек листа и запись операций.
//!
//! Порядок слоёв повторяет оба «экранных» пути (Excel и `xlsx::paint`):
//! сначала заливки, поверх них рамки, поверх всего текст. Поэтому обход листа
//! идёт тремя проходами по заранее собранному списку ячеек, а не одним.
//!
//! Лист режется на страницы заранее ([`crate::pagination`]): сюда приходит
//! срез страницы и её ячейки, отсюда — только геометрия и операции.
//!
//! Диаграммы листа рисуются последним слоем — над заливками, рамками и
//! текстом; картинки лежат между текстом и диаграммами, как в `xlsx::paint`.
//!
//! Колонтитулы и водяной знак (`overlay`) рисуются здесь же: их строки несут
//! символы, которых может не быть в ячейках, и без общего зондового прохода
//! подрезка шрифтов оставила бы у них нулевые глифы.
//!
//! Ещё не перенесены настройки [`PdfOptions`], за которыми стоит заметная
//! работа: закреплённые области — следующий срез спринта.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use doc_converter_render::display_list::Color;
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_ID};
use doc_converter_render::text_measure::measure_text;
use doc_converter_xlsx::layout::{SheetLayout, PX_PER_POINT};
use doc_converter_xlsx::paint::{anchor_rect, display_text};
use doc_converter_xlsx::{CellValue, Orientation, Sheet, Workbook, WorkbookImage};
use lopdf::Dictionary;

use crate::fonts::{EmbeddedFonts, Face};
use crate::image;
use crate::layout::{PageGeometry, RectPx};
use crate::options::{Margins, PageConfig, PageOrientation, PdfOptions};
use crate::overlay::{self, Align, OverlayMargins, PageContext, Watermark, WatermarkLayer};
use crate::pagination::{paginate, print_scale, PageSlice, PaintedCell, SheetPage};
use crate::pdf::content;
use crate::pdf::fonts::PreparedFont;
use crate::pdf::ops::{
    ExtendedGraphicsState, ExtendedGraphicsStateId, FontId, Line, LinePoint, Op, Point, Pt,
    TextItem, TextMatrix, XObjectId,
};
use crate::pdf::session::Session;
use crate::styles::to_pdf_color;
use crate::{annot, background, border, chart, fonts, text, PdfError};

/// Размер LRU-кэша метрик: столько же, сколько у canvas-пути.
const FONT_CACHE: usize = 4096;

/// Цвет линий печатной сетки — тот же, что у canvas (`dl_color::GRID` в
/// `xlsx::paint`): `D9D9D9`.
const GRID_COLOR: Color = Color(0xD9_D9_D9_FF);

/// Толщина линий сетки в точках: 1 px раскладки при 96 dpi — столько же
/// кладёт canvas (`draw_grid` пишет `stroke_w: 1.0`).
const GRID_WIDTH_PT: f32 = 0.75;

/// Допуск совпадения координат, пиксели раскладки: шапка, повторяемые
/// столбцы и поток сходятся на общей границе.
const COORD_EPS: f32 = 1e-3;

/// Кегль колонтитулов, pt: умолчание диалога Excel (10 pt).
///
/// Абсолютный, без масштаба печати: колонтитул живёт в поле страницы, а не в
/// полосе набора, и при `fit_to_width` не должен ужиматься вместе с ячейками.
const STAMP_FONT_SIZE_PT: f32 = 10.0;

/// Имя `/GS` водяного знака в ресурсах документа.
///
/// Фиксировано, а не случайно: операции знака собирает и зондовый проход,
/// который идёт до начала сессии, — имя должно быть известно обеим сторонам.
const WATERMARK_GS: &str = "WatermarkAlpha";

/// Открыть лист книги и собрать PDF всеми его страницами.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если индекса нет в книге; [`PdfError::Font`],
/// если байты шрифта не разобрались; [`PdfError::Io`], если приёмник
/// вернул ошибку записи.
pub fn export(
    book: &Workbook,
    sheet_index: usize,
    options: &PdfOptions,
) -> Result<Vec<u8>, PdfError> {
    let mut pdf = Vec::new();
    export_to(book, sheet_index, options, &mut pdf)?;
    Ok(pdf)
}

/// Собрать PDF и писать его в приёмник страница за страницей.
///
/// Частный случай [`export_book_to`] с одним листом: запись страниц общая,
/// поэтому одиночный экспорт и книга не расходятся ни пагинацией, ни
/// закладками. Страница живёт только до [`Session::write_page`]:
/// содержимое освобождается сразу после записи, поэтому пик не растёт с числом
/// страниц. Хвост (закладки, каталог, xref) дописывается в конце — приёмнику
/// не нужен `seek`.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если индекса нет в книге; [`PdfError::Font`],
/// если байты шрифта не разобрались; [`PdfError::Io`], если приёмник
/// вернул ошибку записи.
pub fn export_to<W: Write>(
    book: &Workbook,
    sheet_index: usize,
    options: &PdfOptions,
    out: &mut W,
) -> Result<(), PdfError> {
    export_book_to(book, &[sheet_index], options, out)
}

/// Добавить в набор символы, которые страницы листа выводят каждым начертанием.
///
/// Зонды нужны потому, что сессия подрезает шрифты до записи первой страницы
/// ([`Session::add_font`]), а подрезка перенумеровывает глифы: символ, не
/// попавший в зонды, выведется нулевым глифом — молча. Поэтому символы
/// собираются до записи, и источник у них один — та же [`draw_page`], что
/// пойдёт в PDF: проход зондов рисует страницы и выбрасывает операции,
/// оставляя только символы. Вариант «пре-скан текста» стоил бы почти столько
/// же (перенос и клиппинг чисел всё равно считает `break_lines`), но
/// разъехался бы с `draw_page` при первой правке клиппинга или переноса;
/// вариант «полный шрифт» весил бы ~760 `КиБ` на начертание против бюджета
/// `content-dense` в 32 `КиБ`.
///
/// Замер двойной отрисовки (release, macOS, `scale-500-pages.xlsx` — 480
/// страниц): проход зондов 35 мс, экспорт целиком 162 мс против 152 мс до
/// правки (+7 %): вторая отрисовка с прогретым кэшем метрик дешевле, чем та
/// экономия, которую даёт стриминговая запись. Полные числа 50/100/500 — в
/// `docs/sprint-7/streaming-report.md` (B3).
///
/// Диаграммы зондируются наравне с текстом: их подписи — такие же символы,
/// которых в ячейках может не быть, и без них подрезка оставила бы у
/// диаграммы нулевые глифы. Раскладка при этом не считается второй раз — она
/// приходит готовой ([`chart_layouts`]).
///
/// Ключ накопления — [`Face`], а не [`FontId`]: зонды идут до сессии, и id
/// шрифтов ещё не существует; обратное сопоставление делает
/// [`EmbeddedFonts::face`] по id, которые painter сам построил заранее
/// ([`EmbeddedFonts::from_faces`]).
///
/// Набор — параметр, а не результат: книга собирает его со всех своих листов
/// (шрифты подрезаются один раз на документ), поэтому лист добавляет в общий
/// набор свои символы, а не заводит собственный.
#[allow(clippy::too_many_arguments)] // Как у `draw_page`: связывать их структурой — прятать порядок слоёв.
fn collect_glyphs(
    glyphs: &mut BTreeMap<Face, BTreeSet<char>>,
    sheet: SheetFrame<'_>,
    pages: &[SheetPage<'_>],
    geometry: &PageGeometry,
    fonts: &EmbeddedFonts,
    registry: &mut FontRegistry,
    charts: &[chart::Layout],
    options: &PdfOptions,
    first_page_no: usize,
    page_count: usize,
) {
    let base = StampBase::from_options(options);
    for (index, sheet_page) in pages.iter().enumerate() {
        let page_geom = geometry.with_page_top_px(sheet_page.slice.offset_y);
        let mut ops: Vec<Op> = Vec::new();
        // Картинки в зондовый проход не передаются: `Op::UseXobject` символов
        // не несёт, а `XObject`'ы к этому моменту уже декодированы —
        // повторное размещение стоило бы клипов впустую.
        // Колонтитулы и водяной знак рисуются наравне с ячейками: их символы
        // обязаны попасть в подрезку шрифтов.
        draw_page(
            &mut ops,
            registry,
            fonts,
            &page_geom,
            sheet,
            sheet_page,
            charts,
            &[],
            options.page.print_grid_lines,
            base.page(first_page_no + index, page_count),
        );
        for op in &ops {
            if let Op::WriteText { font, items, .. } = op {
                let set = glyphs.entry(fonts.face(font)).or_default();
                for item in items {
                    if let TextItem::Text(text) = item {
                        set.extend(text.chars());
                    }
                }
            }
        }
    }
}

/// Прямоугольник ячейки в координатах страницы: из координат листа вычитается
/// сдвиг среза ([`PageSlice::offset_x`] — по горизонтали, `offset_y` делает
/// геометрия) и прибавляется сдвиг центрирования, посчитанный пагинацией.
///
/// Сдвиг центрирования получают все ячейки страницы, и потока, и повторяемые
/// части: их прямоугольники уже приведены к началу координат страницы, поэтому
/// общий сдвиг двигает полосу набора целиком.
pub(crate) fn page_rect(rect: RectPx, slice: &PageSlice) -> RectPx {
    RectPx::new(
        rect.x - slice.offset_x + slice.center_x_px,
        rect.y + slice.center_y_px,
        rect.w,
        rect.h,
    )
}

/// Разложить диаграммы листа в прямоугольники их якорей.
///
/// Раскладка не зависит от страницы, а [`draw_page`] вызывается дважды —
/// зондами шрифтов и записью, — поэтому считается здесь один раз на лист:
/// страницы потом только переносят готовые примитивы сдвигом среза. Пустые
/// раскладки (нулевой якорь, ни одной серии) отсеиваются заранее, чтобы
/// `draw_page` не открывал клип вхолостую.
fn chart_layouts(sheet: &Sheet, layout: &SheetLayout) -> Vec<chart::Layout> {
    sheet
        .charts
        .iter()
        .filter_map(|placed| {
            let (x, y, w, h) = anchor_rect(layout, placed.anchor);
            let chart = chart::Layout::new(RectPx::new(x, y, w, h), &placed.chart);
            (!chart.is_empty()).then_some(chart)
        })
        .collect()
}

/// Размещения картинок листа по страницам: индекс тот же, что у
/// [`SheetPage`] в [`SheetPagination::pages`](crate::pagination::SheetPagination).
///
/// Якорь берётся той же геометрией, что у диаграмм ([`anchor_rect`] и
/// [`page_rect`]), а видимая часть — пересечение с полосой страницы: так же
/// нарезает картинку canvas-путь (`xlsx::paint::draw_images`), растягивая её
/// на видимый прямоугольник. Картинка с повреждённым якорем (неположительная
/// сторона или сторона за пределами страницы) пропускается.
///
/// Декодирование и регистрация отделены от размещения: `XObject`'ы — ресурсы
/// документа, и все они обязаны лечь в сессию до первой страницы
/// ([`Session::add_image`]), поэтому байты собираются одним проходом
/// ([`image::Registry::collect`] в [`prepare_book`]), а операции — другим
/// ([`image_ops`]), когда имена уже назначены. Одинаковые байты дают один
/// `XObject`: одна и та же media-часть в нескольких якорях — один объект на
/// документ.
fn image_placements<'b>(
    book: &'b Workbook,
    sheet: &Sheet,
    layout: &SheetLayout,
    page: &PageGeometry,
    pages: &[SheetPage<'_>],
) -> Vec<Vec<image::Placement<'b>>> {
    let registry: BTreeMap<u32, &WorkbookImage> = book
        .images()
        .iter()
        .map(|image| (image.id, image))
        .collect();
    let (_, content_h_px) = page.content_px();
    let mut pages_placements = Vec::with_capacity(pages.len());
    for sheet_page in pages {
        let band_top = sheet_page.slice.offset_y;
        let band_bottom = band_top + content_h_px;
        let mut placements: Vec<image::Placement<'_>> = Vec::new();
        // Порядок `Sheet.images` — порядок наложения: первые лежат ниже, и
        // `place_all` сохраняет его в операциях.
        for placed in &sheet.images {
            let Some(image_id) = placed.image_id else {
                continue;
            };
            let Some(media) = registry.get(&image_id) else {
                continue;
            };
            let (x, y, w, h) = anchor_rect(layout, placed.anchor);
            let rect = page_rect(RectPx::new(x, y, w, h), &sheet_page.slice);
            let top = rect.y.max(band_top);
            let bottom = (rect.y + rect.h).min(band_bottom);
            if bottom <= top {
                continue;
            }
            let visible = RectPx::new(rect.x, top, rect.w, bottom - top);
            placements.push(image::Placement {
                id: &media.media,
                mime: &media.mime,
                bytes: &media.bytes,
                rect: page.rect_to_pt(visible).to_pdf(page.height_pt()),
            });
        }
        pages_placements.push(placements);
    }
    pages_placements
}

/// Операции картинок по страницам: [`Op::UseXobject`] с именами из реестра.
///
/// Вызывается после того, как painter назначил имена `XObject`'ам
/// ([`image::Registry::assign`]): до этого [`image::place_all`] отметил бы
/// каждую картинку пропуском «не зарегистрирована».
fn image_ops(registry: &image::Registry, placements: &[Vec<image::Placement<'_>>]) -> Vec<Vec<Op>> {
    placements
        .iter()
        .map(|page| image::place_all(registry, page).ops)
        .collect()
}

/// Линии печатной сетки: по рёбрам ячеек области печати.
///
/// Геометрия — эталон canvas-пути (`xlsx::paint::draw_grid`): рёбра столбцов и
/// строк раскладки, включая замыкающие рёбра последней строки и последнего
/// столбца. Источник решения при этом другой: canvas смотрит на экранную
/// настройку `view.show_grid_lines`, PDF — на печатную
/// [`PageConfig::print_grid_lines`](crate::PageConfig::print_grid_lines), как
/// Excel.
///
/// Линии не выходят за область содержимого: у полосы, разрезанной разрывом
/// строки, и у колонки шире страницы рёбра лежат в поле, а сетки в поле быть
/// не должно — обрезка та же, что у страницы Excel.
fn draw_grid(ops: &mut Vec<Op>, page: &PageGeometry, layout: &SheetLayout, slice: &PageSlice) {
    // Шапка и повторяемые столбцы стоят у краёв области содержимого:
    // пагинация сдвигает их прямоугольники на `slice.offset_*` — тот же сдвиг
    // снимает `page_rect` (`- offset + center`), поэтому в координатах листа
    // их края лежат на `offset` дальше от начала полосы. Пустой диапазон
    // пропускается: у него нет ни одной ячейки, а значит и рёбер.
    let mut xs: Vec<f32> = Vec::new();
    if !slice.repeat_cols.is_empty() {
        for col in slice.repeat_cols.start..=slice.repeat_cols.end {
            push_unique(&mut xs, layout.column_x(col) + slice.offset_x);
        }
    }
    if !slice.cols.is_empty() {
        for col in slice.cols.start..=slice.cols.end {
            push_unique(&mut xs, layout.column_x(col));
        }
    }
    let mut ys: Vec<f32> = Vec::new();
    if !slice.header_rows.is_empty() {
        for row in slice.header_rows.start..=slice.header_rows.end {
            push_unique(&mut ys, layout.row_y(row) + slice.offset_y);
        }
    }
    if !slice.rows.is_empty() {
        for row in slice.rows.start..=slice.rows.end {
            push_unique(&mut ys, layout.row_y(row));
        }
    }

    // Дальше координаты проходят тот же путь, что прямоугольники ячеек
    // (`page_rect` + `rect_to_pt`): сдвиг среза вычитается, центрирование
    // прибавляется. Пересчёт в точки поэтому `px_to_pt`, а не `y_px_to_pt`:
    // вертикальный сдвиг среза снят здесь, второй раз его вычитать нельзя.
    let to_page = |x: f32, center: f32, offset: f32| x - offset + center;
    let mut xs: Vec<f32> = xs
        .into_iter()
        .map(|x| to_page(x, slice.center_x_px, slice.offset_x))
        .collect();
    let mut ys: Vec<f32> = ys
        .into_iter()
        .map(|y| to_page(y, slice.center_y_px, slice.offset_y))
        .collect();

    let (content_right_px, content_bottom_px) = page.content_px();
    xs.retain(|x| (0.0..=content_right_px).contains(x));
    ys.retain(|y| (0.0..=content_bottom_px).contains(y));
    // Одиночная линия — не ребро ячейки: у среза без строк (страница с одной
    // шапкой) или без столбцов сетки нет вовсе, а огрызок в потоке — мусор.
    if xs.len() < 2 || ys.len() < 2 {
        return;
    }
    xs.sort_by(f32::total_cmp);
    ys.sort_by(f32::total_cmp);

    let x_pt = |x: f32| page.origin_x_pt() + page.px_to_pt(x);
    let y_pt = |y: f32| page.height_pt() - (page.origin_y_pt() + page.px_to_pt(y));
    let (left, right) = (x_pt(xs[0]), x_pt(xs[xs.len() - 1]));
    let (top, bottom) = (y_pt(ys[0]), y_pt(ys[ys.len() - 1]));

    // Кисть ставится один раз на всю сетку: у всех линий цвет и толщина общие.
    ops.push(Op::SetOutlineThickness {
        pt: Pt(GRID_WIDTH_PT),
    });
    ops.push(Op::SetOutlineColor {
        col: to_pdf_color(GRID_COLOR),
    });
    for y in ys {
        let y = y_pt(y);
        ops.push(segment((left, y), (right, y)));
    }
    for x in xs {
        let x = x_pt(x);
        ops.push(segment((x, top), (x, bottom)));
    }
}

/// Добавить координату, если такой ещё нет: шапка, повторяемые столбцы и поток
/// сходятся на общей границе, и линия на ней должна быть одна.
fn push_unique(coords: &mut Vec<f32>, value: f32) {
    if !coords
        .iter()
        .any(|existing| (existing - value).abs() < COORD_EPS)
    {
        coords.push(value);
    }
}

/// Отрезок по двум точкам в точках PDF (начало координат — левый нижний угол).
fn segment(from: (f32, f32), to: (f32, f32)) -> Op {
    let point = |(x, y): (f32, f32)| LinePoint {
        p: Point { x: Pt(x), y: Pt(y) },
        bezier: false,
    };
    Op::DrawLine {
        line: Line {
            points: vec![point(from), point(to)],
            is_closed: false,
        },
    }
}

/// Книга, раскладка её листа и колонтитулы: от страницы к странице не меняются.
#[derive(Clone, Copy)]
struct SheetFrame<'a> {
    book: &'a Workbook,
    layout: &'a SheetLayout,
    /// Сырые строки колонтитулов листа (коды Excel, см. [`sheet_overlay`]).
    header: &'a str,
    footer: &'a str,
}

/// Записать ячейки страницы в её операции.
///
/// Начертание выбирается по стилю ячейки, но лишь для записи текста: кегль и
/// перенос считает regular, как canvas-путь.
///
/// Прямоугольники ячеек приходят в координатах листа, поэтому сдвиг полосы
/// столбцов вычитается здесь: у [`PageGeometry`] сдвига по горизонтали нет, а по
/// вертикали его делает `with_page_top_px`.
// Восемь параметров: слои страницы зависят и от книги, и от геометрии, и от
// шрифтов — связывать их структурой ради порога clippy значит прятать порядок
// слоёв, который здесь и читается.
#[allow(clippy::too_many_arguments)]
fn draw_page(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    fonts: &EmbeddedFonts,
    page: &PageGeometry,
    sheet: SheetFrame<'_>,
    sheet_page: &SheetPage<'_>,
    charts: &[chart::Layout],
    images: &[Op],
    print_grid_lines: bool,
    stamp: PageStamp<'_>,
) {
    // Водяной знак «под содержимым» — раньше всего остального: ячейки,
    // картинки и диаграммы обязаны лечь поверх него.
    if let Some(watermark) = stamp.watermark {
        if watermark.layer == WatermarkLayer::Under {
            draw_watermark(ops, fonts, page, watermark);
        }
    }
    // Сетка — под всем содержимым: заливка и рамка её перекрывают, как в Excel.
    if print_grid_lines {
        draw_grid(ops, page, sheet.layout, &sheet_page.slice);
    }
    let book = sheet.book;
    let on_page = |painted: &PaintedCell<'_>| page_rect(painted.rect, &sheet_page.slice);
    for painted in &sheet_page.cells {
        if let Some(color) = painted.style.fill {
            background::fill_rect(
                ops,
                page.rect_to_pt(on_page(painted)),
                page.height_pt(),
                color,
            );
        }
    }
    for painted in &sheet_page.cells {
        if border::is_visible(&painted.style.border) {
            border::draw_border(
                ops,
                page.rect_to_pt(on_page(painted)),
                page.height_pt(),
                &painted.style.border,
                book.theme(),
            );
        }
    }
    for painted in &sheet_page.cells {
        let cell = painted.cell;
        let Some(mut value) = display_text(book, cell, painted.style.number_format) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let rect = on_page(painted);
        if matches!(cell.value, CellValue::Number(_)) {
            let size_px = painted.style.font_size_pt * PX_PER_POINT * page.scale();
            value = text::clip_number(value, size_px, text::inner_width_px(page, rect), registry);
        }
        text::draw_cell_text(
            ops,
            registry,
            fonts.id(Face::of(painted.style.bold, painted.style.italic)),
            page,
            &value,
            &painted.style,
            rect,
        );
    }
    // Картинки лежат поверх текста и под диаграммами — порядок canvas-пути
    // (`xlsx::paint::draw_images` вызывается между текстом и диаграммами).
    ops.extend_from_slice(images);
    draw_charts(ops, registry, fonts, page, sheet_page, charts);
    if let Some(watermark) = stamp.watermark {
        if watermark.layer == WatermarkLayer::Over {
            draw_watermark(ops, fonts, page, watermark);
        }
    }
    // Колонтитулы последними: они живут в поле страницы и рисуются поверх
    // всего, как в Excel.
    draw_header_footer(ops, registry, fonts, page, sheet, stamp);
}

/// Номер страницы и общие для документа части колонтитула — всё, что
/// [`draw_page`] нужно знать о месте страницы в документе.
#[derive(Clone, Copy)]
struct PageStamp<'a> {
    /// Номер страницы в документе, с 1 (код `&P`).
    page_no: usize,
    /// Всего страниц документа (код `&N`).
    page_count: usize,
    margins: OverlayMargins,
    /// `None` — водяного знака нет.
    watermark: Option<&'a Watermark>,
}

/// Части колонтитула, не зависящие от страницы: поля и водяной знак.
#[derive(Clone, Copy)]
struct StampBase<'a> {
    margins: OverlayMargins,
    watermark: Option<&'a Watermark>,
}

impl<'a> StampBase<'a> {
    fn from_options(options: &'a PdfOptions) -> Self {
        // Колонтитул живёт в поле страницы; поля листа, если их не задал
        // вызывающий, здесь не читаются — полоса колонтитула общая на
        // документ, а поля листов книги разные.
        let margins = options.page.margins.unwrap_or_default();
        Self {
            margins: OverlayMargins::from_mm(
                margins.top_mm,
                margins.right_mm,
                margins.bottom_mm,
                margins.left_mm,
            ),
            watermark: options.overlay.watermark.as_ref(),
        }
    }

    /// Страница с её номером; нумерация сквозная по документу — как у Excel.
    fn page(&self, page_no: usize, page_count: usize) -> PageStamp<'a> {
        PageStamp {
            page_no,
            page_count,
            margins: self.margins,
            watermark: self.watermark,
        }
    }
}

/// Колонтитулы листа: заданные в [`PdfOptions::overlay`] перебивают книжные
/// (`PrintSettings::odd_header`/`odd_footer`), пустая настройка берёт книжную.
///
/// `&P`/`&N` здесь ещё не разворачиваются: номера страниц известны только
/// после пагинации, и подстановку делает [`draw_header_footer`] на каждой
/// странице.
fn sheet_overlay<'a>(options: &'a PdfOptions, sheet: &'a Sheet) -> (&'a str, &'a str) {
    let header = if options.overlay.header.is_empty() {
        sheet.print.odd_header.as_deref().unwrap_or_default()
    } else {
        options.overlay.header.as_str()
    };
    let footer = if options.overlay.footer.is_empty() {
        sheet.print.odd_footer.as_deref().unwrap_or_default()
    } else {
        options.overlay.footer.as_str()
    };
    (header, footer)
}

/// Верхний и нижний колонтитулы страницы: три секции Excel в поле между краем
/// страницы и областью содержимого.
///
/// Строки приходят сырыми; `&P`/`&N` разворачиваются здесь, когда номер
/// страницы и их общее число уже известны.
fn draw_header_footer(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    fonts: &EmbeddedFonts,
    page: &PageGeometry,
    sheet: SheetFrame<'_>,
    stamp: PageStamp<'_>,
) {
    let ctx = PageContext::new(stamp.page_no, stamp.page_count);
    let blocks = [
        (
            overlay::parse_header_footer(sheet.header, ctx),
            overlay::header_band(page.width_pt(), page.height_pt(), stamp.margins),
            true,
        ),
        (
            overlay::parse_header_footer(sheet.footer, ctx),
            overlay::footer_band(page.width_pt(), stamp.margins),
            false,
        ),
    ];
    for (parts, band, is_header) in blocks {
        for (align, text) in [
            (Align::Left, &parts.left),
            (Align::Center, &parts.center),
            (Align::Right, &parts.right),
        ] {
            if text.is_empty() {
                continue;
            }
            draw_stamp_text(ops, registry, fonts, text, band, is_header, align);
        }
    }
}

/// Секция колонтитула: выравнивание по полосе и базовая линия кегля.
fn draw_stamp_text(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    fonts: &EmbeddedFonts,
    text: &str,
    band: overlay::Band,
    is_header: bool,
    align: Align,
) {
    let width_pt = stamp_text_width_pt(registry, text);
    let x = overlay::anchor_x(band, align, width_pt);
    let y = if is_header {
        overlay::header_baseline(band, STAMP_FONT_SIZE_PT)
    } else {
        overlay::footer_baseline(band, STAMP_FONT_SIZE_PT)
    };
    let font = fonts.id(Face::Regular).clone();
    ops.push(Op::SetFillColor {
        col: to_pdf_color(Color::BLACK),
    });
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point { x: Pt(x), y: Pt(y) },
    });
    ops.push(Op::SetFontSize {
        size: Pt(STAMP_FONT_SIZE_PT),
        font: font.clone(),
    });
    ops.push(Op::WriteText {
        items: vec![TextItem::Text(text.to_owned())],
        font,
    });
    ops.push(Op::EndTextSection);
}

/// Ширина строки колонтитула в точках — метриками того же regular-шрифта, что
/// рисуется.
///
/// У `overlay` метрик нет, и [`overlay::estimate_text_width`] оценивает ширину
/// средним глифом; painter считает точнее — от ширины зависит выравнивание
/// центральной и правой секций.
fn stamp_text_width_pt(registry: &mut FontRegistry, text: &str) -> f32 {
    let size_px = STAMP_FONT_SIZE_PT * PX_PER_POINT;
    measure_text(registry, DEFAULT_FONT_ID, size_px, text) / PX_PER_POINT
}

/// Водяной знак: текст под углом, по центру страницы.
///
/// Прозрачность — только через `/GS`: сплошная заливка `opacity` не выражает,
/// а ресурс зарегистрирован до начала записи ([`watermark_gs_id`]).
fn draw_watermark(
    ops: &mut Vec<Op>,
    fonts: &EmbeddedFonts,
    page: &PageGeometry,
    watermark: &Watermark,
) {
    let Some(draw) = overlay::watermark_draw(page.width_pt(), page.height_pt(), watermark) else {
        return;
    };
    let font = fonts.id(Face::Regular).clone();
    ops.push(Op::SaveGraphicsState);
    if overlay::needs_extgstate(draw.opacity) {
        ops.push(Op::LoadGraphicsState {
            gs: watermark_gs_id(),
        });
    }
    ops.push(Op::SetFillColor {
        col: to_pdf_color(Color::BLACK),
    });
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextMatrix {
        matrix: TextMatrix::Raw(draw.matrix),
    });
    ops.push(Op::SetFontSize {
        size: Pt(draw.font_size_pt),
        font: font.clone(),
    });
    ops.push(Op::WriteText {
        items: vec![TextItem::Text(draw.text)],
        font,
    });
    ops.push(Op::EndTextSection);
    ops.push(Op::RestoreGraphicsState);
}

/// Идентификатор `/GS` водяного знака в ресурсах документа.
fn watermark_gs_id() -> ExtendedGraphicsStateId {
    ExtendedGraphicsStateId(WATERMARK_GS.to_owned())
}

/// Диаграммы страницы — поверх остальных слоёв.
///
/// Диаграмма не делится по строкам, как ячейка: каждый якорь лежит на всех
/// страницах целиком, а видимую часть задаёт клип — пересечение якоря с
/// областью содержимого страницы. Так диаграмма на границе страниц режется
/// между ними, а не оставляет след в поле. По горизонтали клипа нет: лист по
/// столбцам не делится (Спринт 7), и ячейки в правое поле не режутся —
/// диаграмма следует тому же правилу.
fn draw_charts(
    ops: &mut Vec<Op>,
    registry: &mut FontRegistry,
    fonts: &EmbeddedFonts,
    page: &PageGeometry,
    sheet_page: &SheetPage<'_>,
    charts: &[chart::Layout],
) {
    if charts.is_empty() {
        return;
    }
    let (_, content_h_px) = page.content_px();
    let band_top = sheet_page.slice.offset_y;
    let band_bottom = band_top + content_h_px;
    for placed in charts {
        let rect = page_rect(placed.rect(), &sheet_page.slice);
        let clip_top = rect.y.max(band_top);
        let clip_bottom = (rect.y + rect.h).min(band_bottom);
        if clip_bottom <= clip_top {
            continue;
        }
        let clip = RectPx::new(rect.x, clip_top, rect.w, clip_bottom - clip_top);
        // Кегль и начертание диаграммы задаёт `layout` в пикселях, а в PDF
        // текст идёт regular: `ChartPrim::Text` начертаний не несёт.
        chart::draw(
            ops,
            registry,
            fonts.id(Face::Regular),
            page,
            rect,
            clip,
            placed,
        );
    }
}

/// Лист, подготовленный к записи: раскладка, геометрия страниц, их срезы и
/// разложенные по якорям диаграммы.
///
/// Срезы строятся один раз и живут до конца записи: пагинация — самая дорогая
/// часть подготовки, и второй проход по тому же листу ради неё не нужен. Плата
/// — память: у книги срезы всех её листов лежат в памяти одновременно (сама
/// книга и так разобрана целиком). Одиночный экспорт срезов не копит — у него
/// лист один.
struct PreparedSheet<'a> {
    sheet: &'a Sheet,
    layout: SheetLayout,
    page: PageGeometry,
    pagination: crate::pagination::SheetPagination<'a>,
    charts: Vec<chart::Layout>,
    /// Размещения картинок по страницам: по ним [`image::Registry::collect`]
    /// декодирует уникальные части до начала сессии.
    placements: Vec<Vec<image::Placement<'a>>>,
    /// Операции картинок по страницам; пусто, пока реестр не получил имена
    /// `XObject`'ов от сессии ([`image_ops`]).
    images: Vec<Vec<Op>>,
}

/// Поля страницы, объявленные листом (`<pageMargins>`), в терминах
/// [`PageConfig`]; `None` — лист их не объявляет.
fn sheet_margins(sheet: &Sheet) -> Option<Margins> {
    sheet.print.margins.map(|margins| Margins {
        top_mm: margins.top_mm,
        right_mm: margins.right_mm,
        bottom_mm: margins.bottom_mm,
        left_mm: margins.left_mm,
    })
}

/// Граница «уместить на N страниц» из листового `fitToWidth`/`fitToHeight`.
///
/// `0` по семантике Excel — «сколько потребуется», то есть границы нет;
/// отсутствующий атрибут разбор уже заменил дефолтом схемы (1).
fn fit_pages(pages: u32) -> Option<u32> {
    (pages != 0).then_some(pages)
}

/// Разрешить масштаб печати в конфиге листа: явные настройки вызывающего
/// сильнее листовых, листовые — сильнее умолчания.
///
/// Уровни сверху вниз, первое сработавшее решает:
/// 1. `fit_to_width`/`fit_to_height` вызывающего (`Some`) — как есть;
/// 2. `scale` вызывающего, если он отличается от умолчания 1.0: ручной
///    масштаб и «уместить» в Excel взаимоисключающие;
/// 3. `fitToPage` листа — его `fitToWidth`/`fitToHeight` (`0` — «сколько
///    потребуется», то есть границы нет);
/// 4. `scale` листа, если это не дефолтные 100%;
/// 5. умолчание — уместить по ширине: широкая таблица печатается одной
///    страницей по ширине, а не полосами столбцов; по высоте — сколько
///    потребуется.
///
/// Уровень 2 нужен ради явных вызывающих: интерфейс зовёт `exportPdf({})` и
/// полей не присылает вовсе, а CLI передаёт `--scale` явно.
// Сравнение с умолчанием точное: `1.0` — литерал `PageConfig::default`, а не
// результат вычислений.
#[allow(clippy::float_cmp)]
fn resolve_page_scale(page: &mut PageConfig, sheet: &Sheet) {
    let print = &sheet.print;
    if page.fit_to_width.is_some() || page.fit_to_height.is_some() {
        return;
    }
    if page.scale != 1.0 {
        return;
    }
    if print.fit_to_page {
        page.fit_to_width = fit_pages(print.fit_to_width);
        page.fit_to_height = fit_pages(print.fit_to_height);
        return;
    }
    if print.scale != 100 {
        // Масштаб — процент, счётчик много меньше 2^24: точности f32 хватает.
        #[allow(clippy::cast_precision_loss)]
        let scale = print.scale as f32 / 100.0;
        page.scale = scale;
        return;
    }
    page.fit_to_width = Some(1);
}

/// Подготовить лист к записи: раскладка, геометрия, срезы, диаграммы.
///
/// Масштаб считается на лист: `fit_to_width`/`fit_to_height` — настройка
/// листа, и у листов одной книги он выходит разным. Поля, ориентация и
/// масштаб — тоже свойства листа, и незаданные вызывающим подставляются из
/// книги (`<pageMargins>`, `<pageSetup>`), а если лист молчит — умолчанием
/// [`resolve_page_scale`]; остальные настройки страницы общие, поэтому
/// геометрия собирается из копии конфига с итоговым масштабом.
fn prepare_sheet<'a>(
    book: &'a Workbook,
    sheet_index: usize,
    options: &PdfOptions,
) -> Result<PreparedSheet<'a>, PdfError> {
    let sheet = book
        .sheets()
        .get(sheet_index)
        .ok_or(PdfError::NoSuchSheet(sheet_index))?;
    let layout = SheetLayout::new(sheet);
    let mut page_cfg = options.page.clone();
    // Поля лист несёт сам (`<pageMargins>`): приложение печатает лист с ними, и
    // без этой подстановки лист уезжает от эталона на разницу полей. Заданные
    // вызывающим поля сильнее — он печатает не «как Excel», а как ему сказали.
    page_cfg.margins = page_cfg.margins.or_else(|| sheet_margins(sheet));
    // Ориентация — тоже свойство листа (`<pageSetup orientation>`), и при
    // `Auto` (умолчание) она берётся из книги: без подстановки альбомный лист
    // печатался портретом. `Orientation::Default` листа (атрибута нет) — это
    // ориентация принтера по схеме; у PDF принтера нет, и Excel такой лист
    // печатает книжно. Явные Portrait/Landscape вызывающего сильнее
    // листовых — он печатает не «как Excel», а как ему сказали. Подстановка
    // до `print_scale`: подбор «уместить по ширине» считает ширину области
    // содержимого, а у альбомной страницы она другая.
    page_cfg.orientation = match page_cfg.orientation {
        PageOrientation::Auto => match sheet.print.orientation {
            Orientation::Landscape => PageOrientation::Landscape,
            Orientation::Portrait | Orientation::Default => PageOrientation::Portrait,
        },
        explicit => explicit,
    };
    // Масштаб — из настроек листа или умолчания, до `print_scale`: без этой
    // подстановки лист, который Excel печатает ужатым по ширине (`fitToPage`),
    // режется на страницы-полосы, а широкая таблица без настроек — и вовсе на
    // произведение полос столбцов и строк.
    resolve_page_scale(&mut page_cfg, sheet);
    // `fit_to_width` пересчитывает масштаб до сборки геометрии: от него зависят
    // и полосы, и координаты ячеек.
    page_cfg.scale = print_scale(&page_cfg, sheet, &layout);
    let page = PageGeometry::new(&page_cfg);
    // `page_cfg` несёт и масштаб, и повтор шапки/первых столбцов: пагинация
    // читает их одним конфигом, а геометрия — уже с итоговым масштабом.
    let pagination = paginate(book, sheet, &layout, &page, &page_cfg);
    let charts = chart_layouts(sheet, &layout);
    Ok(PreparedSheet {
        sheet,
        layout,
        page,
        pagination,
        charts,
        placements: Vec::new(),
        images: Vec::new(),
    })
}

/// Собрать один PDF из нескольких листов книги: листы идут подряд, каждый —
/// своими страницами, закладка ведёт на первую страницу листа.
///
/// `sheets` — индексы листов в порядке вывода; пустой срез — вся книга. Это и
/// есть «печать всей книги» Excel — один документ, а не файл на лист: закладки
/// на листы остаются навигацией по книге, а не по отдельным файлам.
/// Вызывающему, которому нужна вся книга, не приходится собирать список
/// индексов: пустой срез — обычный случай.
///
/// Индексы проверяются до первой записи: приёмник уже принял бы байты, и
/// ошибка на середине оставила бы обрезанный PDF.
///
/// Шрифты подрезаются один раз на документ ([`Session::add_font`]), поэтому
/// символы собираются со всех листов в общий набор: символ листа, не попавший в
/// зонды, вывелся бы нулевым глифом — молча.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если какого-то индекса нет в книге;
/// [`PdfError::NoSheets`], если в книге нет листов; [`PdfError::Font`], если
/// байты шрифта не разобрались; [`PdfError::Io`], если приёмник вернул
/// ошибку записи.
pub(crate) fn export_book_to<W: Write>(
    book: &Workbook,
    sheets: &[usize],
    options: &PdfOptions,
    out: &mut W,
) -> Result<(), PdfError> {
    let (mut prepared, title, mut registry) = prepare_book(book, sheets, options)?;
    write_book(&mut prepared, book, options, &mut registry, &title, out)
}

/// Подготовить листы книги: открыть листы, посчитать заголовок документа и
/// декодировать картинки в реестр.
///
/// Пустая книга отсекается здесь же: у PDF без страниц нет каталога страниц, и
/// файл невалиден.
fn prepare_book<'a>(
    book: &'a Workbook,
    sheets: &[usize],
    options: &PdfOptions,
) -> Result<(Vec<PreparedSheet<'a>>, String, image::Registry), PdfError> {
    let selection: Vec<usize> = if sheets.is_empty() {
        (0..book.sheets().len()).collect()
    } else {
        sheets.to_vec()
    };
    let mut prepared = selection
        .iter()
        .map(|&index| prepare_sheet(book, index, options))
        .collect::<Result<Vec<_>, _>>()?;
    prepared.first().ok_or(PdfError::NoSheets)?;

    // Заголовок документа: заданный пользователем, иначе — имя листа, если он
    // один (так же, как в одиночном экспорте). У книги своего имени нет:
    // подставить имя первого листа значило бы выдать часть за целое.
    let title = if options.title.is_empty() {
        if let [only] = prepared.as_slice() {
            only.sheet.meta.name.clone()
        } else {
            String::new()
        }
    } else {
        options.title.clone()
    };

    // Картинки декодируются с дедупликацией по байтам до начала сессии:
    // `XObject`'ы — глобальные ресурсы, и добавить их после первой страницы
    // нельзя. Имена им раздаст сессия ([`Session::add_image`]) в порядке
    // регистрации.
    let mut registry = image::Registry::default();
    for sheet in &mut prepared {
        sheet.placements = image_placements(
            book,
            sheet.sheet,
            &sheet.layout,
            &sheet.page,
            &sheet.pagination.pages,
        );
        for page in &sheet.placements {
            registry.collect(page);
        }
    }
    Ok((prepared, title, registry))
}

/// Зарегистрировать глобальные ресурсы в сессии, собрать зонды глифов и
/// записать страницы; хвост (закладки, каталог, xref) допишет
/// [`Session::finish`] — приёмнику не нужен `seek`.
///
/// Шрифты подрезаются один раз на документ ([`Session::add_font`]), поэтому
/// символы собираются со всех листов в общий набор: символ листа, не попавший в
/// зонды, вывелся бы нулевым глифом — молча. Нумерация зондов та же, что у
/// записи: `&P`/`&N` в колонтитулах обязаны совпасть.
// Регистрация шрифтов, картинок и ExtGState — один проход «до первой страницы»,
// выносить его в функции значило бы разрывать порядок регистрации.
#[allow(clippy::too_many_lines)]
fn write_book<W: Write>(
    prepared: &mut [PreparedSheet<'_>],
    book: &Workbook,
    options: &PdfOptions,
    registry: &mut image::Registry,
    title: &str,
    out: &mut W,
) -> Result<(), PdfError> {
    // Начертания книги: ими ограничивается набор встраиваемых шрифтов.
    let mut faces = fonts::FaceSet::default();
    for sheet in prepared.iter() {
        faces.merge(sheet.pagination.faces);
    }
    // Имена шрифтов («F0», «F1», …) строятся до сессии, но тем же порядком
    // начертаний, каким их зарегистрирует [`Session::add_font`]: зондовый
    // проход обязан ссылаться на шрифты, которых ещё нет.
    let embedded = EmbeddedFonts::from_faces(faces);

    let page_count: usize = prepared
        .iter()
        .map(|sheet| sheet.pagination.pages.len())
        .sum();
    let mut metric_registry = FontRegistry::new(FONT_CACHE);
    let mut glyphs: BTreeMap<Face, BTreeSet<char>> = BTreeMap::new();
    // Нумерация зондов та же, что у записи: `&P`/`&N` в колонтитулах обязаны
    // совпасть, иначе подрезанный шрифт не покроет символы реальной страницы.
    let mut first_page_no = 1;
    for sheet in prepared.iter() {
        let (header, footer) = sheet_overlay(options, sheet.sheet);
        collect_glyphs(
            &mut glyphs,
            SheetFrame {
                book,
                layout: &sheet.layout,
                header,
                footer,
            },
            &sheet.pagination.pages,
            &sheet.page,
            &embedded,
            &mut metric_registry,
            &sheet.charts,
            options,
            first_page_no,
            page_count,
        );
        first_page_no += sheet.pagination.pages.len();
    }

    let mut session = Session::begin(out, page_count, options.compress)?;
    session.set_title(title);

    // Шрифты — в фиксированном порядке начертаний, регуляр всегда: без него
    // текст с начертанием вне набора вывелся бы нулевым глифом.
    for face in [Face::Regular, Face::Bold, Face::Italic, Face::BoldItalic] {
        if face != Face::Regular && !faces.contains(face) {
            continue;
        }
        let used = glyphs.get(&face).cloned().unwrap_or_default();
        let name = session.add_font(face.bytes(), &used)?;
        debug_assert_eq!(
            name,
            embedded.id(face).0,
            "сессия раздала имя шрифта не в том порядке, что EmbeddedFonts"
        );
    }

    // Прозрачность водяного знака — только через extended graphics state:
    // ресурс обязан лечь в документ до первой страницы, имя фиксировано
    // константой, а операции знака (в том числе в зондах) на него ссылаются.
    if let Some(watermark) = options.overlay.watermark.as_ref() {
        if overlay::needs_extgstate(watermark.opacity) && !watermark.text.trim().is_empty() {
            let alpha = overlay::clamp_opacity(watermark.opacity);
            let state = ExtendedGraphicsState {
                current_stroke_alpha: alpha,
                current_fill_alpha: alpha,
            };
            let mut dict = Dictionary::new();
            dict.set("Type", "ExtGState");
            dict.set("CA", state.current_stroke_alpha);
            dict.set("ca", state.current_fill_alpha);
            session.add_graphics_state(WATERMARK_GS, dict)?;
        }
    }

    // Картинки — по той же причине: `Op::UseXobject` переводится по именам
    // `XObject`'ов, и объекты должны лежать в ресурсах до первой страницы.
    // Имена собираются отдельным проходом: `add_image` занимает сессию, а
    // `registry.decoded` — реестр, и `assign` требует реестр изменяемым.
    let names: Vec<String> = registry
        .decoded()
        .iter()
        .map(|decoded| session.add_image(decoded))
        .collect::<Result<_, _>>()?;
    for (index, name) in names.into_iter().enumerate() {
        registry.assign(index, XObjectId(name));
    }
    for sheet in prepared.iter_mut() {
        sheet.images = image_ops(registry, &sheet.placements);
    }

    // Закладки — на лист, и ведут на его первую страницу. Копятся до `finish`:
    // сессия пишет `/Outlines` в хвосте, где каталог ещё не записан.
    if options.bookmarks {
        let spans: Vec<annot::SheetSpan<'_>> = prepared
            .iter()
            .map(|sheet| annot::SheetSpan {
                name: &sheet.sheet.meta.name,
                pages: sheet.pagination.pages.len(),
            })
            .collect();
        let mut page = 0;
        let entries: Vec<(String, usize)> = spans
            .iter()
            .map(|span| {
                let entry = (span.name.to_owned(), page);
                page += span.pages;
                entry
            })
            .collect();
        session.set_outline(&entries)?;
    }

    write_pages(
        &mut session,
        prepared,
        book,
        &embedded,
        &mut metric_registry,
        options,
    )?;
    session.finish()?;
    Ok(())
}

/// Записать страницы подготовленных листов в порядке вывода: лист за листом,
/// страница за страницей.
///
/// `#[allow]`: семь параметров — это состояние записи (сессия, шрифты, кэш
/// метрик), книга и настройки; связывать их структурой значило бы прятать
/// порядок обхода листов.
#[allow(clippy::too_many_arguments)]
fn write_pages<W: Write>(
    session: &mut Session<'_, W>,
    prepared: &[PreparedSheet<'_>],
    book: &Workbook,
    fonts: &EmbeddedFonts,
    registry: &mut FontRegistry,
    options: &PdfOptions,
) -> Result<(), PdfError> {
    let page_count: usize = prepared
        .iter()
        .map(|sheet| sheet.pagination.pages.len())
        .sum();
    let base = StampBase::from_options(options);
    // Нумерация сквозная по документу: `&P` считает страницы книги целиком, а
    // не листа — так же нумерует их Excel при печати всей книги.
    let mut page_no = 0;
    for sheet in prepared {
        let (header, footer) = sheet_overlay(options, sheet.sheet);
        let frame = SheetFrame {
            book,
            layout: &sheet.layout,
            header,
            footer,
        };
        // Примечания ищут свою страницу один раз на лист: `page_annotations`
        // зовётся на каждой странице, а поиск перебирает все страницы листа —
        // внутри цикла это было бы квадратом по их числу.
        let comment_placements =
            annot::comment_placements(sheet.sheet, &sheet.layout, &sheet.pagination.pages);
        for (page_index, sheet_page) in sheet.pagination.pages.iter().enumerate() {
            page_no += 1;
            let page_geom = sheet.page.with_page_top_px(sheet_page.slice.offset_y);
            // `image_ops` строит по элементу на страницу; пустой срез —
            // страховка на случай расхождения, а не рабочий режим.
            let images = sheet.images.get(page_index).map_or(&[][..], Vec::as_slice);
            let mut ops: Vec<Op> = Vec::new();
            draw_page(
                &mut ops,
                registry,
                fonts,
                &page_geom,
                frame,
                sheet_page,
                &sheet.charts,
                images,
                options.page.print_grid_lines,
                base.page(page_no, page_count),
            );
            ops.extend(annot::page_annotations(
                sheet.sheet,
                &sheet.layout,
                &sheet.pagination.pages,
                &comment_placements,
                &page_geom,
                &sheet_page.slice,
                page_index,
            ));
            // Аннотации в content stream не пишутся — они уходят словарями в
            // `/Annots` страницы; `page_ref` разрешает `/Dest` внутренних
            // ссылок в id зарезервированных сессией страниц.
            let annots: Vec<Dictionary> = ops
                .iter()
                .filter_map(|op| match op {
                    Op::Annotation { annot } => Some(annot::annotation_to_dict(annot, &|index| {
                        session.page_ref(index)
                    })),
                    _ => None,
                })
                .collect();
            ops.retain(|op| !matches!(op, Op::Annotation { .. }));
            // Карта шрифтов живёт только до `translate`: `write_page` требует
            // сессию целиком, и заимствование `prepared_fonts` обязано кончиться.
            let content = {
                let fonts_map: BTreeMap<FontId, &PreparedFont> = session
                    .prepared_fonts()
                    .iter()
                    .map(|(name, prepared)| (FontId(name.clone()), prepared))
                    .collect();
                content::translate(&ops, &fonts_map)?
            };
            session.write_page(
                &content,
                [0.0, 0.0, page_geom.width_pt(), page_geom.height_pt()],
                &annots,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PageConfig;

    /// Срез страницы без полос и повторов: важны только сдвиги.
    fn plain_slice() -> PageSlice {
        PageSlice {
            rows: 0..0,
            cols: 0..0,
            header_rows: 0..0,
            repeat_cols: 0..0,
            offset_y: 0.0,
            offset_x: 0.0,
            center_x_px: 0.0,
            center_y_px: 0.0,
        }
    }

    /// Раскладка листа из общей фикстуры: тесты сетки смотрят на её рёбра.
    fn layout() -> SheetLayout {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/xlsx/content-mixed-types.xlsx");
        let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        let book = doc_converter_xlsx::open(bytes).expect("книга открывается");
        let sheet = book.sheets().first().expect("лист есть");
        SheetLayout::new(sheet)
    }

    /// Отрезки из операций страницы: концы в точках PDF.
    fn segments(ops: &[Op]) -> Vec<((f32, f32), (f32, f32))> {
        ops.iter()
            .filter_map(|op| match op {
                Op::DrawLine { line } => {
                    let [from, to] = line.points.as_slice() else {
                        return None;
                    };
                    Some(((from.p.x.0, from.p.y.0), (to.p.x.0, to.p.y.0)))
                }
                _ => None,
            })
            .collect()
    }

    /// Шапка, повторяемые столбцы и полоса потока сходятся на общих границах:
    /// сетка page идёт по рёбрам раскладки без удвоенных линий.
    ///
    /// Срез — второй страницы листа с двумя строками шапки и первым столбцом
    /// в повторе: блоки стоят вплотную (шапка у верхнего края, повтор — у
    /// левого), поэтому рёбра на их стыке должны схлопнуться в одну линию.
    #[test]
    fn grid_merges_header_flow_and_repeat_edges() {
        let layout = layout();
        let page = PageGeometry::new(&PageConfig::default());
        let mut slice = plain_slice();
        slice.header_rows = 0..2;
        slice.repeat_cols = 0..1;
        slice.rows = 10..12;
        slice.cols = 1..3;
        slice.offset_y = layout.row_y(10) - layout.row_y(2);
        slice.offset_x = layout.column_x(1) - layout.column_x(1);

        let mut ops = Vec::new();
        draw_grid(&mut ops, &page, &layout, &slice);
        let segments = segments(&ops);
        let horizontal: Vec<_> = segments
            .iter()
            .filter(|(from, to)| (from.1 - to.1).abs() < COORD_EPS)
            .collect();
        let vertical: Vec<_> = segments
            .iter()
            .filter(|(from, to)| (from.0 - to.0).abs() < COORD_EPS)
            .collect();

        // Рёбра: строки шапки 0..2 и плотно примыкающие к ним строки потока,
        // столбцы повтора 0..1 и столбцы полосы 1..3.
        let rows = [0, 1, 2, 3, 4].map(|row| layout.row_y(row));
        assert_eq!(horizontal.len(), rows.len(), "горизонтальных линий");
        let cols = [0, 1, 2, 3].map(|col| layout.column_x(col));
        assert_eq!(vertical.len(), cols.len(), "вертикальных линий");

        let y_pt = |px: f32| page.height_pt() - (page.origin_y_pt() + page.px_to_pt(px));
        let x_pt = |px: f32| page.origin_x_pt() + page.px_to_pt(px);
        // Сравниваются множества координат: у PDF начало отсчёта внизу,
        // и порядок обхода строк в точках обратен порядку строк листа.
        let mut actual: Vec<f32> = horizontal.iter().map(|(from, _)| from.1).collect();
        let mut expected: Vec<f32> = rows.iter().map(|&row| y_pt(row)).collect();
        actual.sort_by(f32::total_cmp);
        expected.sort_by(f32::total_cmp);
        for (actual, expected) in actual.iter().zip(&expected) {
            assert!(
                (actual - expected).abs() < 1e-3,
                "горизонталь {actual} pt против ребра {expected} pt"
            );
        }
        let mut actual: Vec<f32> = vertical.iter().map(|(from, _)| from.0).collect();
        let mut expected: Vec<f32> = cols.iter().map(|&col| x_pt(col)).collect();
        actual.sort_by(f32::total_cmp);
        expected.sort_by(f32::total_cmp);
        for (actual, expected) in actual.iter().zip(&expected) {
            assert!(
                (actual - expected).abs() < 1e-3,
                "вертикаль {actual} pt против ребра {expected} pt"
            );
        }

        // Линии дотягиваются до краёв сетки: последнее ребро строк (столбцов)
        // замыкает область печати.
        for (from, to) in &horizontal {
            assert!((from.0 - x_pt(cols[0])).abs() < 1e-3 && (to.0 - x_pt(cols[3])).abs() < 1e-3);
        }
        for (from, to) in &vertical {
            assert!((from.1 - y_pt(rows[0])).abs() < 1e-3 && (to.1 - y_pt(rows[4])).abs() < 1e-3);
        }
    }

    /// Лист без данных: пустой срез описывает страницу без области печати, и
    /// сетка не рисует даже вырожденных линий.
    #[test]
    fn empty_slice_draws_no_grid() {
        let mut ops = Vec::new();
        draw_grid(
            &mut ops,
            &PageGeometry::new(&PageConfig::default()),
            &layout(),
            &plain_slice(),
        );
        assert!(segments(&ops).is_empty(), "на пустом срезе линий нет");
    }

    /// Центрирование сдвигает прямоугольник в точках: 96 px раскладки при 100%
    /// — это 72 pt, значит половинный сдвиг в 48 px — 36 pt.
    #[test]
    fn centering_shifts_cell_rect_in_points() {
        let page = PageGeometry::new(&PageConfig::default());
        let rect = RectPx::new(0.0, 0.0, 96.0, 96.0);
        let plain = page.rect_to_pt(page_rect(rect, &plain_slice()));

        let mut slice = plain_slice();
        slice.center_x_px = 48.0;
        slice.center_y_px = 48.0;
        let centered = page.rect_to_pt(page_rect(rect, &slice));

        let dx = centered.x - plain.x;
        let dy = centered.y - plain.y;
        assert!((dx - 36.0).abs() < 1e-3, "сдвиг по x: {dx} pt");
        assert!((dy - 36.0).abs() < 1e-3, "сдвиг по y: {dy} pt");
    }

    /// Срез вычитается до центрирования: сдвиг возвращает ячейку к левому краю
    /// полосы, и лишь потом она уезжает к середине страницы.
    #[test]
    fn slice_offset_is_subtracted_before_centering() {
        let mut slice = plain_slice();
        slice.offset_x = 40.0;
        slice.center_x_px = 25.0;
        slice.center_y_px = 5.0;

        let shifted = page_rect(RectPx::new(100.0, 200.0, 30.0, 10.0), &slice);
        assert_eq!(shifted, RectPx::new(85.0, 205.0, 30.0, 10.0));
    }
}
