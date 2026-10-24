//! PDF-экспорт PPTX: слайд = страница, размер страницы из `p:sldSz`.
//!
//! Раскладка приходит от `doc-converter-pptx-layout` ([`layout_slide`]) в
//! пикселях при 96 dpi от левого верхнего угла слайда — те же единицы, что у
//! canvas-пути (`pptx-render`). Здесь они переводятся в точки общей геометрией
//! ([`PageGeometry`]), как у DOCX: вторая конвенция пересчёта разошлась бы с
//! первой на первой же правке.
//!
//! Шрифт один — Carlito из `doc-converter-render`, тот же, которым раскладка
//! мерила текст, поэтому переносы в PDF совпадают с canvas-путём по построению
//! (ADR-0007). Диаграммы рисует общий PDF-бэкенд
//! ([`doc_converter_pdf::chart`], ADR-0011), картинки — общий реестр
//! `XObject`'ов ([`doc_converter_pdf::image`], ADR-0022). Крейт пишет свой
//! экспортёр поверх публичного постраничного бэкенда `doc-converter-pdf`
//! (ADR-0033: просмотрщик и экспортёр разнесены по модулям).
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::FRAC_PI_2;

use doc_converter_core::units::MM_PER_PX;
use doc_converter_pdf::chart;
use doc_converter_pdf::fonts::{EmbeddedFonts, Face, FaceSet};
use doc_converter_pdf::image::{self, Placement};
use doc_converter_pdf::layout::{PageGeometry, RectPx};
use doc_converter_pdf::pdf::content;
use doc_converter_pdf::pdf::export;
use doc_converter_pdf::pdf::ops::{
    Line, LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, TextItem, WindingOrder,
    XObjectId,
};
use doc_converter_pdf::pdf::session::Session;
use doc_converter_pdf::styles::to_pdf_color;
use doc_converter_pdf::{Margins, PageConfig, PageSize, PdfOptions};
use doc_converter_pptx::model::{ImagePart, Presentation, Rgb};
use doc_converter_pptx_layout::{layout_slide, Item, PlacedRun, SlideLayout};
use doc_converter_render::display_list::Color;
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE, DEFAULT_FONT_ID};
use doc_converter_render::geometry::Rect;

/// Ошибка экспорта PPTX в PDF.
///
/// `non_exhaustive`: набор вариантов будет расти вместе с бэкендом.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Раскладка слайда не собралась.
    #[error("pptx layout failed: {0}")]
    Layout(String),
    /// В презентации нет слайдов: PDF без страниц невалиден.
    #[error("presentation has no slides")]
    NoSlides,
    /// Шрифт не разобран: битые байты или отказ подрезки.
    #[error("invalid font data: {0}")]
    Font(String),
    /// Картинка не декодирована: неподдержанный mime или битые байты.
    #[error("invalid image data: {0}")]
    Image(String),
    /// Приёмник PDF вернул ошибку записи.
    #[error("write failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Ошибка бэкенда `doc-converter-pdf` в терминах этого крейта.
impl From<doc_converter_pdf::PdfError> for Error {
    fn from(error: doc_converter_pdf::PdfError) -> Self {
        use doc_converter_pdf::PdfError as Backend;
        match error {
            Backend::NoPages => Self::NoSlides,
            Backend::Font(message) => Self::Font(message),
            Backend::Image(message) => Self::Image(message),
            Backend::Io(io) => Self::Io(io),
            other => Self::Layout(format!("pdf: {other}")),
        }
    }
}

/// Результат операций крейта.
pub type Result<T> = std::result::Result<T, Error>;

/// Сегментов дуги на четверть скруглённого угла: восемь точек дают угол,
/// неотличимый на странице, и не нагружают поток содержимого.
const ARC_SEGMENTS: usize = 8;

