//! Сборка кадра страницы DOCX в [`DisplayList`].
//!
//! [`build`] переводит готовую раскладку ([`PageLayout`]) в команды рисования —
//! тот же путь, который проходит лист XLSX (`xlsx::paint::build`): кадр
//! собирается в пикселях canvas, painter читает его из ring'а и про DOCX не
//! знает ничего.
//!
//! Соглашение о координатах взято у xlsx. Раскладка живёт в пикселях от левого
//! верхнего угла страницы, команды — в физических пикселях canvas, поэтому
//! каждую точку переводит [`Viewport::layout_to_screen`], а размеры (кегль,
//! толщина линий) умножаются на `viewport.scale`, в который уже входят и зум,
//! и DPR.

use doc_converter_render::display_list::{
    Color, DisplayList, DrawCommand, StringRef, TextAlign, TextBaseline,
};
use doc_converter_render::geometry::Point;
use doc_converter_render::viewport::Viewport;

use crate::layout::engine::{half_points_to_px, LayoutItem, PageLayout, Rect};
use crate::layout::float::FloatKind;

mod tables;

use tables::draw_table;

/// Гарнитура по умолчанию: ей рисовано всё, чью гарнитуру раскладка не назвала.
///
/// Имя берётся у реестра шрифтов ([`doc_converter_render::font::DEFAULT_FONT_FAMILY`]),
/// а не пишется строкой на месте: это то же лицо, которым мерит раскладка.
/// `LayoutItem::Paragraph::font_family` при этом несёт имя измеренного лица —
/// известное реестру семейство либо запасное (см. `layout::paragraph`), — так
/// что painter ставит canvas'у ровно тот шрифт, по метрикам которого посчитаны
/// переносы. Элементы без гарнитуры (ячейки таблиц, пустые абзацы) рисуются
/// запасным лицом: `DisplayList` самодостаточен и на реестр не ссылается.
const LAYOUT_FONT_FAMILY: &str = doc_converter_render::font::DEFAULT_FONT_FAMILY;

/// Обводка плейсхолдера рисунка, которому нечем рисоваться.
const PLACEHOLDER_STROKE: Color = Color::rgba(0x80, 0x80, 0x80, 0xFF);

/// Толщина обводки плейсхолдера в пикселях раскладки.
const PLACEHOLDER_STROKE_W: f32 = 1.0;

/// Собрать страницу `page` в `DisplayList`.
///
/// `page` — индекс страницы с нуля; номера приходят из UI, поэтому отсутствующая
/// страница даёт пустой кадр ([`DrawCommand::Clear`]), а не панику.
///
/// Координаты `layout` — пиксели от левого верхнего угла страницы, координаты
/// команд — физические пиксели canvas (см. модульный комментарий).
pub fn build(layout: &PageLayout, page: usize, viewport: Viewport, out: &mut DisplayList) {
    // Список команд переиспользуется между кадрами: без очистки следующий кадр
    // лёг бы поверх предыдущего.
    out.clear();
    out.push(DrawCommand::Clear);

    // Индекс нужен дважды: страница берётся из `pages`, колонтитул — из
    // `chrome` по тому же номеру.
    let page_index = page;
    let Some(page) = layout.pages.get(page_index) else {
        return;
    };

    let font = out.intern(LAYOUT_FONT_FAMILY);

    // Страница шире окна, и без клипа её содержимое выползало бы на соседние
    // элементы интерфейса за пределами canvas.
    out.push(DrawCommand::PushClip {
        x: 0.0,
        y: 0.0,
        w: viewport.w,
        h: viewport.h,
    });

    let (page_x, page_y) = to_screen(&viewport, 0.0, 0.0);
    fill_rect(
        out,
        page_x,
        page_y,
        page.width * viewport.scale,
        page.height * viewport.scale,
        Color::WHITE,
    );

    draw_items(&page.items, &viewport, font, out);

    // Колонтитулы рисуются после тела: они лежат в полях страницы и с телом
    // не пересекаются, но если колонтитул перерос поле, поверх текста должен
    // остаться он — так же поступает Word, раздвигая полосу набора.
    if let Some(chrome) = layout.chrome.get(page_index) {
        draw_items(&chrome.header, &viewport, font, out);
        draw_items(&chrome.footer, &viewport, font, out);
    }

    out.push(DrawCommand::PopClip);
}

