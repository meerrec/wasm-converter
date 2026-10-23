//! PDF-экспорт DOCX: страницы готовой раскладки в точки PDF.
//!
//! Раскладка приходит целиком от `doc-converter-docx`
//! ([`doc_converter_docx::layout::layout_document`]): движок уже разбил абзацы
//! на строки и по страницам, а координаты отдал в пикселях при 96 dpi от
//! левого верхнего угла страницы. Здесь они переводятся в точки той же
//! геометрией, что и лист XLSX ([`PageGeometry`]): вторая конвенция пересчёта
//! разошлась бы с первой на первой же правке.
//!
//! Шрифт один — Carlito из `doc-converter-render`, тот же, которым раскладка
//! мерила текст, поэтому точки переноса в PDF совпадают с canvas-путём по
//! построению (ADR-0005). По той же причине не используется
//! `LayoutItem::Paragraph::font_family`: реестр знает одну гарнитуру, а рисовать
//! строку другой значило бы разойтись с её измеренной шириной — так же решает
//! и canvas-путь (`doc_converter_docx::render`).
//!
//! Подчёркивание в PDF пока не рисуется: бэкенд линию сам не ставит, а
//! отдельный срез для неё ещё не сделан — ровно как у ячеек XLSX
//! (`crate::styles::CellStyle::underline`).

mod floats;
mod images;
mod tables;

use std::collections::{BTreeMap, BTreeSet};

use doc_converter_docx::layout::{LayoutItem, Page, PageChrome, PageLayout, Rect};
use doc_converter_docx::{Document, HalfPoint, LayoutOptions};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::{default_font_bytes, FontRegistry, DEFAULT_FONT_ID};
use lopdf::Dictionary;
use skrifa::raw::TableProvider;

use crate::fonts::{EmbeddedFonts, Face, FaceSet};
use crate::layout::PageGeometry;
use crate::options::{Margins, PageConfig, PageSize, PdfOptions};
use crate::pdf::content;
use crate::pdf::fonts::PreparedFont;
use crate::pdf::ops::{FontId, Op, Point, Pt, TextItem, XObjectId};
use crate::pdf::session::Session;
use crate::styles::to_pdf_color;
use crate::PdfError;

/// Размер LRU-кэша метрик: тот же, что у XLSX-пути (`painter::FONT_CACHE`).
const FONT_CACHE: usize = 4096;

/// Миллиметров в пикселе раскладки: 96 dpi против 25.4 мм в дюйме.
///
/// Единственный вход [`PageGeometry`] для произвольной страницы —
/// [`PageSize::Custom`] в миллиметрах, поэтому размер раскладки переводится
/// сперва в миллиметры; обратно в точки его вернёт сама геометрия
/// (`PT_PER_MM`), и `Page.width` дойдёт до PDF без второй константы.
const MM_PER_PX: f32 = 25.4 / 96.0;