/// Собрать PDF из всех слайдов презентации.
///
/// # Errors
/// [`Error::Layout`], если раскладка слайда не собралась; [`Error::NoSlides`],
/// если в презентации нет слайдов; [`Error::Font`], если байты шрифта не
/// разобрались; [`Error::Image`], если сессия не приняла картинку;
/// [`Error::Io`], если приёмник вернул ошибку записи.
pub fn export(pres: &Presentation, options: &PdfOptions) -> Result<Vec<u8>> {
    let layouts = (0..pres.slide_count())
        .map(|index| layout_slide(pres, index))
        .collect::<std::result::Result<Vec<_>, doc_converter_pptx::Error>>()
        .map_err(|error| Error::Layout(error.to_string()))?;
    layouts.first().ok_or(Error::NoSlides)?;

    // Геометрия страницы зависит только от размера слайда: считается один раз
    // на слайд и переиспользуется обоими проходами (сбор глифов и запись
    // страниц), чтобы не строить `PageGeometry` дважды.
    let geometries: Vec<PageGeometry> = layouts.iter().map(geometry).collect();

    let faces = collect_faces(&layouts);
    let embedded = EmbeddedFonts::from_faces(faces);
    let mut images = Images::collect(pres);
    let mut registry = FontRegistry::new(DEFAULT_FONT_CACHE);

    // Шрифты подрезаются до записи первой страницы (`Session::add_font`), а
    // подрезка перенумеровывает глифы: символ, не попавший в зонды, вывелся бы
    // нулевым глифом — молча. Поэтому символы собираются первой отрисовкой, той
    // же `draw_slide`, что пойдёт в PDF, — как у листа XLSX (`painter`) и
    // документа DOCX (`docx::export`).
    let glyphs = collect_glyphs(&layouts, &geometries, &embedded, &mut registry, &images);

    let mut out = Vec::new();
    let mut session = Session::begin(&mut out, layouts.len(), options.compress)?;
    if !options.title.is_empty() {
        session.set_title(&options.title);
    }
    session.set_info(&[
        (String::from("Author"), options.author.clone()),
        (String::from("Subject"), options.subject.clone()),
        (String::from("Keywords"), options.keywords.join(",")),
    ]);

    // Шрифты — в фиксированном порядке начертаний, regular всегда: без него
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
    let names: Vec<String> = images
        .registry()
        .decoded()
        .iter()
        .map(|decoded| session.add_image(decoded))
        .collect::<std::result::Result<_, doc_converter_pdf::PdfError>>()?;
    for (index, name) in names.into_iter().enumerate() {
        images.assign(index, XObjectId(name));
    }

    for (layout, geometry) in layouts.iter().zip(&geometries) {
        let ops = draw_slide(layout, geometry, &embedded, &mut registry, &images);
        // Карта шрифтов — та, что живёт в сессии: `translate` занимает её
        // неизменяемо и возвращает готовые байты, поэтому заимствование
        // кончается до `write_page` с его `&mut self`.
        let content = content::translate(&ops, session.prepared_fonts())?;
        session.write_page(
            content,
            [0.0, 0.0, geometry.width_pt(), geometry.height_pt()],
            &[],
        )?;
    }
    session.finish()?;
    Ok(out)
}

/// Начертания, встретившиеся в тексте слайдов: ими подрезаются шрифты.
fn collect_faces(layouts: &[SlideLayout]) -> FaceSet {
    // Пары (полужирный, курсив) по строкам текста слайдов; вставку начертаний
    // делает общий [`export::collect_faces`], как и у DOCX.
    let pairs = layouts
        .iter()
        .flat_map(|layout| &layout.items)
        .filter_map(|item| match item {
            Item::Text { lines } => Some(lines),
            _ => None,
        })
        .flatten()
        .flat_map(|line| &line.runs)
        .map(|run| (run.bold, run.italic));
    let mut faces = export::collect_faces(pairs);
    // Регулярное начертание нужно всегда: шрифт подрезается до записи первой
    // страницы, и слайд без текста вышел бы вообще без встроенного шрифта.
    faces.insert(Face::Regular);
    faces
}

/// Символы, которые слайды выводят каждым начертанием: первая отрисовка,
/// та же, что пойдёт в PDF, — источник один, и подрезка не потеряет глифов.
fn collect_glyphs(
    layouts: &[SlideLayout],
    geometries: &[PageGeometry],
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &Images<'_>,
) -> BTreeMap<Face, BTreeSet<char>> {
    let mut glyphs: BTreeMap<Face, BTreeSet<char>> = BTreeMap::new();
    for (layout, geometry) in layouts.iter().zip(geometries) {
        let ops = draw_slide(layout, geometry, embedded, registry, images);
        // Обход команд общий с DOCX/XLSX: одна правка `WriteText` — все пути.
        let page_glyphs = export::collect_text_glyphs(&ops, |font| embedded.face(font));
        for (face, chars) in page_glyphs {
            glyphs.entry(face).or_default().extend(chars);
        }
    }
    glyphs
}