/// Нарисовать элементы в порядке раскладки.
fn draw_items(items: &[LayoutItem], viewport: &Viewport, font: StringRef, out: &mut DisplayList) {
    for item in items {
        match item {
            LayoutItem::Paragraph {
                rect,
                text,
                color,
                font_family,
                size,
                bold,
                italic,
                underline,
                ..
            } => {
                let (x, y) = to_screen(viewport, rect.x, rect.y);
                let text = out.intern(text);
                // Гарнитура строки — та, которой её мерила раскладка; имя
                // приходит из `w:rFonts` через реестр шрифтов. Без имени
                // (ячейка таблицы, пустой абзац) рисует запасное лицо.
                let font = match font_family {
                    Some(family) => out.intern(family),
                    None => font,
                };
                out.push(DrawCommand::Text {
                    x,
                    y,
                    text,
                    font,
                    // Кегль — в тех же физических пикселях, что и координаты:
                    // `half_points_to_px` даёт пиксели раскладки, масштаб их
                    // домножает, как это делает xlsx для шрифта ячейки.
                    size: half_points_to_px(size.value()) * viewport.scale,
                    color: text_color(*color),
                    // Выравнивание уже учтено в `rect.x` (`align_shift` в
                    // раскладке абзаца), поэтому повторно его применять нельзя.
                    align: TextAlign::Left,
                    // `rect.y` — верх строки: движок отсчитывает его от курсора
                    // вниз за вычетом высоты строки и интервала после неё.
                    baseline: TextBaseline::Top,
                    bold: *bold,
                    italic: *italic,
                    underline: *underline,
                });
            }
            LayoutItem::Table { cells, .. } => draw_table(cells, viewport, font, out),
            LayoutItem::Float(float) => match &float.kind {
                FloatKind::Image { image_id, .. } => {
                    draw_image(&float.rect, *image_id, viewport, out);
                }
                // Текстовый блок раскладка ещё не несёт: видно только габариты.
                FloatKind::TextBox { .. } => draw_placeholder(&float.rect, viewport, out),
            },
            LayoutItem::Image { rect, image_id, .. } => {
                draw_image(rect, *image_id, viewport, out);
            }
            // Разрывы — метки раскладки, рисовать нечего.
            LayoutItem::PageBreak | LayoutItem::ColumnBreak => {}
        }
    }
}

/// Нарисовать рисунок: команда `Image`, а без байтов — плейсхолдер.
///
/// Байты приходят из [`crate::Document::media`] через раскладку
/// ([`LayoutItem::Image::image_id`]); сам `DisplayList` картинок не несёт — их
/// регистрирует у painter'а потребитель (так же устроен путь XLSX:
/// `xlsx_image_bytes` → `createImageBitmap` → `register_bitmap`).
fn draw_image(rect: &Rect, image_id: Option<u32>, viewport: &Viewport, out: &mut DisplayList) {
    let Some(bitmap_id) = image_id else {
        // Рисовать нечем (битая или отсутствующая часть): на месте картинки
        // остаются её габариты — их видно и можно сверить с раскладкой.
        draw_placeholder(rect, viewport, out);
        return;
    };
    let (x, y) = to_screen(viewport, rect.x, rect.y);
    out.push(DrawCommand::Image {
        x,
        y,
        w: rect.width * viewport.scale,
        h: rect.height * viewport.scale,
        bitmap_id,
    });
}

