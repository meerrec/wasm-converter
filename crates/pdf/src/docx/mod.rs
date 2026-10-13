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
//! Подчёркивание в PDF пока не рисуется: printpdf линию сам не ставит, а
//! отдельный срез для неё ещё не сделан — ровно как у ячеек XLSX
//! (`crate::styles::CellStyle::underline`).

mod floats;
mod images;
mod tables;

use std::collections::{BTreeMap, BTreeSet};

use doc_converter_docx::layout::{LayoutItem, Page, PageChrome, PageLayout, Rect};
use doc_converter_docx::{Document, HalfPoint, LayoutOptions};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_ID};
use printpdf::streaming::StreamSession;
use printpdf::{
    FontId, Mm, Op, ParsedFont, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg, Point, Pt,
    TextItem,
};

use crate::fonts::{Face, FaceSet};
use crate::layout::PageGeometry;
use crate::options::{Margins, PageConfig, PageSize, PdfOptions};
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

// --- Встраивание шрифтов: DOCX-путь пока на printpdf (ADR-0022, слайс 5) ---

/// Шрифт, встроенный в документ printpdf.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EmbeddedFont {
    id: FontId,
}

impl EmbeddedFont {
    /// Идентификатор в ресурсах документа: им помечаются текстовые операции.
    #[must_use]
    const fn id(&self) -> &FontId {
        &self.id
    }
}

/// Шрифты страниц DOCX: regular всегда, выделенные начертания — как
/// встретились. Двойник `crate::fonts::EmbeddedFonts` (своего бэкенда) на
/// printpdf-`FontId`: DOCX-путь переезжает на свой бэкенд следующей волной, а
/// до неё общий модуль шрифтов ему не годится.
#[derive(Debug)]
struct EmbeddedFonts {
    regular: EmbeddedFont,
    bold: Option<EmbeddedFont>,
    italic: Option<EmbeddedFont>,
    bold_italic: Option<EmbeddedFont>,
}

impl EmbeddedFonts {
    /// Шрифт начертания; невстроенное начертание заменяется regular.
    ///
    /// Замена — страховка на случай, если спрашивают начертание, которого в
    /// наборе не было: пустой записи текста printpdf не примет.
    #[must_use]
    fn id(&self, face: Face) -> &FontId {
        let embedded = match face {
            Face::Regular => None,
            Face::Bold => self.bold.as_ref(),
            Face::Italic => self.italic.as_ref(),
            Face::BoldItalic => self.bold_italic.as_ref(),
        };
        embedded.unwrap_or(&self.regular).id()
    }
}

/// Встроить начертания из набора (и всегда — regular).
///
/// Разбор TTF и подстановка (subsetting) — на стороне printpdf: он же
/// собирает `ToUnicode`, без которого кириллицу не извлечь из готового PDF.
///
/// # Errors
/// [`PdfError::Font`], если printpdf не принял байты шрифта.
fn embed(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
    faces: FaceSet,
) -> Result<EmbeddedFonts, PdfError> {
    Ok(EmbeddedFonts {
        regular: embed_face(doc, warnings, Face::Regular)?,
        bold: embed_optional(doc, warnings, faces, Face::Bold)?,
        italic: embed_optional(doc, warnings, faces, Face::Italic)?,
        bold_italic: embed_optional(doc, warnings, faces, Face::BoldItalic)?,
    })
}

/// Встроить начертание, только если оно есть в наборе.
fn embed_optional(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
    faces: FaceSet,
    face: Face,
) -> Result<Option<EmbeddedFont>, PdfError> {
    if !faces.contains(face) {
        return Ok(None);
    }
    embed_face(doc, warnings, face).map(Some)
}

/// Встроить одно начертание.
fn embed_face(
    doc: &mut PdfDocument,
    warnings: &mut Vec<PdfWarnMsg>,
    face: Face,
) -> Result<EmbeddedFont, PdfError> {
    let parsed = ParsedFont::from_bytes(face.bytes(), 0, warnings)
        .ok_or_else(|| PdfError::Font(format!("printpdf rejected the {face:?} font")))?;
    Ok(EmbeddedFont {
        id: doc.add_font(&parsed),
    })
}

/// Цвет `DisplayList` (`RRGGBBAA`) в цвет printpdf (нормированный RGB).
///
/// Двойник `crate::styles::to_pdf_color` (своего бэкенда): DOCX-путь на
/// printpdf до следующей волны. Альфа в PDF не переносится — прозрачность
/// потребовала бы `/ExtGState`, это отдельный срез; непрозрачный цвет от неё
/// не зависит.
#[must_use]
fn to_pdf_color(color: Color) -> printpdf::Color {
    let [r, g, b, _alpha] = color.0.to_be_bytes();
    printpdf::Color::Rgb(printpdf::Rgb::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        None,
    ))
}