/// Геометрия страницы: слайд уже в пикселях 96 dpi, координаты элементов — от
/// левого верхнего угла слайда. Поля нулевые, масштаб 100%: размер страницы
/// задаёт `p:sldSz`, а не настройки печати.
fn geometry(layout: &SlideLayout) -> PageGeometry {
    let cfg = PageConfig {
        size: PageSize::Custom {
            w_mm: layout.width * MM_PER_PX,
            h_mm: layout.height * MM_PER_PX,
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

/// Отрисовать слайд в операции PDF: фон, шейпы, картинки, диаграммы, текст —
/// в порядке элементов раскладки (он задаёт наложение).
fn draw_slide(
    layout: &SlideLayout,
    page: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    images: &Images<'_>,
) -> Vec<Op> {
    let mut ops = Vec::new();
    if let Some(background) = layout.background {
        draw_rect(
            &mut ops,
            page,
            RectPx::new(0.0, 0.0, layout.width, layout.height),
            Some(background),
            None,
            0.0,
            0.0,
        );
    }
    for item in &layout.items {
        match item {
            Item::Rect {
                rect,
                fill,
                stroke,
                stroke_w,
                radius,
            } => draw_rect(
                &mut ops,
                page,
                RectPx::new(rect.x, rect.y, rect.w, rect.h),
                *fill,
                *stroke,
                *stroke_w,
                *radius,
            ),
            Item::Image { rect, image_id } => {
                ops.extend(images.ops(*image_id, *rect, page));
            }
            Item::Chart { rect, data } => {
                if rect.w <= 0.0 || rect.h <= 0.0 {
                    continue;
                }
                let anchor = RectPx::new(rect.x, rect.y, rect.w, rect.h);
                let chart = chart::Layout::new(anchor, data);
                chart::draw(
                    &mut ops,
                    registry,
                    embedded.id(Face::Regular),
                    page,
                    anchor,
                    anchor,
                    &chart,
                );
            }
            Item::Text { lines } => {
                for line in lines {
                    for run in &line.runs {
                        draw_run(&mut ops, page, embedded, registry, line.x, line.y, run);
                    }
                }
            }
        }
    }
    ops
}

/// Заливка и контур шейпа. Скругление аппроксимируется полигоном с дугами по
/// [`ARC_SEGMENTS`] точек на четверть — у PDF нет команды дуги.
fn draw_rect(
    ops: &mut Vec<Op>,
    page: &PageGeometry,
    rect: RectPx,
    fill: Option<Rgb>,
    stroke: Option<Rgb>,
    stroke_w: f32,
    radius: f32,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let points = shape_points(page, rect, radius);
    let polygon = |mode| Polygon {
        rings: vec![PolygonRing {
            points: points.clone(),
        }],
        mode,
        winding_order: WindingOrder::NonZero,
    };
    if let Some(fill) = fill {
        ops.push(Op::SetFillColor {
            col: to_pdf_color(text_color(Some(fill))),
        });
        ops.push(Op::DrawPolygon {
            polygon: polygon(PaintMode::Fill),
        });
    }
    if let Some(stroke) = stroke {
        if stroke_w <= 0.0 {
            return;
        }
        ops.push(Op::SetOutlineColor {
            col: to_pdf_color(text_color(Some(stroke))),
        });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(page.px_to_pt(stroke_w)),
        });
        ops.push(Op::DrawPolygon {
            polygon: polygon(PaintMode::Stroke),
        });
    }
}

/// Точки контура прямоугольника со скруглением, в координатах PDF (левый
/// нижний угол, ось `y` вверх). Без скругления — четыре угла.
fn shape_points(page: &PageGeometry, rect: RectPx, radius: f32) -> Vec<LinePoint> {
    let pdf = page.rect_to_pt(rect).to_pdf(page.height_pt());
    let (left, bottom) = (pdf.x.0, pdf.y.0);
    let (right, top) = (left + pdf.width.0, bottom + pdf.height.0);
    let radius = radius
        .min(page.px_to_pt(rect.w) / 2.0)
        .min(page.px_to_pt(rect.h) / 2.0)
        .max(0.0);
    if radius <= 0.0 {
        return [(left, bottom), (right, bottom), (right, top), (left, top)]
            .into_iter()
            .map(point)
            .collect();
    }
    let mut points = Vec::with_capacity(4 * (ARC_SEGMENTS + 1));
    // Каждый угол — четверть окружности вокруг своего центра; стороны между
    // дугами — прямые отрезки. Обход против часовой стрелки.
    let arc = |cx: f32, cy: f32, start: f32, out: &mut Vec<LinePoint>| {
        for step in 0..=ARC_SEGMENTS {
            #[allow(clippy::cast_precision_loss)]
            let angle = start + FRAC_PI_2 * (step as f32 / ARC_SEGMENTS as f32);
            out.push(point((
                cx + radius * angle.cos(),
                cy + radius * angle.sin(),
            )));
        }
    };
    arc(left + radius, bottom + radius, -FRAC_PI_2, &mut points); // левый нижний
    arc(right - radius, bottom + radius, 0.0, &mut points); // правый нижний
    arc(right - radius, top - radius, FRAC_PI_2, &mut points); // правый верхний
    arc(
        left + radius,
        top - radius,
        std::f32::consts::PI,
        &mut points,
    ); // левый верхний
    points
}

/// Точка контура в координатах PDF.
fn point((x, y): (f32, f32)) -> LinePoint {
    LinePoint {
        p: Point { x: Pt(x), y: Pt(y) },
        bezier: false,
    }
}

/// Строка прогона: базовую линию даёт ascent метрик того же шрифта, что мерил
/// раскладку; `line.y` — верх строки. Каждый прогон — своё `BT`/`ET`: `Td`
/// внутри текстовой секции смещает начало относительно предыдущего, а
/// координаты у нас абсолютные.
fn draw_run(
    ops: &mut Vec<Op>,
    page: &PageGeometry,
    embedded: &EmbeddedFonts,
    registry: &mut FontRegistry,
    line_x: f32,
    line_y: f32,
    run: &PlacedRun,
) {
    if run.text.is_empty() || run.size_px <= 0.0 {
        return;
    }
    let metrics = registry.measure(DEFAULT_FONT_ID, run.size_px, "");
    let baseline_px = line_y + metrics.ascent;
    let x_px = line_x + run.x;
    let font = embedded.id(Face::of(run.bold, run.italic)).clone();

    ops.push(Op::SetFillColor {
        col: to_pdf_color(text_color(run.color)),
    });
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point {
            x: Pt(page.origin_x_pt() + page.px_to_pt(x_px)),
            y: Pt(page.height_pt() - page.origin_y_pt() - page.y_px_to_pt(baseline_px)),
        },
    });
    ops.push(Op::SetFontSize {
        size: Pt(page.px_to_pt(run.size_px)),
        font: font.clone(),
    });
    ops.push(Op::WriteText {
        items: vec![TextItem::Text(run.text.clone())],
        font,
    });
    ops.push(Op::EndTextSection);

    if run.underline {
        let width_px = registry
            .measure(DEFAULT_FONT_ID, run.size_px, &run.text)
            .width;
        let y_pt = page.height_pt()
            - page.origin_y_pt()
            - page.y_px_to_pt(baseline_px)
            - page.px_to_pt(run.size_px * 0.12);
        ops.push(Op::SetOutlineColor {
            col: to_pdf_color(text_color(run.color)),
        });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(page.px_to_pt(run.size_px * 0.06).max(0.5)),
        });
        ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    point((page.origin_x_pt() + page.px_to_pt(x_px), y_pt)),
                    point((page.origin_x_pt() + page.px_to_pt(x_px + width_px), y_pt)),
                ],
                is_closed: false,
            },
        });
    }
}