/// Собрать PDF из всех страниц документа.
///
/// # Errors
/// [`PdfError::DocxLayout`], если раскладка не собралась; [`PdfError::NoPages`],
/// если она не дала ни одной страницы; [`PdfError::Font`], если байты шрифта не
/// разобрались; [`PdfError::Image`], если сессия не приняла картинку;
/// [`PdfError::Io`], если приёмник вернул ошибку записи.
pub(crate) fn export(doc: &Document, options: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    let mut registry = FontRegistry::new(FONT_CACHE);
    let layout =
        doc_converter_docx::layout::layout_document(doc, &LayoutOptions::default(), &mut registry)
            .map_err(PdfError::DocxLayout)?;
    // PDF без страниц невалиден: у него нет каталога страниц.
    layout.pages.first().ok_or(PdfError::NoPages)?;

    let faces = collect_faces(&layout);
    // Имена шрифтов («F0», «F1», …) строятся до сессии, но тем же порядком
    // начертаний, каким их зарегистрирует [`Session::add_font`]: зондовый
    // проход обязан ссылаться на шрифты, которых ещё нет.
    let embedded = EmbeddedFonts::from_faces(faces);
    let mut images = images::Images::collect(doc);

    // Шрифты подрезаются до записи первой страницы (`Session::add_font`), а
    // подрезка перенумеровывает глифы: символ, не попавший в зонды, вывелся бы
    // нулевым глифом — молча. Поэтому символы собираются первой отрисовкой, той
    // же `draw_page`, что пойдёт в PDF, — так же, как у листа XLSX
    // (`painter::collect_glyphs`).
    let glyphs = collect_glyphs(&layout, &embedded, &mut registry, &images);

    let mut out = Vec::new();
    let mut session = Session::begin(&mut out, layout.pages.len(), options.compress)?;
    session.set_title(&title(doc, options));
    apply_metadata(&mut session, doc, options);

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

    // Картинки — по той же причине: `Op::UseXobject` переводится по именам
    // `XObject`'ов, и объекты обязаны лежать в ресурсах до первой страницы.
    // Имена собираются отдельным проходом: `add_image` занимает сессию, а
    // `decoded` — реестр, и `assign` требует реестр изменяемым.
    let names: Vec<String> = images
        .decoded()
        .iter()
        .map(|decoded| session.add_image(decoded))
        .collect::<Result<_, _>>()?;
    for (index, name) in names.into_iter().enumerate() {
        images.assign(index, XObjectId(name));
    }

    for (index, page) in layout.pages.iter().enumerate() {
        let geometry = geometry(page);
        let mut ops = draw_page(
            page,
            layout.chrome.get(index),
            &geometry,
            &embedded,
            &mut registry,
            &images,
        );
        // Аннотации в content stream не пишутся — они уходят словарями в
        // `/Annots` страницы; `page_ref` разрешает `/Dest` внутренних ссылок
        // в id зарезервированных сессией страниц.
        let annots: Vec<Dictionary> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Annotation { annot } => Some(crate::annot::annotation_to_dict(annot, &|i| {
                    session.page_ref(i)
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
            [0.0, 0.0, geometry.width_pt(), geometry.height_pt()],
            &annots,
        )?;
    }
    session.finish()?;
    Ok(out)
}

/// Заголовок документа: заданный пользователем перебивает книжный.
fn title(doc: &Document, options: &PdfOptions) -> String {
    if !options.title.is_empty() {
        return options.title.clone();
    }
    doc.metadata.title.clone().unwrap_or_default()
}

/// Подставить свойства пакета `docProps/core.xml` в `/Info`, если настройки не
/// заданы.
///
/// Ключевые слова — одна строка через запятую: в модели это список, а
/// `/Keywords` — строка (так же соединял их вендоренный бэкенд).
fn apply_metadata(session: &mut Session<'_, Vec<u8>>, source: &Document, options: &PdfOptions) {
    session.set_info(&[
        (
            String::from("Author"),
            pick(&options.author, source.metadata.creator.as_deref()),
        ),
        (
            String::from("Subject"),
            pick(&options.subject, source.metadata.subject.as_deref()),
        ),
        (String::from("Keywords"), options.keywords.join(",")),
    ]);
}

/// Непустая настройка, иначе значение из документа.
fn pick(option: &str, fallback: Option<&str>) -> String {
    if option.is_empty() {
        fallback.unwrap_or_default().to_owned()
    } else {
        option.to_owned()
    }
}

/// Геометрия страницы: раскладка DOCX уже в пикселях 96 dpi, а координаты
/// элементов — от левого верхнего угла самой страницы.
///
/// Поэтому поля нулевые: полосу набора движок учёл сам, и геометрии остаётся
/// только пересчёт единиц. Ориентация не задаётся отдельно — раскладка уже
/// отдала повернутые `width`/`height` секции.
fn geometry(page: &Page) -> PageGeometry {
    let cfg = PageConfig {
        size: PageSize::Custom {
            w_mm: page.width * MM_PER_PX,
            h_mm: page.height * MM_PER_PX,
        },
        margins: Some(Margins {
            top_mm: 0.0,
            right_mm: 0.0,
            bottom_mm: 0.0,
            left_mm: 0.0,
        }),
        scale: 1.0,
        ..PageConfig::default()
    };
    PageGeometry::new(&cfg)
}

/// Начертания, встретившиеся на страницах: ими подрезаются встроенные шрифты.
///
/// Обход рекурсивный: содержимое ячеек таблицы — такие же элементы раскладки, и
/// начертание из ячейки обязано попасть в набор, даже пока таблицы не рисуются.
fn collect_faces(layout: &PageLayout) -> FaceSet {
    fn walk(items: &[LayoutItem], faces: &mut FaceSet) {
        for item in items {
            match item {
                LayoutItem::Paragraph { bold, italic, .. } => {
                    faces.insert(Face::of(*bold, *italic));
                }
                LayoutItem::Table { cells, .. } => {
                    for cell in cells {
                        walk(&cell.content, faces);
                    }
                }
                LayoutItem::Float(_)
                | LayoutItem::Image { .. }
                | LayoutItem::PageBreak
                | LayoutItem::ColumnBreak => {}
            }
        }
    }

    let mut faces = FaceSet::default();
    for page in &layout.pages {
        walk(&page.items, &mut faces);
    }
    // Начертания колонтитулов нужны так же: шрифт подрезается до записи первой
    // страницы, и полужирный колонтитул без своей грани вышел бы regular'ом.
    for chrome in &layout.chrome {
        walk(&chrome.header, &mut faces);
        walk(&chrome.footer, &mut faces);
    }
    faces
}

/// Символы каждой страницы по начертаниям: то, что реально попадёт в PDF.
///
/// Источник — те же операции, что уйдут в запись ([`draw_page`]): пре-скан
/// текста по элементам раскладки разошёлся бы с отрисовкой при первой же правке
/// (например, когда таблицы начнут рисовать своё содержимое). Ключ накопления —
/// [`Face`], а не [`FontId`]: зонды идут до сессии, и id шрифтов ещё не
/// существует; обратное сопоставление делает [`EmbeddedFonts::face`].
fn collect_glyphs(
    layout: &PageLayout,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &images::Images,
) -> BTreeMap<Face, BTreeSet<char>> {
    let mut glyphs: BTreeMap<Face, BTreeSet<char>> = BTreeMap::new();
    for (index, page) in layout.pages.iter().enumerate() {
        let geometry = geometry(page);
        for op in draw_page(
            page,
            layout.chrome.get(index),
            &geometry,
            embedded,
            registry,
            images,
        ) {
            if let Op::WriteText { font, items } = op {
                let set = glyphs.entry(embedded.face(&font)).or_default();
                for item in items {
                    if let TextItem::Text(text) = item {
                        set.extend(text.chars());
                    }
                }
            }
        }
    }
    glyphs
}

/// Операции одной страницы в порядке рисования.
///
/// `chrome` — колонтитулы этой страницы; `None` — у раскладки их нет (короткий
/// `chrome` при рассинхроне): страница выходит без колонтитулов, без паники —
/// так же решает canvas-путь ([`doc_converter_docx::render::build`]).
///
/// Порядок — тело, затем верхний колонтитул, затем нижний: колонтитул лежит в
/// полосе полей и с телом не пересекается, но если он перерос своё поле, поверх
/// текста должен остаться он — там же, где его ждёт Word, раздвигая полосу
/// набора. Так же поступает и canvas-путь: два слоя одного документа не должны
/// расходиться на первой же правке.
fn draw_page(
    page: &Page,
    chrome: Option<&PageChrome>,
    geometry: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &images::Images,
) -> Vec<Op> {
    let mut ops = Vec::new();
    // Диспетчер общий с телом: абзацы, таблицы и рисунки колонтитула рисуются
    // теми же путями, что и элементы страницы.
    draw_items(
        &page.items,
        &mut ops,
        geometry,
        embedded,
        registry,
        images,
        0.0,
    );
    if let Some(chrome) = chrome {
        for items in [&chrome.header, &chrome.footer] {
            draw_items(items, &mut ops, geometry, embedded, registry, images, 0.0);
        }
    }
    ops
}

/// Диспетчер по вариантам [`LayoutItem`].
///
/// Ячейки таблицы сюда не спускаются: их содержимое рисует сам слайс таблиц
/// (`tables::draw`) — иначе он не смог бы разложить содержимое по ячейкам.
///
/// `offset_y` — вертикальный сдвиг в пикселях раскладки: им содержимое ячейки
/// смещается по `w:vAlign` (`VAlign::Center`/`Bottom`), на верхнем уровне он
/// нулевой.
fn draw_items(
    items: &[LayoutItem],
    ops: &mut Vec<Op>,
    page: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &images::Images,
    offset_y: f32,
) {
    for item in items {
        match item {
            LayoutItem::Paragraph {
                rect,
                text,
                size,
                color,
                bold,
                italic,
                fragments,
                ..
            } => {
                // Подсветка — фон под текстом: прямоугольник на всю высоту
                // строки, по ширине фрагмента. Рисуется до текста, иначе
                // заливка перекрыла бы знаки.
                draw_highlights(ops, page, rect, fragments, offset_y);
                draw_line(
                    ops,
                    page,
                    embedded,
                    registry,
                    LineRun {
                        // Сдвиг снимается здесь: дальше строка живёт в своих
                        // координатах, как и любая строка тела документа.
                        rect: Rect::new(rect.x, rect.y + offset_y, rect.width, rect.height),
                        text,
                        size: *size,
                        color: *color,
                        bold: *bold,
                        italic: *italic,
                    },
                );
            }
            LayoutItem::Table { cells, .. } => {
                tables::draw(ops, cells, page, embedded, registry, images, offset_y);
            }
            LayoutItem::Float(float) => {
                floats::draw(ops, float, page, embedded, registry, images, offset_y);
            }
            // Разрывы — метки раскладки: страницы уже разрезаны, рисовать нечего.
            LayoutItem::PageBreak | LayoutItem::ColumnBreak => {}
            // Без байтов (`image_id: None`) рисунок не рисуется — поведение
            // экспорта до этого среза: места в раскладке он уже занял, но
            // показать нечего.
            LayoutItem::Image { rect, image_id, .. } => {
                ops.extend(images.ops(*image_id, *rect, offset_y, page));
            }
        }
    }
}

/// Залить фоном подсвеченные фрагменты строки.
///
/// Фрагмент несёт смещение и ширину в пикселях раскладки; высота — вся строка,
/// как рисует подсветку Word. Строки без подсветки не дают ни одной операции.
fn draw_highlights(
    ops: &mut Vec<Op>,
    page: &PageGeometry,
    rect: &Rect,
    fragments: &[doc_converter_docx::layout::LineFragment],
    offset_y: f32,
) {
    for fragment in fragments {
        let Some(rgb) = fragment.highlight else {
            continue;
        };
        let rect = crate::layout::RectPt {
            x: page.px_to_pt(rect.x + fragment.x),
            y: page.px_to_pt(rect.y + offset_y),
            w: page.px_to_pt(fragment.width),
            h: page.px_to_pt(rect.height),
        };
        crate::background::fill_rect(ops, rect, page.height_pt(), highlight_color(rgb));
    }
}

/// Цвет подсветки `RRGGBB` → `RRGGBBAA`, как у [`text_color`].
fn highlight_color(rgb: u32) -> Color {
    Color(((rgb & 0x00FF_FFFF) << 8) | 0xFF)
}

/// Строка абзаца для отрисовки: поля [`LayoutItem::Paragraph`] без
/// идентификатора узла и гарнитуры (шрифт у экспорта один).
#[derive(Clone, Copy)]
struct LineRun<'a> {
    /// Прямоугольник строки: `y` — её верх.
    rect: Rect,
    /// Текст строки, уже разбитый раскладкой по словам.
    text: &'a str,
    /// Кегль в полупунктах.
    size: HalfPoint,
    /// Цвет знаков (`RRGGBB`); `None` — цвет по умолчанию.
    color: Option<u32>,
    /// Полужирный.
    bold: bool,
    /// Курсив.
    italic: bool,
}

/// Нарисовать строку абзаца.
///
/// PDF ставит текст по базовой линии, а раскладка знает верх строки: подъём
/// берётся из typo-асцента (`sTypoAscender`) шрифта, каким мерила раскладка
/// (`DEFAULT_FONT_ID`), с usWin-фолбэком — так «чернила» ставит `LibreOffice`.
/// Выравнивание повторно не применяется: оно уже учтено в `rect.x` при
/// раскладке.
fn draw_line(
    ops: &mut Vec<Op>,
    page: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    run: LineRun<'_>,
) {
    if run.text.is_empty() || run.rect.width <= 0.0 || run.rect.height <= 0.0 {
        return;
    }
    let size_px = doc_converter_docx::layout::half_points_to_px(run.size.value());
    let metrics = registry.measure(DEFAULT_FONT_ID, size_px, "");
    let cursor_x_pt = page.px_to_pt(run.rect.x);
    // Базовая линия — по typo-асценту (`sTypoAscender`), как ставит текст
    // LibreOffice, эталон оракула: usWin/hhea-асцент у Carlito 0,952 em против
    // typo 0,75 em опускает «чернила» примерно на 2 pt — главный источник
    // разрыва SSIM (4,6 px @150 dpi на long_table). Шрифту без typo-метрик
    // (нет OS/2 или нулевое поле) — фолбэк на usWin, как рекомендуют
    // спецификации.
    let ascent_px = typo_ascent_px(size_px).unwrap_or(metrics.ascent);
    let baseline_pt = page.px_to_pt(run.rect.y + ascent_px);
    let font = embedded.id(Face::of(run.bold, run.italic)).clone();

    ops.push(Op::SetFillColor {
        col: to_pdf_color(text_color(run.color)),
    });
    // Каждая строка — своё `BT`/`ET`: `Td` внутри текстовой секции смещает
    // начало относительно предыдущего, а координаты у нас абсолютные.
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point {
            x: Pt(cursor_x_pt),
            y: Pt(page.height_pt() - baseline_pt),
        },
    });
    ops.push(Op::SetFontSize {
        size: Pt(page.px_to_pt(size_px)),
        font: font.clone(),
    });
    ops.push(Op::WriteText {
        items: vec![TextItem::Text(run.text.to_owned())],
        font,
    });
    ops.push(Op::EndTextSection);
}