/// Собрать PDF из всех страниц документа.
///
/// # Errors
/// [`PdfError::DocxLayout`], если раскладка не собралась; [`PdfError::NoPages`],
/// если она не дала ни одной страницы; [`PdfError::Font`], если printpdf не
/// принял байты шрифта; [`PdfError::Io`], если приёмник вернул ошибку записи.
pub(crate) fn export(doc: &Document, options: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    let mut registry = FontRegistry::new(FONT_CACHE);
    let layout =
        doc_converter_docx::layout::layout_document(doc, &LayoutOptions::default(), &mut registry)
            .map_err(PdfError::DocxLayout)?;
    // PDF без страниц невалиден: у него нет каталога страниц.
    let first_page = layout.pages.first().ok_or(PdfError::NoPages)?;

    let mut warnings: Vec<PdfWarnMsg> = Vec::new();
    let mut pdf = PdfDocument::new(&title(doc, options));
    apply_metadata(&mut pdf, doc, options);
    let embedded = embed(&mut pdf, &mut warnings, collect_faces(&layout))?;

    // `XObject`'ы — ресурсы документа: `StreamSession` переводит по ним
    // `Op::UseXobject` в момент записи страницы, поэтому рисунки обязаны лечь в
    // ресурсы до `begin` — так же, как у листа XLSX (`painter::image_ops`).
    let images = images::Images::collect(doc, &mut pdf.resources);

    // Шрифты подрезаются до записи первой страницы (`StreamSession::begin`), а
    // подрезка перенумеровывает глифы: символ, не попавший в зонды, вывелся бы
    // нулевым глифом — молча. Поэтому символы собираются первой отрисовкой, той
    // же `draw_page`, что пойдёт в PDF, — так же, как у листа XLSX
    // (`painter::collect_glyphs`).
    let probe = probe_pages(
        collect_glyphs(&layout, &embedded, &mut registry, &images),
        first_page,
    );

    let save = PdfSaveOptions {
        // `optimize` у printpdf сжимает потоки — это и есть `compress`.
        optimize: options.compress,
        ..PdfSaveOptions::default()
    };
    let mut out = Vec::new();
    let mut session = StreamSession::begin(
        &pdf,
        &probe,
        BTreeSet::new(),
        layout.pages.len(),
        &save,
        &mut out,
        &mut warnings,
    )?;
    for (index, page) in layout.pages.iter().enumerate() {
        let geometry = geometry(page);
        let ops = draw_page(
            page,
            layout.chrome.get(index),
            &geometry,
            &embedded,
            &mut registry,
            &images,
        );
        let pdf_page = PdfPage::new(
            Mm::from(Pt(geometry.width_pt())),
            Mm::from(Pt(geometry.height_pt())),
            ops,
        );
        session.write_page(&pdf_page, &mut warnings)?;
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

/// Подставить свойства пакета `docProps/core.xml`, если настройки не заданы.
///
/// Ключевые слова не переносятся: в модели это одна строка, а в
/// [`PdfOptions::keywords`] — список, и делить её по запятым значило бы гадать о
/// формате исходника.
fn apply_metadata(doc: &mut PdfDocument, source: &Document, options: &PdfOptions) {
    let info = &mut doc.metadata.info;
    info.author = pick(&options.author, source.metadata.creator.as_deref());
    info.subject = pick(&options.subject, source.metadata.subject.as_deref());
    info.keywords.clone_from(&options.keywords);
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

/// Страницы-зонды для подрезки шрифтов: по одной на встроенное начертание, с
/// одной текстовой операцией из всех его символов.
///
/// Размер страницы-зонда printpdf не читает, но он берётся с первой страницы
/// документа — как у листа-донора в `painter::probe_pages_from`.
fn probe_pages(glyphs: BTreeMap<FontId, BTreeSet<char>>, page: &Page) -> Vec<PdfPage> {
    let geometry = geometry(page);
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
/// (например, когда таблицы начнут рисовать своё содержимое).
fn collect_glyphs(
    layout: &PageLayout,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &images::Images,
) -> BTreeMap<FontId, BTreeSet<char>> {
    let mut glyphs: BTreeMap<FontId, BTreeSet<char>> = BTreeMap::new();
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
                let set = glyphs.entry(font.clone()).or_default();
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
                ..
            } => draw_line(
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
            ),
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
/// берётся из метрик того же шрифта, каким мерила раскладка (`DEFAULT_FONT_ID`),
/// — так же считает [`crate::text`] для ячейки XLSX. Выравнивание повторно не
/// применяется: оно уже учтено в `rect.x` при раскладке.
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
    let baseline_pt = page.px_to_pt(run.rect.y + metrics.ascent);
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

/// Цвет знака: раскладка хранит `w:color` как `RRGGBB`, printpdf ждёт `RRGGBBAA`
/// в [`Color`] — каналы меняются местами, альфа добавляется непрозрачная.
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