/// Цвет знака: раскладка хранит `None` как «тёмный цвет темы» — для PDF это
/// чёрный, как у canvas-пути. `Rgb` — `RRGGBB`, а [`Color`] ждёт `RRGGBBAA`.
fn text_color(rgb: Option<Rgb>) -> Color {
    match rgb {
        Some(rgb) => Color::from_rgb24(rgb.to_u32()),
        None => Color::BLACK,
    }
}

/// Картинки презентации: `image_id` раскладки → имя `XObject` в сессии.
///
/// Реестр общий с листом XLSX ([`doc_converter_pdf::image`]): дедупликация по байтам,
/// декодирование PNG/JPEG, размещение с отсечением по прямоугольнику якоря.
struct Images<'a> {
    /// Media-часть по `image_id` — для размещения при отрисовке слайда.
    by_id: BTreeMap<u32, &'a ImagePart>,
    /// Общий реестр декодированных картинок; имена назначает сессия.
    registry: image::Registry,
}

impl<'a> Images<'a> {
    /// Декодирует все media-части презентации и строит индекс по `image_id`.
    fn collect(pres: &'a Presentation) -> Self {
        // Прямоугольник размещения реестру не важен — он декодирует байты;
        // настоящие якоря приходят при отрисовке.
        let placements: Vec<Placement<'a>> = pres
            .images
            .iter()
            .map(|part| Placement {
                id: &part.name,
                mime: &part.mime,
                bytes: &part.bytes,
                rect: doc_converter_pdf::pdf::ops::Rect {
                    x: Pt(0.0),
                    y: Pt(0.0),
                    width: Pt(1.0),
                    height: Pt(1.0),
                },
            })
            .collect();
        let mut registry = image::Registry::default();
        registry.collect(&placements);
        Self {
            by_id: pres.images.iter().map(|part| (part.id, part)).collect(),
            registry,
        }
    }

