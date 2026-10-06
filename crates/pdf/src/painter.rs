//! Сборка страниц PDF: обход ячеек листа и запись операций printpdf.
//!
//! Порядок слоёв повторяет оба «экранных» пути (Excel и `xlsx::paint`):
//! сначала заливки, поверх них рамки, поверх всего текст. Поэтому обход листа
//! идёт тремя проходами по заранее собранному списку ячеек, а не одним.
//!
//! Лист режется на страницы заранее ([`crate::pagination`]): сюда приходит
//! срез страницы и её ячейки, отсюда — только геометрия и операции.
//!
//! Диаграммы листа рисуются последним слоем — над заливками, рамками и
//! текстом, как в `xlsx::paint` (картинки там лежат между текстом и
//! диаграммами; в PDF они ещё не перенесены и встанут туда же).
//!
//! Ещё не перенесены настройки [`PdfOptions`], за которыми стоит заметная
//! работа: закреплённые области и картинки — следующие срезы спринта.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use doc_converter_render::display_list::Color;
use doc_converter_render::font::FontRegistry;
use doc_converter_xlsx::layout::{SheetLayout, PX_PER_POINT};
use doc_converter_xlsx::paint::{anchor_rect, display_text};
use doc_converter_xlsx::{CellValue, Sheet, Workbook};
use printpdf::streaming::StreamSession;
use printpdf::{
    FontId, Line, LinePoint, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg, Point, Pt,
    TextItem,
};

use crate::fonts::{EmbeddedFonts, Face};
use crate::layout::{PageGeometry, RectPx};
use crate::options::PdfOptions;
use crate::pagination::{paginate, print_scale, PageSlice, PaintedCell, SheetPage};
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