/// Подъём базовой линии по typo-асценту (`sTypoAscender`) шрифта по умолчанию,
/// в пикселях; `None` — typo-метрик нет (нет OS/2 или нулевое поле: старые
/// версии таблицы).
///
/// [`FontRegistry::measure`] отдаёт usWin/hhea-асцент (стратегия `FreeType`),
/// а не typo, — за `sTypoAscender` приходится идти напрямую; значение в
/// em-долях умножается на `size_px`.
fn typo_ascent_px(size_px: f32) -> Option<f32> {
    let font = skrifa::FontRef::new(default_font_bytes()).ok()?;
    let upem = f32::from(font.head().ok()?.units_per_em());
    let typo_ascender = font.os2().ok()?.s_typo_ascender();
    (upem > 0.0 && typo_ascender != 0).then(|| f32::from(typo_ascender) / upem * size_px)
}

/// Цвет знака: раскладка хранит `w:color` как `RRGGBB`, а [`Color`] ждёт
/// `RRGGBBAA` — каналы меняются местами, альфа добавляется непрозрачная.
///
/// Значение без `w:color` — чёрный: `auto` в OOXML и есть цвет текста по
/// умолчанию. То же преобразование, что у canvas-пути
/// (`doc_converter_docx::render`).
fn text_color(rgb: Option<u32>) -> Color {
    match rgb {
        Some(rgb) => Color(((rgb & 0x00FF_FFFF) << 8) | 0xFF),
        None => Color::BLACK,
    }
}