/// Плейсхолдер рисунка — прямоугольник с контуром.
fn draw_placeholder(rect: &Rect, viewport: &Viewport, out: &mut DisplayList) {
    let (x, y) = to_screen(viewport, rect.x, rect.y);
    out.push(DrawCommand::Rect {
        x,
        y,
        w: rect.width * viewport.scale,
        h: rect.height * viewport.scale,
        fill: Color::TRANSPARENT,
        stroke: PLACEHOLDER_STROKE,
        stroke_w: PLACEHOLDER_STROKE_W * viewport.scale,
        radius: [0.0; 4],
    });
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

/// Цвет знака: раскладка хранит `w:color` как `RRGGBB`, `DisplayList` ждёт
/// `RRGGBBAA` — каналы меняются местами, альфа добавляется непрозрачная.
///
/// Значение без `w:color` — чёрный: `auto` в OOXML и есть цвет текста по
/// умолчанию.
fn text_color(rgb: Option<u32>) -> Color {
    match rgb {
        Some(rgb) => Color(((rgb & 0x00FF_FFFF) << 8) | 0xFF),
        None => Color::BLACK,
    }
}

/// Точка раскладки → физические пиксели canvas.
fn to_screen(viewport: &Viewport, x: f32, y: f32) -> (f32, f32) {
    let point = viewport.layout_to_screen(Point::new(x, y));
    (point.x, point.y)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use doc_converter_render::display_list::LineStyle;
    use doc_converter_render::font::FontRegistry;

    use std::collections::BTreeMap;

    use doc_converter_core::NodeId;

    use super::*;
    use crate::layout::engine::{TableCellLayout, VAlign, PX_PER_POINT};
    use crate::layout::{layout_document, LayoutOptions, Page};
    use crate::model::{
        BlockItem, Body, Extent, Inline, InlineImage, InlineOrAnchor, Paragraph, Run, RunContent,
    };

    /// Путь к фикстуре от корня репозитория.
    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx")
            .join(name)
    }

    /// Разложить фикстуру; её отсутствие роняет тест, а не пропускает его.
    fn fixture_layout(name: &str) -> PageLayout {
        let path = fixture_path(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|_| panic!("нет фикстуры {}", path.display()));
        let document = crate::open(bytes).expect("фикстура разбирается");
        layout_document(
            &document,
            &LayoutOptions::default(),
            &mut FontRegistry::new(64),
        )
        .expect("раскладка не должна падать")
    }

    /// Окно с прокруткой и зумом: перевод координат виден, а не совпадает с
    /// тождественным.
    fn viewport() -> Viewport {
        Viewport::new(10.0, 20.0, 800.0, 600.0, 2.0)
    }

    /// Собрать кадр страницы.
    fn frame(layout: &PageLayout, page: usize) -> DisplayList {
        let mut out = DisplayList::new();
        build(layout, page, viewport(), &mut out);
        out
    }

    /// Абзацы страницы в порядке раскладки.
    fn paragraphs(page: &Page) -> Vec<&LayoutItem> {
        page.items
            .iter()
            .filter(|item| matches!(item, LayoutItem::Paragraph { .. }))
            .collect()
    }

    /// Текст элемента-абзаца.
    fn paragraph_text(item: &LayoutItem) -> &str {
        match item {
            LayoutItem::Paragraph { text, .. } => text,
            _ => unreachable!("вызывается только для абзаца"),
        }
    }

    /// Команда `Text`, разобранная по полям: так утверждения читаются.
    #[derive(Debug)]
    struct DrawnText {
        text: String,
        font: String,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        align: TextAlign,
        baseline: TextBaseline,
        bold: bool,
    }

    /// Все команды `Text` кадра в порядке рисования.
    fn drawn_texts(dl: &DisplayList) -> Vec<DrawnText> {
        (0..dl.len())
            .filter_map(|i| match dl.cmd(i) {
                Some(DrawCommand::Text {
                    x,
                    y,
                    text,
                    font,
                    size,
                    color,
                    align,
                    baseline,
                    bold,
                    ..
                }) => Some(DrawnText {
                    text: dl.string(*text).to_owned(),
                    font: dl.string(*font).to_owned(),
                    x: *x,
                    y: *y,
                    size: *size,
                    color: *color,
                    align: *align,
                    baseline: *baseline,
                    bold: *bold,
                }),
                _ => None,
            })
            .collect()
    }

    /// Число команд, подходящих под условие.
    fn count(dl: &DisplayList, pred: impl Fn(&DrawCommand) -> bool) -> usize {
        (0..dl.len())
            .filter(|i| dl.cmd(*i).is_some_and(&pred))
            .count()
    }

    /// Команда `Line`, разобранная по полям.
    #[derive(Debug, Clone, Copy)]
    struct DrawnLine {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Color,
        stroke_w: f32,
        style: LineStyle,
    }

    /// Все команды `Line` кадра в порядке рисования.
    fn drawn_lines(dl: &DisplayList) -> Vec<DrawnLine> {
        (0..dl.len())
            .filter_map(|i| match dl.cmd(i) {
                Some(DrawCommand::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke,
                    stroke_w,
                    style,
                }) => Some(DrawnLine {
                    x1: *x1,
                    y1: *y1,
                    x2: *x2,
                    y2: *y2,
                    stroke: *stroke,
                    stroke_w: *stroke_w,
                    style: *style,
                }),
                _ => None,
            })
            .collect()
    }

    /// Ни одна линия не повторяет другую: общее ребро соседей схлопнуто.
    fn unique(lines: &[DrawnLine]) -> bool {
        lines.iter().enumerate().all(|(i, line)| {
            lines[i + 1..]
                .iter()
                .all(|other| !same_segment(line, other))
        })
    }

    /// Отрезки совпадают с точностью до [`assert_px`].
    fn same_segment(a: &DrawnLine, b: &DrawnLine) -> bool {
        let near = |x: f32, y: f32| (x - y).abs() < 0.01;
        near(a.x1, b.x1) && near(a.y1, b.y1) && near(a.x2, b.x2) && near(a.y2, b.y2)
    }

    /// Команды `Image` кадра: `(x, y, w, h, bitmap_id)` в порядке рисования.
    fn drawn_images(dl: &DisplayList) -> Vec<(f32, f32, f32, f32, u32)> {
        (0..dl.len())
            .filter_map(|i| match dl.cmd(i) {
                Some(DrawCommand::Image {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                }) => Some((*x, *y, *w, *h, *bitmap_id)),
                _ => None,
            })
            .collect()
    }

    /// Документ из абзацев — для случаев, которых нет среди фикстур.
    fn document_of(items: Vec<BlockItem>) -> crate::Document {
        crate::Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::new(1),
                items,
                sections: Vec::new(),
            },
            styles: crate::StyleTable::default(),
            numbering: crate::NumberingTable::default(),
            settings: crate::Settings::default(),
            metadata: crate::Metadata::default(),
            rels: crate::Relationships::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            headers: BTreeMap::new(),
            footers: BTreeMap::new(),
            warnings: Vec::new(),
            media: Vec::new(),
        }
    }

    /// Абзац с одним встроенным рисунком; `part` — имя части, если оно известно.
    fn paragraph_with_inline_image(part: Option<&str>) -> BlockItem {
        BlockItem::Paragraph(Paragraph {
            id: NodeId::new(2),
            runs: vec![Inline::Run(Run {
                id: NodeId::new(3),
                content: vec![RunContent::Drawing(InlineOrAnchor {
                    id: NodeId::new(4),
                    inline: Some(InlineImage {
                        id: NodeId::new(5),
                        rel_id: "rIdImg1".to_owned(),
                        part: part.map(str::to_owned),
                        name: None,
                        description: None,
                        extent: Extent {
                            cx: 457_200,
                            cy: 685_800,
                        },
                    }),
                    anchor: None,
                })],
                ..Run::default()
            })],
            ..Paragraph::default()
        })
    }

    /// Прямоугольники заливки кадра.
    fn drawn_rects(dl: &DisplayList) -> Vec<(f32, f32, f32, f32, Color)> {
        (0..dl.len())
            .filter_map(|i| match dl.cmd(i) {
                Some(DrawCommand::Rect {
                    x, y, w, h, fill, ..
                }) => Some((*x, *y, *w, *h, *fill)),
                _ => None,
            })
            .collect()
    }

    /// Ячейки первой таблицы раскладки.
    fn first_table_cells(layout: &PageLayout) -> &[TableCellLayout] {
        layout
            .pages
            .iter()
            .flat_map(|page| page.items.iter())
            .find_map(|item| match item {
                LayoutItem::Table { cells, .. } => Some(cells.as_slice()),
                _ => None,
            })
            .expect("в раскладке есть таблица")
    }

    /// Абзацы среди элементов ячейки.
    fn paragraphs_in(items: &[LayoutItem]) -> impl Iterator<Item = &LayoutItem> {
        items
            .iter()
            .filter(|item| matches!(item, LayoutItem::Paragraph { .. }))
    }

    /// Сравнить координату или размер: `f32` на равенство не проверяем.
    fn assert_px(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{what}: ожидалось {expected} px, получено {actual} px"
        );
    }

    /// Каждая строка абзаца — ровно одна команда `Text`, и она же несёт текст
    /// строки.
    #[test]
    fn every_paragraph_line_becomes_one_text_command() {
        let layout = fixture_layout("simple/multiple_paragraphs_0.docx");
        let dl = frame(&layout, 0);

        let lines = paragraphs(&layout.pages[0]);
        assert!(!lines.is_empty(), "на первой странице есть абзацы");
        assert_eq!(
            drawn_texts(&dl).len(),
            lines.len(),
            "одна команда на строку абзаца"
        );

        let drawn: Vec<String> = drawn_texts(&dl).into_iter().map(|t| t.text).collect();
        let expected: Vec<String> = lines
            .iter()
            .map(|item| paragraph_text(item).to_owned())
            .collect();
        assert_eq!(drawn, expected);
    }

    /// Координаты команды — раскладка, переведённая окном: прокрутка вычитается,
    /// масштаб умножается.
    #[test]
    fn text_is_transformed_into_canvas_pixels() {
        let layout = fixture_layout("simple/one_paragraph.docx");
        let dl = frame(&layout, 0);
        let viewport = viewport();

        let Some(LayoutItem::Paragraph { rect, .. }) = layout.pages[0].items.first() else {
            panic!("первый элемент страницы — абзац");
        };
        let drawn = &drawn_texts(&dl)[0];

        assert_px(drawn.x, (rect.x - viewport.x) * viewport.scale, "x");
        assert_px(drawn.y, (rect.y - viewport.y) * viewport.scale, "y");
        assert_px(
            drawn.size,
            half_points_to_px(22) * viewport.scale,
            "кегль 11 pt в физических пикселях",
        );
    }

    /// `formatting/heading_1.docx`: заголовок рисуется жирным кеглем стиля и
    /// гарнитурой раскладки.
    #[test]
    fn a_heading_is_drawn_bold_and_sized() {
        let layout = fixture_layout("formatting/heading_1.docx");
        let dl = frame(&layout, 0);

        let drawn = &drawn_texts(&dl)[0];
        assert_eq!(drawn.text, "Heading 1");
        assert!(drawn.bold, "жирный из стиля Heading1");
        assert_px(
            drawn.size,
            half_points_to_px(32) * viewport().scale,
            "16 pt",
        );
        // Гарнитуру строка берёт у прогона: заголовок набран Calibri, и это имя
        // уходит в кадр. Лицо то же, что у запасного Carlito: Calibri
        // зарегистрирован алиасом, поэтому ширины сходятся.
        assert_eq!(drawn.font, "Calibri", "гарнитура строки из стиля");
        assert_eq!(drawn.baseline, TextBaseline::Top, "верх строки");
        assert_eq!(drawn.align, TextAlign::Left, "выравнивание уже в rect.x");
    }

    /// Страница выбирается по индексу: вторая страница рисует свои строки и не
    /// несёт строк первой.
    #[test]
    fn the_requested_page_is_the_one_drawn() {
        let layout = fixture_layout("basic/sections.docx");
        assert!(layout.pages.len() >= 2, "фикстура двухстраничная");

        let first: Vec<String> = drawn_texts(&frame(&layout, 0))
            .into_iter()
            .map(|t| t.text)
            .collect();
        let second: Vec<String> = drawn_texts(&frame(&layout, 1))
            .into_iter()
            .map(|t| t.text)
            .collect();

        let expected: Vec<String> = paragraphs(&layout.pages[1])
            .iter()
            .map(|item| paragraph_text(item).to_owned())
            .collect();
        assert!(!expected.is_empty(), "вторая страница не пуста");
        assert_eq!(second, expected);
        assert_ne!(second, first, "страницы рисуют разное");
        assert!(
            !second.contains(&first[0]),
            "строки первой страницы на второй не рисуются"
        );
    }

    /// Номера приходят из UI: страницы за пределами документа дают пустой кадр.
    #[test]
    fn a_missing_page_yields_an_empty_frame() {
        let layout = fixture_layout("simple/one_paragraph.docx");
        assert_eq!(layout.pages.len(), 1, "фикстура одностраничная");

        let dl = frame(&layout, 1);
        assert_eq!(dl.len(), 1, "пустой кадр — только Clear");
        assert!(matches!(dl.cmd(0), Some(DrawCommand::Clear)));

        assert!(
            !drawn_texts(&frame(&layout, 0)).is_empty(),
            "на своей странице текст есть"
        );
    }

    /// `w:color` доезжает до команды как `RRGGBBAA`, а не как `AARRGGBB`.
    #[test]
    fn an_explicit_color_becomes_rrggbbaa() {
        // Первый прогон абзаца зелёный (`w:color 008000`) — его цвет и несёт
        // строка: стиль знака строка берёт у того прогона, который её открыл.
        let layout = fixture_layout("styles/direct_formatting_override.docx");
        let dl = frame(&layout, 0);
        let texts = drawn_texts(&dl);

        let green = texts
            .iter()
            .find(|t| t.text.starts_with("Прямое форматирование"))
            .expect("абзац фикстуры");
        assert_eq!(
            green.color,
            Color::rgba(0x00, 0x80, 0x00, 0xFF),
            "`w:color` доезжает как RGBA, а не AARRGGBB"
        );

        let layout = fixture_layout("basic/multiple_runs.docx");
        let texts = drawn_texts(&frame(&layout, 0));
        let plain = texts
            .iter()
            .find(|t| t.text.starts_with("Plain"))
            .expect("первый run фикстуры");
        assert_eq!(plain.color, Color::BLACK, "без `w:color` — чёрный");
    }

    /// Таблица: границы и текст ячейки.
    #[test]
    fn a_table_draws_cell_borders_and_content() {
        let layout = fixture_layout("tables/simple_2x2.docx");
        let dl = frame(&layout, 0);

        let cells = first_table_cells(&layout);
        assert_eq!(cells.len(), 4, "таблица 2×2");
        let lines = drawn_lines(&dl);
        // Шестнадцать рёбер четырёх ячеек, но общее ребро соседей схлопывается
        // в один отрезок: три горизонтальные границы и три вертикальные, каждая
        // из двух отрезков — по одному на ячейку, между которыми она проходит.
        assert_eq!(lines.len(), 12, "общие рёбра нарисованы по одному разу");
        assert!(
            unique(&lines),
            "ни одно ребро не нарисовано дважды: {lines:?}"
        );
        for line in &lines {
            assert_eq!(line.stroke, Color::BLACK, "`w:color=\"auto\"` — чёрный");
            // `w:sz="4"` — полпункта; толщина масштабируется, как координаты.
            assert_px(
                line.stroke_w,
                4.0 / 8.0 * PX_PER_POINT * viewport().scale,
                "толщина границы",
            );
            assert_eq!(line.style, LineStyle::Solid, "`w:val=\"single\"`");
        }
        assert_eq!(
            drawn_texts(&dl).len(),
            cells
                .iter()
                .map(|cell| paragraphs_in(&cell.content).count())
                .sum::<usize>(),
            "содержимое ячеек нарисовано"
        );
    }

    /// Ячейки лежат в тех же координатах, что и их содержимое: раскладка кладёт
    /// содержимое от верхнего левого угла ячейки, а не в её локальной системе.
    #[test]
    fn cell_content_lands_inside_its_cell() {
        let layout = fixture_layout("tables/simple_2x2.docx");

        for cell in first_table_cells(&layout) {
            for item in &cell.content {
                let LayoutItem::Paragraph { rect, .. } = item else {
                    continue;
                };
                assert!(
                    rect.x + 0.01 >= cell.rect.x
                        && rect.x + rect.width <= cell.rect.x + cell.rect.width + 0.01
                        && rect.y + 0.01 >= cell.rect.y
                        && rect.y + rect.height <= cell.rect.y + cell.rect.height + 0.01,
                    "строка {:?} выходит за ячейку {:?}",
                    rect,
                    cell.rect
                );
            }
        }
    }

    /// Плавающий рисунок рисуется командой `Image` реального размера из
    /// `wp:extent` (914400 EMU = 96 px, значит 120×90 px здесь — 120 и 90).
    #[test]
    fn a_float_draws_the_image_command() {
        let layout = fixture_layout("images/anchor_wrap_square.docx");
        let dl = frame(&layout, 0);

        let images = drawn_images(&dl);
        assert_eq!(images.len(), 1, "ровно одна команда `Image`");
        assert_px(images[0].2, 120.0 * viewport().scale, "ширина");
        assert_px(images[0].3, 90.0 * viewport().scale, "высота");
        assert_eq!(
            count(&dl, |cmd| matches!(
                cmd,
                DrawCommand::Rect {
                    fill: Color::TRANSPARENT,
                    ..
                }
            )),
            0,
            "плейсхолдер рисунку с байтами не нужен"
        );
    }

    /// Встроенные рисунки: каждый — своя команда `Image` размера `wp:extent`
    /// (сайдкар `images/multiple_sizes.json`: 457200×457200 — 48×48 px,
    /// 1371600×685800 — 144×72, 685800×1371600 — 72×144).
    #[test]
    fn inline_images_are_drawn_at_their_extents() {
        let layout = fixture_layout("images/multiple_sizes.docx");
        let dl = frame(&layout, 0);

        let sizes: Vec<(f32, f32)> = drawn_images(&dl)
            .into_iter()
            .map(|(_, _, w, h, _)| (w, h))
            .collect();
        let expected = [(48.0, 48.0), (144.0, 72.0), (72.0, 144.0)];
        assert_eq!(sizes.len(), expected.len(), "по команде на рисунок");
        for ((w, h), (ew, eh)) in sizes.iter().zip(expected) {
            assert_px(*w, ew * viewport().scale, "ширина рисунка");
            assert_px(*h, eh * viewport().scale, "высота рисунка");
        }
        assert!(
            count(&dl, |cmd| matches!(
                cmd,
                DrawCommand::Rect {
                    fill: Color::TRANSPARENT,
                    ..
                }
            )) == 0,
            "габариты рисуются картинкой, а не плейсхолдером"
        );
    }

    /// Рисунок, чьей медиачасти нет (битая или внешняя ссылка), остаётся
    /// плейсхолдером: кадр не падает и не ссылается на чужие байты.
    #[test]
    fn an_image_without_bytes_stays_a_placeholder() {
        let document = document_of(vec![paragraph_with_inline_image(Some(
            "word/media/missing.png",
        ))]);
        let layout = layout_document(
            &document,
            &LayoutOptions::default(),
            &mut FontRegistry::new(64),
        )
        .expect("раскладка не должна падать");
        let dl = frame(&layout, 0);

        assert_eq!(
            drawn_images(&dl).len(),
            0,
            "байтов нет — команды `Image` быть не должно"
        );
        let placeholders: Vec<_> = drawn_rects(&dl)
            .into_iter()
            .filter(|(.., fill)| *fill == Color::TRANSPARENT)
            .collect();
        assert_eq!(placeholders.len(), 1, "габариты видны плейсхолдером");
        assert_px(placeholders[0].2, 48.0 * viewport().scale, "ширина");
        assert_px(placeholders[0].3, 72.0 * viewport().scale, "высота");
    }

    /// Таблицы страницы с глубиной вложенности: у вложенной таблицы глубина
    /// больше нуля.
    fn table_depths(items: &[LayoutItem], depth: usize, out: &mut Vec<(Rect, usize)>) {
        for item in items {
            if let LayoutItem::Table { rect, cells, .. } = item {
                out.push((*rect, depth));
                for cell in cells {
                    table_depths(&cell.content, depth + 1, out);
                }
            }
        }
    }

    /// Абзацы страницы вместе с абзацами ячеек и вложенных таблиц.
    fn paragraphs_on_page(items: &[LayoutItem]) -> usize {
        items
            .iter()
            .map(|item| match item {
                LayoutItem::Paragraph { .. } => 1,
                LayoutItem::Table { cells, .. } => cells
                    .iter()
                    .map(|cell| paragraphs_on_page(&cell.content))
                    .sum(),
                _ => 0,
            })
            .sum()
    }

    /// Заливка ячейки (`w:shd`) — прямоугольник её размера под содержимым.
    #[test]
    fn a_shaded_cell_is_filled_with_its_fill_color() {
        let layout = fixture_layout("tables/borders_shading.docx");
        let dl = frame(&layout, 0);

        // Шапка фикстуры залита `w:shd w:val="clear" w:fill="2F5496"` — так фон
        // ячейки пишет Word, и это единственный цвет заливки в фикстуре.
        let rects = drawn_rects(&dl);
        let shaded: Vec<_> = rects
            .iter()
            .filter(|(_, _, _, _, color)| *color == Color(0x2F54_96FF))
            .collect();
        assert_eq!(shaded.len(), 2, "залиты обе ячейки шапки: {rects:?}");

        let cell = &first_table_cells(&layout)[0];
        let (x, y) = to_screen(&viewport(), cell.rect.x, cell.rect.y);
        assert_px(shaded[0].0, x, "левый край заливки");
        assert_px(shaded[0].1, y, "верхний край заливки");
        assert_px(
            shaded[0].2,
            cell.rect.width * viewport().scale,
            "ширина заливки",
        );
        assert_px(
            shaded[0].3,
            cell.rect.height * viewport().scale,
            "высота заливки",
        );
    }

    /// Границы таблицы: `w:sz` — восьмые доли пункта, цвет — из `w:color`.
    #[test]
    fn a_table_border_uses_its_thickness_and_color() {
        let layout = fixture_layout("tables/borders_shading.docx");
        let dl = frame(&layout, 0);
        let lines = drawn_lines(&dl);

        let blue: Vec<_> = lines
            .iter()
            .filter(|line| line.stroke == Color(0x2F54_96FF))
            .collect();
        assert!(!blue.is_empty(), "границы таблицы нарисованы: {lines:?}");
        for line in blue {
            assert_eq!(line.style, LineStyle::Solid, "`w:val=\"single\"`");
            assert_px(
                line.stroke_w,
                8.0 / 8.0 * PX_PER_POINT * viewport().scale,
                "толщина `w:sz=\"8\"`",
            );
        }
    }

    /// `w:tcBorders` побеждает табличную границу на общем ребре: линия одна и
    /// она тяжелее (`double` ячейки против `single` таблицы).
    #[test]
    fn a_cell_border_wins_over_the_table_border() {
        let layout = fixture_layout("tables/borders_shading.docx");
        let dl = frame(&layout, 0);
        let lines = drawn_lines(&dl);

        let doubles: Vec<_> = lines
            .iter()
            .filter(|line| line.style == LineStyle::Double)
            .collect();
        assert_eq!(doubles.len(), 1, "двойная граница одна: {lines:?}");
        assert_eq!(doubles[0].stroke, Color(0xC000_00FF), "`w:color` двойной");
        assert_px(
            doubles[0].stroke_w,
            6.0 / 8.0 * PX_PER_POINT * viewport().scale,
            "толщина `w:sz=\"6\"`",
        );
    }

    /// Вложенная таблица рисуется внутри ячейки: у неё свои границы.
    #[test]
    fn a_nested_table_draws_its_own_grid() {
        let layout = fixture_layout("tables/nested.docx");
        let dl = frame(&layout, 0);

        let mut tables = Vec::new();
        table_depths(&layout.pages[0].items, 0, &mut tables);
        let (rect, _) = tables
            .iter()
            .find(|(_, depth)| *depth > 0)
            .unwrap_or_else(|| panic!("вложенной таблицы нет: {tables:?}"));

        let (x, y) = to_screen(&viewport(), rect.x, rect.y);
        let (x2, y2) = to_screen(&viewport(), rect.x + rect.width, rect.y + rect.height);
        let inside = drawn_lines(&dl)
            .iter()
            .filter(|line| {
                line.x1 >= x - 0.01
                    && line.x1 <= x2 + 0.01
                    && line.x2 >= x - 0.01
                    && line.x2 <= x2 + 0.01
                    && line.y1 >= y - 0.01
                    && line.y1 <= y2 + 0.01
                    && line.y2 >= y - 0.01
                    && line.y2 <= y2 + 0.01
            })
            .count();
        assert!(inside >= 4, "границы вложенной таблицы: {inside}");
    }

    /// Продолжение `w:vMerge` приносит только геометрию: содержимое объединения
    /// уже нарисовано в его начале, а границы и заливка нужны.
    #[test]
    fn a_merge_continuation_draws_no_extra_content() {
        let layout = fixture_layout("tables/v_merge.docx");
        let dl = frame(&layout, 0);

        let cells = first_table_cells(&layout);
        assert!(
            cells.iter().any(|cell| cell.v_merge_continue),
            "в фикстуре есть продолжение объединения"
        );
        let lines = drawn_lines(&dl);
        assert!(
            !lines.is_empty() && unique(&lines),
            "границы объединённой ячейки нарисованы по одному разу: {lines:?}"
        );
        assert_eq!(
            drawn_texts(&dl).len(),
            paragraphs_on_page(&layout.pages[0].items),
            "содержимое нарисовано один раз — в начале объединения"
        );
    }

    /// Фрагмент таблицы на странице рисуется один раз: ни одна строка не
    /// попадает в кадр дважды, в том числе повторённая строка заголовка.
    #[test]
    fn a_table_fragment_is_drawn_once_per_page() {
        let layout = fixture_layout("tables/table_100_pages.docx");
        assert!(layout.pages.len() > 1, "таблица не помещается на страницу");

        let pages = (0..layout.pages.len().min(10)).chain([layout.pages.len() - 1]);
        for number in pages {
            let dl = frame(&layout, number);
            assert_eq!(
                drawn_texts(&dl).len(),
                paragraphs_on_page(&layout.pages[number].items),
                "страница {}: каждый абзац нарисован один раз",
                number + 1
            );
        }
    }

    /// `w:vAlign` сдвигает содержимое ячейки вниз, а не повторяется рисованием:
    /// строки стоят там, где их положила раскладка.
    #[test]
    fn cell_valign_shifts_the_content_down() {
        let layout = fixture_layout("tables/cell_margins_valign.docx");
        let dl = frame(&layout, 0);
        let texts = drawn_texts(&dl);

        let y = |text: &str| {
            texts
                .iter()
                .find(|drawn| drawn.text == text)
                .unwrap_or_else(|| panic!("нет строки {text:?} в кадре"))
                .y
        };
        let (top, center, bottom) = (y("Сверху"), y("По центру"), y("Снизу"));
        assert!(center > top, "центр ниже верха: {center} > {top}");
        assert!(bottom > center, "низ ниже центра: {bottom} > {center}");

        // Приписывать сдвиг второй раз нельзя: строка ячейки с `vAlign` рисуется
        // ровно там, где её поставила раскладка.
        let cells = first_table_cells(&layout);
        for cell in cells.iter().filter(|cell| cell.v_align != VAlign::Top) {
            for item in &cell.content {
                let LayoutItem::Paragraph { rect, text, .. } = item else {
                    continue;
                };
                let drawn = texts
                    .iter()
                    .find(|drawn| drawn.text == *text)
                    .unwrap_or_else(|| panic!("нет строки {text:?} в кадре"));
                let (_, expected) = to_screen(&viewport(), rect.x, rect.y);
                assert_px(drawn.y, expected, "верх строки ячейки");
            }
        }
    }

    /// Кадр переживает сериализацию: painter получает его из ring'а байтами.
    #[test]
    fn the_frame_survives_serialization() {
        let layout = fixture_layout("simple/multiple_paragraphs_0.docx");
        let dl = frame(&layout, 0);

        let bytes = dl.to_bytes();
        let reader = DisplayList::from_bytes(&bytes).expect("кадр декодируется");
        let decoded = reader
            .iter()
            .collect::<Result<Vec<DrawCommand>, _>>()
            .expect("команды декодируются");

        assert_eq!(decoded.len(), dl.len());
        let decoded_texts = decoded
            .iter()
            .filter(|cmd| matches!(cmd, DrawCommand::Text { .. }))
            .count();
        assert_eq!(decoded_texts, drawn_texts(&dl).len());
    }

    /// Колонтитулы страницы попадают в кадр: canvas и PDF рисуют их из `chrome`.
    #[test]
    fn headers_and_footers_are_drawn_in_the_frame() {
        let layout = fixture_layout("headers_footers/default_header_footer.docx");
        let page = &layout.pages[0];
        let dl = frame(&layout, 0);

        // В потоке страницы колонтитула нет — он приходит только из `chrome`.
        let body: Vec<&str> = paragraphs(page)
            .iter()
            .map(|item| paragraph_text(item))
            .collect();
        assert_eq!(body, ["Body text"]);

        let drawn = drawn_texts(&dl);
        let header = drawn
            .iter()
            .find(|item| item.text == "Default header")
            .expect("верхний колонтитул нарисован");
        let footer = drawn
            .iter()
            .find(|item| item.text == "Default footer")
            .expect("нижний колонтитул нарисован");

        // Колонтитул рисуется в полосе своей линии: верхний — в верхнем поле,
        // нижний — ниже полосы набора.
        let (_, header_y) = to_screen(&viewport(), 0.0, page.margins.header);
        assert_px(header.y, header_y, "верх колонтитула");
        assert!(
            footer.y > to_screen(&viewport(), 0.0, page.height - page.margins.bottom).1,
            "нижний колонтитул ниже полосы набора"
        );
    }
}