    /// Реестр декодированных картинок — аргументы `Session::add_image`.
    fn registry(&self) -> &image::Registry {
        &self.registry
    }

    /// Назначить имя `XObject` картинке с индексом в порядке регистрации.
    fn assign(&mut self, index: usize, name: XObjectId) {
        self.registry.assign(index, name);
    }

    /// Операции вывода картинки в прямоугольник раскладки; пусто — картинки
    /// нет или имя ещё не назначено (зондовый проход).
    fn ops(&self, image_id: u32, rect: Rect, page: &PageGeometry) -> Vec<Op> {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Vec::new();
        }
        let Some(part) = self.by_id.get(&image_id) else {
            return Vec::new();
        };
        let pdf = page
            .rect_to_pt(RectPx::new(rect.x, rect.y, rect.w, rect.h))
            .to_pdf(page.height_pt());
        let placements = [Placement {
            id: &part.name,
            mime: &part.mime,
            bytes: &part.bytes,
            rect: pdf,
        }];
        image::place_all(&self.registry, &placements).ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_converter_pptx::model::{
        Align, Frame, Insets, Paragraph, Run, RunProps, Shape, ShapeKind, Slide, SlideSize,
        TextBody, TextBox,
    };

    /// Презентация из двух слайдов: текст и цветной прямоугольник.
    fn presentation() -> Presentation {
        let text = |text: &str| Shape {
            name: "text".into(),
            frame: Frame {
                x: 914_400,
                y: 914_400,
                w: 9_144_000,
                h: 2_743_200,
            },
            fill: None,
            line: None,
            kind: ShapeKind::Text(TextBox {
                body: TextBody {
                    insets: Insets {
                        left: 91_440,
                        top: 45_720,
                        right: 91_440,
                        bottom: 45_720,
                    },
                    wrap: true,
                    paragraphs: vec![Paragraph {
                        align: Align::Left,
                        runs: vec![Run {
                            text: text.into(),
                            props: RunProps {
                                size: Some(24.0),
                                bold: true,
                                ..RunProps::default()
                            },
                        }],
                        ..Paragraph::default()
                    }],
                    ..TextBody::default()
                },
            }),
        };
        let rect = Shape {
            name: "rect".into(),
            frame: Frame {
                x: 0,
                y: 4_572_000,
                w: 3_048_000,
                h: 914_400,
            },
            fill: Some(Rgb::new(0x44, 0x72, 0xC4)),
            line: Some(doc_converter_pptx::model::Line {
                color: Some(Rgb::new(0x00, 0x00, 0x00)),
                width: Some(95_250),
            }),
            kind: ShapeKind::Geometry(doc_converter_pptx::model::GeometryPreset::Rect),
        };
        Presentation {
            size: SlideSize::default(),
            slides: vec![
                Slide {
                    name: "First".into(),
                    background: Some(Rgb::new(0xFF, 0xFF, 0xFF)),
                    shapes: vec![text("Hello PDF"), rect],
                },
                Slide {
                    name: "Second".into(),
                    shapes: vec![text("Second slide")],
                    ..Slide::default()
                },
            ],
            ..Presentation::default()
        }
    }

    #[test]
    fn export_produces_one_page_per_slide() {
        let pres = presentation();
        let pdf = export(&pres, &PdfOptions::default()).expect("export");
        assert!(pdf.starts_with(b"%PDF"), "заголовок PDF");
        assert!(pdf.ends_with(b"%%EOF"), "хвост PDF");
        let doc = lopdf::Document::load_mem(&pdf).expect("валидный PDF");
        assert_eq!(doc.get_pages().len(), 2, "слайд = страница");
    }

    #[test]
    fn empty_presentation_is_rejected() {
        let pres = Presentation::default();
        assert!(matches!(
            export(&pres, &PdfOptions::default()),
            Err(Error::NoSlides)
        ));
    }
}