/// Открыть лист книги и собрать PDF всеми его страницами.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если индекса нет в книге; [`PdfError::Font`],
/// если printpdf не принял байты шрифта; [`PdfError::Io`], если приёмник
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
/// закладками. Страница живёт только до [`StreamSession::write_page`]:
/// содержимое освобождается сразу после записи, поэтому пик не растёт с числом
/// страниц. Хвост (закладки, каталог, xref) дописывается в конце — приёмнику
/// не нужен `seek`.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если индекса нет в книге; [`PdfError::Font`],
/// если printpdf не принял байты шрифта; [`PdfError::Io`], если приёмник
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
/// Зонды нужны потому, что `StreamSession::begin` подрезает шрифты до записи
/// первой страницы (`prepare_fonts`), а подрезка перенумеровывает глифы:
/// символ, не попавший в зонды, выведется нулевым глифом — молча. Поэтому
/// символы собираются до записи, и источник у них один — та же [`draw_page`],
/// что пойдёт в PDF: проход зондов рисует страницы и выбрасывает операции,
/// оставляя только символы. Вариант «пре-скан текста» стоил бы почти столько
/// же (перенос и клиппинг чисел всё равно считает `break_lines`), но
/// разъехался бы с `draw_page` при первой правке клиппинга или переноса;
/// вариант «полный шрифт» в форке недоступен: `PdfSaveOptions::subset_fonts`
/// нигде не читается (`grep subset_fonts vendor/printpdf/src` — определение
/// поля и дефолт), а полный `DejaVuSans` весит ~760 `КиБ` на начертание
/// против бюджета `content-dense` в 32 `КиБ`.
///
/// Замер двойной отрисовки (release, macOS, `scale-500-pages.xlsx` — 480
/// страниц): проход зондов 35 мс, экспорт целиком 162 мс против 152 мс до
/// правки (+7 %): вторая отрисовка с прогретым кэшем метрик дешевле, чем та
/// экономия, которую даёт стриминговая запись против `doc.save`. Полные числа
/// 50/100/500 — в `docs/sprint-7/streaming-report.md` (B3).
///
/// Диаграммы зондируются наравне с текстом: их подписи — такие же символы,
/// которых в ячейках может не быть, и без них подрезка оставила бы у
/// диаграммы нулевые глифы. Раскладка при этом не считается второй раз — она
/// приходит готовой ([`chart_layouts`]).
///
/// Набор — параметр, а не результат: книга собирает его со всех своих листов
/// (шрифты подрезаются один раз на документ), поэтому лист добавляет в общий
/// набор свои символы, а не заводит собственный.
#[allow(clippy::too_many_arguments)] // Как у `draw_page`: связывать их структурой — прятать порядок слоёв.
fn collect_glyphs(
    glyphs: &mut BTreeMap<FontId, BTreeSet<char>>,
    sheet: SheetFrame<'_>,
    pages: &[SheetPage<'_>],
    geometry: &PageGeometry,
    fonts: &EmbeddedFonts,
    registry: &mut FontRegistry,
    charts: &[chart::Layout],
    print_grid_lines: bool,
) {
    for sheet_page in pages {
        let page_geom = geometry.with_page_top_px(sheet_page.slice.offset_y);
        let mut ops: Vec<Op> = Vec::new();
        draw_page(
            &mut ops,
            registry,
            fonts,
            &page_geom,
            sheet,
            sheet_page,
            charts,
            print_grid_lines,
        );
        for op in &ops {
            if let Op::WriteText { font, items, .. } = op {
                let set = glyphs.entry(font.clone()).or_default();
                for item in items {
                    if let TextItem::Text(text) = item {
                        set.extend(text.chars());
                    }
                }
            }
        }
    }
}

/// Кадры-зонды из собранных символов: по одному на начертание, с одной
/// текстовой операцией из всех его символов.
///
/// Размер страницы-зонда printpdf не читает — берётся с листа-донора, чтобы
/// зонд не отличался от страниц документа даже габаритами.
fn probe_pages_from(
    glyphs: BTreeMap<FontId, BTreeSet<char>>,
    geometry: &PageGeometry,
) -> Vec<PdfPage> {
    glyphs
        .into_iter()
        .map(|(font, chars)| {
            PdfPage::new(
                Mm::from(Pt(geometry.width_pt())),
                Mm::from(Pt(geometry.height_pt())),
                vec![Op::WriteText {
                    items: vec![TextItem::Text(chars.into_iter().collect())],
                    font,
                }],
            )
        })
        .collect()
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

/// Книга и раскладка её листа: по ним идёт обход ячеек и строится сетка, а от
/// страницы к странице они не меняются.
#[derive(Clone, Copy)]
struct SheetFrame<'a> {
    book: &'a Workbook,
    layout: &'a SheetLayout,
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
    print_grid_lines: bool,
) {
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
    draw_charts(ops, registry, fonts, page, sheet_page, charts);
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
}

/// Подготовить лист к записи: раскладка, геометрия, срезы, диаграммы.
///
/// Масштаб считается на лист: `fit_to_width`/`fit_to_height` — настройка
/// листа, и у листов одной книги он выходит разным. Остальные настройки
/// страницы общие, поэтому геометрия собирается из копии конфига с итоговым
/// масштабом.
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
    // `fit_to_width` пересчитывает масштаб до сборки геометрии: от него зависят
    // и полосы, и координаты ячеек.
    let mut page_cfg = options.page.clone();
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
/// Шрифты подрезаются один раз на документ ([`StreamSession::begin`]), поэтому
/// символы собираются со всех листов в общий набор: символ листа, не попавший в
/// зонды, вывелся бы нулевым глифом — молча.
///
/// # Errors
/// [`PdfError::NoSuchSheet`], если какого-то индекса нет в книге;
/// [`PdfError::NoSheets`], если в книге нет листов; [`PdfError::Font`], если
/// printpdf не принял байты шрифта; [`PdfError::Io`], если приёмник вернул
/// ошибку записи.
pub(crate) fn export_book_to<W: Write>(
    book: &Workbook,
    sheets: &[usize],
    options: &PdfOptions,
    out: &mut W,
) -> Result<(), PdfError> {
    let selection: Vec<usize> = if sheets.is_empty() {
        (0..book.sheets().len()).collect()
    } else {
        sheets.to_vec()
    };
    let prepared = selection
        .iter()
        .map(|&index| prepare_sheet(book, index, options))
        .collect::<Result<Vec<_>, _>>()?;
    // Пустая книга — не «все её листы, которых нет»: у PDF без страниц нет
    // каталога страниц, и файл невалиден.
    let first = prepared.first().ok_or(PdfError::NoSheets)?;

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
    let mut doc = PdfDocument::new(&title);
    doc.metadata.info.author.clone_from(&options.author);
    doc.metadata.info.subject.clone_from(&options.subject);
    doc.metadata.info.keywords.clone_from(&options.keywords);

    let mut warnings: Vec<PdfWarnMsg> = Vec::new();
    // Закладка — на лист, и ведёт на его первую страницу. `pages` — длина
    // листа: по ней `add_outline` отсчитывает начало следующего. Записывается
    // до страниц: printpdf резервирует их id заранее, и закладка обязана
    // попасть в тот же документ.
    let spans: Vec<annot::SheetSpan<'_>> = prepared
        .iter()
        .map(|sheet| annot::SheetSpan {
            name: &sheet.sheet.meta.name,
            pages: sheet.pagination.pages.len(),
        })
        .collect();
    annot::add_outline(&mut doc, &spans, options.bookmarks);

    let mut faces = fonts::FaceSet::default();
    for sheet in &prepared {
        faces.merge(sheet.pagination.faces);
    }
    let embedded = fonts::embed(&mut doc, &mut warnings, faces)?;

    let mut registry = FontRegistry::new(FONT_CACHE);
    let mut glyphs: BTreeMap<FontId, BTreeSet<char>> = BTreeMap::new();
    for sheet in &prepared {
        collect_glyphs(
            &mut glyphs,
            SheetFrame {
                book,
                layout: &sheet.layout,
            },
            &sheet.pagination.pages,
            &sheet.page,
            &embedded,
            &mut registry,
            &sheet.charts,
            options.page.print_grid_lines,
        );
    }
    let probe = probe_pages_from(glyphs, &first.page);
    let save = PdfSaveOptions {
        // `optimize` у printpdf сжимает потоки — это и есть `compress`.
        optimize: options.compress,
        ..PdfSaveOptions::default()
    };
    let page_count: usize = prepared
        .iter()
        .map(|sheet| sheet.pagination.pages.len())
        .sum();
    let mut session = StreamSession::begin(
        &doc,
        &probe,
        BTreeSet::new(),
        page_count,
        &save,
        out,
        &mut warnings,
    )?;

    write_pages(
        &mut session,
        &prepared,
        book,
        &embedded,
        &mut registry,
        options,
        &mut warnings,
    )?;
    session.finish()?;
    Ok(())
}

/// Записать страницы подготовленных листов в порядке вывода: лист за листом,
/// страница за страницей.
///
/// `#[allow]`: семь параметров — это состояние записи (сессия, шрифты, кэш
/// метрик, предупреждения), книга и настройки; связывать их структурой значило
/// бы прятать порядок обхода листов.
#[allow(clippy::too_many_arguments)]
fn write_pages<W: Write>(
    session: &mut StreamSession<'_, '_, W>,
    prepared: &[PreparedSheet<'_>],
    book: &Workbook,
    fonts: &EmbeddedFonts,
    registry: &mut FontRegistry,
    options: &PdfOptions,
    warnings: &mut Vec<PdfWarnMsg>,
) -> Result<(), PdfError> {
    for sheet in prepared {
        let frame = SheetFrame {
            book,
            layout: &sheet.layout,
        };
        for sheet_page in &sheet.pagination.pages {
            let page_geom = sheet.page.with_page_top_px(sheet_page.slice.offset_y);
            let mut ops: Vec<Op> = Vec::new();
            draw_page(
                &mut ops,
                registry,
                fonts,
                &page_geom,
                frame,
                sheet_page,
                &sheet.charts,
                options.page.print_grid_lines,
            );
            ops.extend(annot::page_annotations(
                sheet.sheet,
                &sheet.layout,
                &sheet.pagination.pages,
                &page_geom,
                &sheet_page.slice,
            ));
            let pdf_page = PdfPage::new(
                Mm::from(Pt(page_geom.width_pt())),
                Mm::from(Pt(page_geom.height_pt())),
                ops,
            );
            session.write_page(&pdf_page, warnings)?;
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
        // Сравниваются множества координат: у printpdf начало отсчёта внизу,
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
