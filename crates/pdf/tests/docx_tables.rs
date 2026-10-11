//! Таблицы DOCX в PDF: границы, заливка, выравнивание и текст ячеек.
//!
//! Проверки структурные, теми же приёмами, что у `tests/docx.rs` и
//! `tests/grid.rs`: PDF разбирается `lopdf`, текст восстанавливается картами
//! `ToUnicode`, заливки и штрихи берутся из операций потока содержимого.
//! Эталон считается из раскладки (`layout_document`), а не вписывается в тест
//! числами: совпавший с отрисовкой дефект иначе остался бы незамеченным.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use doc_converter_docx::layout::{
    half_points_to_px, layout_document, LayoutItem, LayoutOptions, Page, PageLayout,
    TableCellLayout, VAlign,
};
use doc_converter_docx::{Border, BorderStyle, Color as DocColor, Document, ShadingPattern};
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_ID};
use lopdf::content::{Content, Operation};
use lopdf::Document as Pdf;
use lopdf::Object;

/// Пикселей раскладки в точке PDF: 96 dpi против 72 dpi.
const PX_PER_POINT: f32 = 96.0 / 72.0;
/// Допуск сверки координат и толщины — доли точки.
const EPS: f32 = 1e-3;
/// Размер LRU-кэша метрик — как у крейта (`painter::FONT_CACHE`).
const FONT_CACHE: usize = 4096;

/// Путь к фикстуре в `test-fixtures/docx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/docx")
        .join(name)
}

/// Открыть фикстуру; её отсутствие роняет тест, а не пропускает его.
fn document(name: &str) -> Document {
    let path = fixture(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    doc_converter_docx::open(bytes).unwrap_or_else(|e| panic!("{name} не открылся: {e}"))
}

/// Разложить документ так же, как это сделает экспорт.
fn layout(doc: &Document) -> PageLayout {
    layout_document(
        doc,
        &LayoutOptions::default(),
        &mut FontRegistry::new(FONT_CACHE),
    )
    .expect("раскладка собирается")
}

/// Экспортировать документ настройками по умолчанию.
fn export(doc: &Document) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_docx(doc)
        .expect("документ экспортируется")
}

/// Ячейки всех таблиц страницы в порядке отрисовки.
fn cells_of(page: &Page) -> Vec<&TableCellLayout> {
    page.items
        .iter()
        .filter_map(|item| match item {
            LayoutItem::Table { cells, .. } => Some(cells.iter()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Текст страницы `index` (с нуля), восстановленный картами `ToUnicode`.
fn page_text(bytes: &[u8], index: usize) -> String {
    let pdf = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    let pages = pdf.get_pages();
    let id = *pages
        .values()
        .nth(index)
        .unwrap_or_else(|| panic!("страницы {index} нет"));
    text_runs(&pdf, id)
        .into_iter()
        .map(|(_, run)| run)
        .collect()
}

/// Карта `ToUnicode` каждого шрифта страницы: имя ресурса → «код → символ».
fn page_cmaps(pdf: &Pdf, page: lopdf::ObjectId) -> HashMap<String, HashMap<u16, char>> {
    let mut cmaps = HashMap::new();
    for (name, font) in pdf.get_page_fonts(page).expect("шрифты страницы") {
        let Ok(to_unicode) = font.get(b"ToUnicode") else {
            continue;
        };
        let Ok(id) = to_unicode.as_reference() else {
            continue;
        };
        let Ok(stream) = pdf.get_object(id).and_then(lopdf::Object::as_stream) else {
            continue;
        };
        let Ok(content) = stream.get_plain_content() else {
            continue;
        };
        cmaps.insert(
            String::from_utf8_lossy(&name).into_owned(),
            parse_bfchar(&String::from_utf8_lossy(&content)),
        );
    }
    cmaps
}

/// Пары «код в потоке → символ Unicode» из `beginbfchar`-секции CMap.
fn parse_bfchar(cmap: &str) -> HashMap<u16, char> {
    let mut map = HashMap::new();
    let mut inside = false;
    for line in cmap.lines() {
        let line = line.trim();
        if line.ends_with("beginbfchar") {
            inside = true;
        } else if line == "endbfchar" {
            inside = false;
        } else if inside {
            let Some((source, target)) = line.split_once(' ') else {
                continue;
            };
            let code = u16::from_str_radix(source.trim_matches(['<', '>']), 16);
            let value = u16::from_str_radix(target.trim_matches(['<', '>']), 16);
            if let (Ok(code), Ok(value)) = (code, value) {
                if let Some(ch) = char::from_u32(u32::from(value)) {
                    map.insert(code, ch);
                }
            }
        }
    }
    map
}

/// Прогон текста в потоке содержимого: имя ресурса шрифта и строка.
fn text_runs(pdf: &Pdf, page: lopdf::ObjectId) -> Vec<(String, String)> {
    let cmaps = page_cmaps(pdf, page);
    let content = pdf.get_page_content(page).expect("поток содержимого");
    let text = String::from_utf8_lossy(&content);

    // Операторы `Tf`/`Tj` идут в потоке по порядку; в hex-строке ни `T`, ни
    // `j` встретиться не могут (это не hex-цифры), поэтому слияние позиций
    // токенов даёт настоящую последовательность прогонов.
    let mut events: Vec<(usize, &str)> = Vec::new();
    events.extend(text.match_indices("Tf"));
    events.extend(text.match_indices("Tj"));
    events.sort_by_key(|(at, _)| *at);

    let mut runs = Vec::new();
    let mut font = String::new();
    for (at, token) in events {
        if token == "Tf" {
            if let Some(slash) = text[..at].rfind('/') {
                font = text[slash + 1..at]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
            }
            continue;
        }
        let Some(open) = text[..at].rfind('<') else {
            continue;
        };
        let Some(close) = text[open..at].find('>') else {
            continue;
        };
        let hex = &text[open + 1..open + close];
        let mut decoded = String::new();
        for pair in hex.as_bytes().chunks(4) {
            let Ok(code) = u16::from_str_radix(&String::from_utf8_lossy(pair), 16) else {
                continue;
            };
            if let Some(ch) = cmaps.get(&font).and_then(|map| map.get(&code)) {
                decoded.push(*ch);
            }
        }
        runs.push((font.clone(), decoded));
    }
    runs
}

/// Операции страниц по отдельности, в порядке страниц: `get_page_content`
/// распаковывает FlateDecode.
fn pages_ops(bytes: &[u8]) -> Vec<Vec<Operation>> {
    let doc = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    doc.get_pages()
        .values()
        .map(|&page_id| {
            let bytes = doc.get_page_content(page_id).expect("поток содержимого");
            Content::decode(&bytes)
                .expect("операции разбираются")
                .operations
        })
        .collect()
}

/// Операции всех страниц одним списком.
fn all_ops(bytes: &[u8]) -> Vec<Operation> {
    pages_ops(bytes).into_iter().flatten().collect()
}

/// Число из операнда.
fn number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("координата не число: {other:?}"),
    }
}

/// Отрезок, как он записан в потоке: концы в точках PDF и состояние кисти.
#[derive(Debug, Clone)]
struct Stroke {
    from: (f32, f32),
    to: (f32, f32),
    rgb: Option<(f32, f32, f32)>,
    width: f32,
    /// Шаблон штриха (`d`): пустой — линия сплошная.
    dash: Vec<i64>,
}

/// Штрихи потока: пути из двух точек, замкнутые оператором `S`.
///
/// Состояние кисти (`RG`, `w`, `d`) переносится на следующие за ним пути:
/// printpdf не повторяет цвет перед каждой линией. Полигоны (заливки ячеек и
/// плейсхолдеры плавающих рисунков) длиннее двух точек и сюда не попадают.
fn strokes(ops: &[Operation]) -> Vec<Stroke> {
    let mut out = Vec::new();
    let mut rgb = None;
    let mut width = 1.0;
    let mut dash = Vec::new();
    let mut path: Vec<(f32, f32)> = Vec::new();
    for op in ops {
        match op.operator.as_str() {
            "RG" => {
                rgb = Some((
                    number(&op.operands[0]),
                    number(&op.operands[1]),
                    number(&op.operands[2]),
                ));
            }
            "w" => width = number(&op.operands[0]),
            "d" => {
                dash = match &op.operands[0] {
                    Object::Array(values) => values.iter().map(number).map(|v| v as i64).collect(),
                    _ => Vec::new(),
                };
            }
            "m" => {
                path.clear();
                path.push((number(&op.operands[0]), number(&op.operands[1])));
            }
            "l" => path.push((number(&op.operands[0]), number(&op.operands[1]))),
            "S" => {
                if let [from, to] = path[..] {
                    out.push(Stroke {
                        from,
                        to,
                        rgb,
                        width,
                        dash: dash.clone(),
                    });
                }
                path.clear();
            }
            // Заливки и отсечения закрывают путь без штриха.
            "f" | "f*" | "B" | "B*" | "n" => path.clear(),
            _ => {}
        }
    }
    out
}

/// Залитая фигура потока: вершины пути и цвет `rg`.
#[derive(Debug, Clone)]
struct Fill {
    corners: Vec<(f32, f32)>,
    rgb: Option<(f32, f32, f32)>,
}

/// Заливки потока: путь, законченный оператором `f`/`f*`.
fn fills(ops: &[Operation]) -> Vec<Fill> {
    let mut out = Vec::new();
    let mut rgb = None;
    let mut path: Vec<(f32, f32)> = Vec::new();
    for op in ops {
        match op.operator.as_str() {
            "rg" => {
                rgb = Some((
                    number(&op.operands[0]),
                    number(&op.operands[1]),
                    number(&op.operands[2]),
                ));
            }
            "m" => {
                path.clear();
                path.push((number(&op.operands[0]), number(&op.operands[1])));
            }
            "l" => path.push((number(&op.operands[0]), number(&op.operands[1]))),
            "f" | "f*" => {
                out.push(Fill {
                    corners: path.clone(),
                    rgb,
                });
                path.clear();
            }
            "S" | "B" | "B*" | "n" => path.clear(),
            _ => {}
        }
    }
    out
}

/// Позиции текстового курсора (`Td`) в порядке потока: по одной на строку.
///
/// Каждая строка начинается собственным `BT`, поэтому операнды `Td` —
/// абсолютные координаты её базовой линии (см. `docx::draw_line`).
fn text_cursors(ops: &[Operation]) -> Vec<(f32, f32)> {
    ops.iter()
        .filter(|op| op.operator == "Td")
        .map(|op| (number(&op.operands[0]), number(&op.operands[1])))
        .collect()
}

/// Совпадает ли штрих с ожидаемой линией: концы — как множество, с допуском.
///
/// Порядок концов в потоке свой, а сложение координат в геометрии крейта и в
/// тесте даёт разные последние разряды (`(x + w) / k` против `x / k + w / k`) —
/// поэтому концы сверяются в любом порядке и с [`EPS`].
fn same_point(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
}

/// Совпадают ли два отрезка как множества концов.
fn same_segment(a: ((f32, f32), (f32, f32)), b: ((f32, f32), (f32, f32))) -> bool {
    (same_point(a.0, b.0) && same_point(a.1, b.1)) || (same_point(a.0, b.1) && same_point(a.1, b.0))
}

#[track_caller]
fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < EPS,
        "ожидалось {expected}, получено {actual}"
    );
}

/// Текст, который есть только внутри ячеек, доходит до PDF.
///
/// `tables/simple_2x2.docx` — таблица без абзацев вокруг: любой текст на
/// странице может прийти только из ячеек, поэтому найденные `A1`–`B2` не
/// спутать с текстом тела.
#[test]
fn cell_text_reaches_the_pdf() {
    let name = "tables/simple_2x2.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let text = page_text(&export(&doc), 0);

    assert_eq!(cells_of(&layout.pages[0]).len(), 4, "{name}: ячеек");
    assert!(
        !layout.pages[0]
            .items
            .iter()
            .any(|item| matches!(item, LayoutItem::Paragraph { .. })),
        "{name}: абзацы вне таблицы — текст в PDF уже не доказывает, что он из ячеек"
    );
    for expected in ["A1", "A2", "B1", "B2"] {
        assert!(
            text.contains(expected),
            "текста ячейки {expected:?} нет в PDF: {text:?}"
        );
    }
}

/// Таблица стоит между абзацами и не ломает их порядок.
///
/// `complex/text_and_table.docx`: строка «до», заголовок `H1`/`H2`, строка
/// данных `C1`/`C2` и строка «после» — счёт идёт по прогонам потока, поэтому
/// проверяется и порядок отрисовки.
#[test]
fn table_sits_between_paragraphs() {
    let name = "complex/text_and_table.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let text = page_text(&export(&doc), 0);

    assert_eq!(cells_of(&layout.pages[0]).len(), 4, "{name}: ячеек");
    let mut at = 0;
    for expected in ["Text before", "H1", "H2", "C1", "C2", "Text after"] {
        let found = text[at..]
            .find(expected)
            .unwrap_or_else(|| panic!("{expected:?} нет в PDF после позиции {at}: {text:?}"));
        at += found + expected.len();
    }
}

// ---------------------------------------------------------------------------
// Границы, заливка и выравнивание по вертикали
// ---------------------------------------------------------------------------

/// Во сколько раз укрупняются координаты в ключе склейки рёбер — как у крейта.
const EDGE_QUANT: f32 = 100.0;

/// Ключ ребра: упорядоченные концы, округлённые до сотой доли точки.
type EdgeKey = ((i64, i64), (i64, i64));

/// Ключ совпавших рёбер: у ячейки слева ребро идёт снизу вверх, у соседа
/// справа — сверху вниз.
fn edge_key(from: (f32, f32), to: (f32, f32)) -> EdgeKey {
    let q = |v: f32| (v * EDGE_QUANT).round() as i64;
    let a = (q(from.0), q(from.1));
    let b = (q(to.0), q(to.1));
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Ожидаемая линия границы в точках PDF.
#[derive(Debug, Clone)]
struct ExpectedLine {
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    rgb: (f32, f32, f32),
    /// Штрих `w:val` (`d`-оператор): пустой — сплошная.
    dash: Vec<i64>,
    /// Двойная граница: линий две, по краям полосы `w:sz`.
    double: bool,
}

/// Цвет границы `w:color`: `auto` и отсутствие цвета — чёрный.
fn border_rgb(color: Option<&DocColor>) -> (f32, f32, f32) {
    match color {
        Some(DocColor::Rgb(rgb)) => (
            ((rgb >> 16) & 0xFF) as f32 / 255.0,
            ((rgb >> 8) & 0xFF) as f32 / 255.0,
            (rgb & 0xFF) as f32 / 255.0,
        ),
        _ => (0.0, 0.0, 0.0),
    }
}

/// Толщина границы в точках: `w:sz` — восьмые доли пункта.
fn border_width(border: &Border) -> f32 {
    border.sz.map_or(0.5, |sz| sz as f32 / 8.0)
}

/// Штрих границы по `w:val`.
fn border_dash(style: &BorderStyle) -> Vec<i64> {
    match style {
        BorderStyle::Dotted => vec![1, 2],
        BorderStyle::Dashed | BorderStyle::DashSmallGap | BorderStyle::DashDotStroked => vec![4, 3],
        BorderStyle::DotDash => vec![3, 2, 1, 2],
        BorderStyle::DotDotDash => vec![3, 2, 1, 2, 1, 2],
        _ => Vec::new(),
    }
}

/// Рёбра раскладки: совпавшие по геометрии склеены, из разошедшихся границ
/// осталась более толстая — правило экспорта, повторённое независимо.
///
/// `offset_y` — сдвиг содержимого ячейки по `w:vAlign`: вложенная таблица
/// уезжает вниз вместе с содержимым ячейки-родителя. Рёбра, проходящие через
/// тело ячейки (внутренние границы продолжений вертикального объединения), не
/// входят в набор: в Word их нет.
fn layout_edges(pages: &[Page]) -> BTreeMap<EdgeKey, ExpectedLine> {
    /// Прямоугольник ячейки в точках PDF: левый, верхний, правый, нижний края.
    struct Bounds([f32; 4]);

    impl Bounds {
        fn of(cell: &TableCellLayout, height_pt: f32, offset_y: f32) -> Self {
            let (y, bottom) = (
                (cell.rect.y + offset_y) / PX_PER_POINT,
                (cell.rect.y + offset_y + cell.rect.height) / PX_PER_POINT,
            );
            let (left, right) = (
                cell.rect.x / PX_PER_POINT,
                (cell.rect.x + cell.rect.width) / PX_PER_POINT,
            );
            Self([left, height_pt - y, right, height_pt - bottom])
        }

        /// Точка строго внутри — не на кромке.
        fn covers(&self, (x, y): (f32, f32)) -> bool {
            let [left, top, right, bottom] = self.0;
            x > left + EPS && x < right - EPS && y < top - EPS && y > bottom + EPS
        }
    }

    fn walk(
        items: &[LayoutItem],
        height_pt: f32,
        offset_y: f32,
        edges: &mut BTreeMap<EdgeKey, ExpectedLine>,
    ) {
        for item in items {
            let LayoutItem::Table { cells, .. } = item else {
                continue;
            };
            let bounds: Vec<Bounds> = cells
                .iter()
                .map(|cell| Bounds::of(cell, height_pt, offset_y))
                .collect();
            for cell in cells {
                let [left, top, right, bottom] = Bounds::of(cell, height_pt, offset_y).0;
                for (border, from, to) in [
                    (cell.borders.top.as_ref(), (left, top), (right, top)),
                    (
                        cell.borders.bottom.as_ref(),
                        (left, bottom),
                        (right, bottom),
                    ),
                    (cell.borders.left.as_ref(), (left, bottom), (left, top)),
                    (cell.borders.right.as_ref(), (right, bottom), (right, top)),
                ] {
                    let Some(border) = border else { continue };
                    let midpoint = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                    if bounds.iter().any(|bounds| bounds.covers(midpoint)) {
                        continue;
                    }
                    let double =
                        matches!(border.val, BorderStyle::Double | BorderStyle::DoubleWave);
                    let line = ExpectedLine {
                        from,
                        to,
                        width: border_width(border),
                        rgb: border_rgb(border.color.as_ref()),
                        dash: border_dash(&border.val),
                        double,
                    };
                    match edges.get(&edge_key(from, to)) {
                        Some(existing) if existing.width >= line.width => {}
                        _ => {
                            edges.insert(edge_key(from, to), line);
                        }
                    }
                }
                walk(
                    &cell.content,
                    height_pt,
                    offset_y + v_align_shift(cell),
                    edges,
                );
            }
        }
    }

    let mut edges = BTreeMap::new();
    for page in pages {
        walk(&page.items, page.height / PX_PER_POINT, 0.0, &mut edges);
    }
    edges
}

/// Линии, которые обязана дать раскладка: у двойной границы их две — по краям
/// полосы `w:sz`, каждая в треть её толщины.
fn expected_lines(edges: &BTreeMap<EdgeKey, ExpectedLine>) -> Vec<ExpectedLine> {
    let mut lines = Vec::new();
    for edge in edges.values() {
        let (width, offsets): (f32, &[f32]) = if edge.double {
            (edge.width / 3.0, &[-edge.width / 3.0, edge.width / 3.0])
        } else {
            (edge.width, &[0.0])
        };
        let (dx, dy) = (edge.to.0 - edge.from.0, edge.to.1 - edge.from.1);
        let length = dx.hypot(dy);
        let normal = if length > 0.0 {
            (-dy / length, dx / length)
        } else {
            (0.0, 0.0)
        };
        for offset in offsets {
            lines.push(ExpectedLine {
                from: (
                    edge.from.0 + normal.0 * offset,
                    edge.from.1 + normal.1 * offset,
                ),
                to: (edge.to.0 + normal.0 * offset, edge.to.1 + normal.1 * offset),
                width,
                ..edge.clone()
            });
        }
    }
    lines
}

/// Совпадает ли штрих потока с ожидаемой линией.
fn matches_line(stroke: &Stroke, line: &ExpectedLine) -> bool {
    if !same_segment((stroke.from, stroke.to), (line.from, line.to)) {
        return false;
    }
    if (stroke.width - line.width).abs() >= EPS || stroke.dash != line.dash {
        return false;
    }
    let Some(rgb) = stroke.rgb else {
        return false;
    };
    (rgb.0 - line.rgb.0).abs() < EPS
        && (rgb.1 - line.rgb.1).abs() < EPS
        && (rgb.2 - line.rgb.2).abs() < EPS
}

/// Границы ячеек повторяют `w:tcBorders`/`w:tblBorders`: по одной линии на
/// ребро, толщиной `w:sz`/8 pt, цветом `w:color` и штрихом по `w:val`.
///
/// Набор линий считается из раскладки (вместе с вложенными таблицами), а
/// поток обязан дать ровно его: ни потерянной линии, ни лишней — общее ребро
/// соседей рисуется один раз, а `nil`/`none` не рисуются вовсе.
///
/// Сверка идёт по страницам: фрагменты таблицы на разных страницах начинаются
/// от одной и той же кромки листа, и общий по документу набор склеил бы их
/// рёбра в один ключ (`tables/long_table.docx` начинается на обеих страницах
/// с одной и той же строки), а поток нарисовал бы их дважды.
#[test]
fn borders_come_from_the_layout() {
    for name in [
        "tables/simple_2x2.docx",
        "complex/text_and_table.docx",
        "tables/borders_shading.docx",
        "tables/nested.docx",
        "tables/v_merge.docx",
        "tables/cell_margins_valign.docx",
        "tables/long_table.docx",
    ] {
        let doc = document(name);
        let layout = layout(&doc);
        assert!(
            !layout_edges(&layout.pages).is_empty(),
            "{name}: в раскладке нет границ"
        );

        let pages = pages_ops(&export(&doc));
        assert_eq!(
            pages.len(),
            layout.pages.len(),
            "{name}: страниц в PDF и раскладке"
        );
        for (index, page) in layout.pages.iter().enumerate() {
            let edges = layout_edges(std::slice::from_ref(page));
            let mut drawn = strokes(&pages[index]);
            for line in &expected_lines(&edges) {
                let found = drawn
                    .iter()
                    .position(|stroke| matches_line(stroke, line))
                    .unwrap_or_else(|| panic!("{name}: линии {line:?} в потоке нет"));
                drawn.remove(found);
            }
            assert!(drawn.is_empty(), "{name}: лишние штрихи: {drawn:?}");
        }
    }
}

/// Двойная граница (`w:tcBorders`) — две линии цвета `w:color`, каждая в треть
/// толщины `w:sz`.
///
/// `tables/borders_shading.docx`: нижняя граница ячейки A2 — `double`, `sz=6`
/// (0.75 pt), `C00000`; соседа снизу у неё нет, поэтому склейка её не съедает.
#[test]
fn cell_double_border_draws_two_lines() {
    let name = "tables/borders_shading.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let page = &layout.pages[0];
    let height_pt = page.height / PX_PER_POINT;

    let cell = cells_of(page)
        .into_iter()
        .find(|cell| {
            cell.borders.bottom.as_ref().is_some_and(|border| {
                matches!(border.val, BorderStyle::Double | BorderStyle::DoubleWave)
            })
        })
        .expect("в раскладке нет двойной границы");
    let border = cell.borders.bottom.as_ref().unwrap();
    let (width, edge_y) = (
        border_width(border),
        height_pt - (cell.rect.y + cell.rect.height) / PX_PER_POINT,
    );
    let (x0, x1) = (
        cell.rect.x / PX_PER_POINT,
        (cell.rect.x + cell.rect.width) / PX_PER_POINT,
    );
    let rgb = border_rgb(border.color.as_ref());

    let strokes = strokes(&all_ops(&export(&doc)));
    for offset in [-width / 3.0, width / 3.0] {
        let found = strokes.iter().any(|stroke| {
            let y = edge_y + offset;
            same_point(stroke.from, (x0, y))
                && same_point(stroke.to, (x1, y))
                && (stroke.width - width / 3.0).abs() < EPS
                && stroke.rgb.is_some_and(|actual| {
                    (actual.0 - rgb.0).abs() < EPS
                        && (actual.1 - rgb.1).abs() < EPS
                        && (actual.2 - rgb.2).abs() < EPS
                })
        });
        assert!(
            found,
            "{name}: нет линии двойной границы со сдвигом {offset}"
        );
    }
}

/// Заливка ячейки (`w:shd`) — прямоугольник её площади цветом `w:fill`.
///
/// `w:val="clear"` — это фон `w:fill` без узора (так Word и пишет заливку),
/// `nil` заливки не задаёт; `auto` цвета не задаёт вовсе.
#[test]
fn shading_fills_the_cell_rects() {
    let name = "tables/borders_shading.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let page = &layout.pages[0];
    let height_pt = page.height / PX_PER_POINT;

    let mut expected = Vec::new();
    for cell in cells_of(page) {
        let Some(shading) = &cell.shading else {
            continue;
        };
        if matches!(shading.val, ShadingPattern::Nil) {
            continue;
        }
        let Some(DocColor::Rgb(_)) = shading.fill else {
            continue;
        };
        expected.push((
            [
                (
                    cell.rect.x / PX_PER_POINT,
                    height_pt - (cell.rect.y + cell.rect.height) / PX_PER_POINT,
                ),
                (
                    (cell.rect.x + cell.rect.width) / PX_PER_POINT,
                    height_pt - cell.rect.y / PX_PER_POINT,
                ),
            ],
            border_rgb(shading.fill.as_ref()),
        ));
    }
    assert!(expected.len() >= 3, "{name}: в раскладке мало заливок");

    let mut drawn = fills(&all_ops(&export(&doc)));
    for (rect, rgb) in &expected {
        let found = drawn
            .iter()
            .position(|fill| {
                let (left, right) = (rect[0].0, rect[1].0);
                let (bottom, top) = (rect[0].1, rect[1].1);
                let corners = [(left, bottom), (right, bottom), (right, top), (left, top)];
                fill.corners.len() == corners.len()
                    && corners
                        .iter()
                        .all(|corner| fill.corners.iter().any(|c| same_point(*c, *corner)))
                    && fill.rgb.is_some_and(|actual| {
                        (actual.0 - rgb.0).abs() < EPS
                            && (actual.1 - rgb.1).abs() < EPS
                            && (actual.2 - rgb.2).abs() < EPS
                    })
            })
            .unwrap_or_else(|| panic!("{name}: заливки {rect:?} цветом {rgb:?} нет"));
        drawn.remove(found);
    }
    assert!(drawn.is_empty(), "{name}: лишние заливки: {drawn:?}");
}

/// Сдвиг содержимого ячейки по `w:vAlign` — тот же, что считает экспорт.
fn v_align_shift(cell: &TableCellLayout) -> f32 {
    let mut bounds: Option<(f32, f32)> = None;
    for item in &cell.content {
        let rect = match item {
            LayoutItem::Paragraph { rect, .. }
            | LayoutItem::Table { rect, .. }
            | LayoutItem::Image { rect, .. } => rect,
            LayoutItem::Float(float) => &float.rect,
            LayoutItem::PageBreak | LayoutItem::ColumnBreak => continue,
        };
        let bottom = rect.y + rect.height;
        bounds = Some(match bounds {
            Some((top, low)) => (top.min(rect.y), low.max(bottom)),
            None => (rect.y, bottom),
        });
    }
    let Some((top, bottom)) = bounds else {
        return 0.0;
    };
    let free = (cell.rect.y + cell.rect.height) - bottom - (top - cell.rect.y);
    match cell.v_align {
        VAlign::Top => 0.0,
        VAlign::Center => (free / 2.0).max(0.0),
        VAlign::Bottom => free.max(0.0),
    }
}

/// Абзацы ячеек в порядке отрисовки — вместе с ячейкой, в которой они лежат.
fn cell_paragraphs<'a>(
    items: &'a [LayoutItem],
    out: &mut Vec<(&'a LayoutItem, &'a TableCellLayout)>,
) {
    for item in items {
        let LayoutItem::Table { cells, .. } = item else {
            continue;
        };
        for cell in cells {
            for content in &cell.content {
                if matches!(content, LayoutItem::Paragraph { .. }) {
                    out.push((content, cell));
                }
            }
            cell_paragraphs(&cell.content, out);
        }
    }
}

/// `w:vAlign` сдвигает содержимое ячейки вниз: `center` — на половину
/// свободного места, `bottom` — на всё свободное.
///
/// `tables/cell_margins_valign.docx`: в первой строке три ячейки с одинаковой
/// геометрией и `top`/`center`/`bottom` — базовая линия каждой строки сверяется
/// с раскладкой (подъём берётся из метрик того же шрифта, каким мерила
/// раскладка), во второй — ячейки с несимметричными полями `w:tcMar`.
#[test]
fn v_align_moves_content_down() {
    let name = "tables/cell_margins_valign.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let page = &layout.pages[0];
    let height_pt = page.height / PX_PER_POINT;
    let mut registry = FontRegistry::new(FONT_CACHE);

    let mut paragraphs = Vec::new();
    cell_paragraphs(&page.items, &mut paragraphs);
    let cursors = text_cursors(&all_ops(&export(&doc)));
    assert_eq!(
        cursors.len(),
        paragraphs.len(),
        "{name}: базовых линий в потоке столько же, сколько абзацев в ячейках"
    );

    let mut shifted = 0;
    for ((item, cell), (x, y)) in paragraphs.iter().zip(&cursors) {
        let LayoutItem::Paragraph { rect, size, .. } = item else {
            unreachable!()
        };
        let shift = v_align_shift(cell);
        if shift > 0.0 {
            shifted += 1;
        }
        close(*x, rect.x / PX_PER_POINT);
        let ascent = registry
            .measure(DEFAULT_FONT_ID, half_points_to_px(size.value()), "")
            .ascent;
        close(*y, height_pt - (rect.y + shift + ascent) / PX_PER_POINT);
    }
    assert!(
        shifted >= 2,
        "{name}: ни одна ячейка не сдвинута по w:vAlign"
    );
}

/// Вложенная таблица рисуется как обычная: её рёбра лежат внутри ячейки.
///
/// `tables/nested.docx`: таблица в первой ячейке внешней таблицы.
#[test]
fn nested_table_is_drawn_inside_the_cell() {
    let name = "tables/nested.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let page = &layout.pages[0];
    let height_pt = page.height / PX_PER_POINT;

    let mut nested = Vec::new();
    for cell in cells_of(page) {
        for item in &cell.content {
            if let LayoutItem::Table { rect, .. } = item {
                nested.push((rect, &cell.rect, cell));
            }
        }
    }
    assert!(
        !nested.is_empty(),
        "{name}: вложенной таблицы в раскладке нет"
    );

    let strokes = strokes(&all_ops(&export(&doc)));
    for (rect, parent, cell) in nested {
        assert!(
            rect.x >= parent.x - EPS
                && rect.y >= parent.y - EPS
                && rect.x + rect.width <= parent.x + parent.width + EPS
                && rect.y + rect.height <= parent.y + parent.height + EPS,
            "{name}: вложенная таблица {rect:?} вышла за ячейку {parent:?}"
        );
        // Вложенная таблица уезжает вниз вместе с содержимым ячейки
        // (`w:vAlign`), поэтому её рёбра сверяются со сдвигом.
        let rect = &doc_converter_docx::layout::Rect::new(
            rect.x,
            rect.y + v_align_shift(cell),
            rect.width,
            rect.height,
        );
        let (x0, x1) = (rect.x / PX_PER_POINT, (rect.x + rect.width) / PX_PER_POINT);
        let (top, bottom) = (
            height_pt - rect.y / PX_PER_POINT,
            height_pt - (rect.y + rect.height) / PX_PER_POINT,
        );
        // Контур вложенной таблицы обводится по ячейкам: соседние ячейки лишь
        // касаются кромками, а схлопываются только перекрывающиеся рёбра
        // (см. `docx::render::tables::draw_borders`), поэтому сторона — не одна
        // линия, а цепочка отрезков, и сверяется покрытие стороны целиком.
        for (what, horizontal, fixed, start, end) in [
            ("верхняя", true, top, x0, x1),
            ("нижняя", true, bottom, x0, x1),
            ("левая", false, x0, bottom, top),
            ("правая", false, x1, bottom, top),
        ] {
            let mut spans: Vec<(f32, f32)> = strokes
                .iter()
                .filter_map(|stroke| {
                    let (fixed_a, fixed_b, along_a, along_b) = if horizontal {
                        (stroke.from.1, stroke.to.1, stroke.from.0, stroke.to.0)
                    } else {
                        (stroke.from.0, stroke.to.0, stroke.from.1, stroke.to.1)
                    };
                    if (fixed_a - fixed).abs() >= EPS || (fixed_b - fixed).abs() >= EPS {
                        return None;
                    }
                    let (a, b) = (
                        along_a.min(along_b).max(start),
                        along_a.max(along_b).min(end),
                    );
                    (b - a > EPS).then_some((a, b))
                })
                .collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut covered = start;
            for (a, b) in spans {
                if a <= covered + EPS {
                    covered = covered.max(b);
                }
            }
            assert!(
                covered >= end - EPS,
                "{name}: {what} сторона вложенной таблицы прорисована до {covered}, а должна до {end}"
            );
        }
    }
}

/// Внутри вертикально объединённой ячейки горизонтальных линий нет.
///
/// `tables/v_merge.docx`: ячейка `A1` объединяет три строки, поэтому её
/// прямоугольник выше остальных; `insideH`-границы её продолжений — нарезка
/// раскладки по строкам, а не линии Word.
#[test]
fn merged_cell_has_no_internal_lines() {
    let name = "tables/v_merge.docx";
    let doc = document(name);
    let layout = layout(&doc);
    let page = &layout.pages[0];
    let height_pt = page.height / PX_PER_POINT;

    let cells = cells_of(page);
    let merged = cells
        .iter()
        .max_by(|a, b| a.rect.height.total_cmp(&b.rect.height))
        .expect("в раскладке нет ячеек");
    assert!(
        cells.iter().any(|cell| cell.v_merge_continue),
        "{name}: нет продолжений объединения"
    );

    let (left, right) = (
        merged.rect.x / PX_PER_POINT,
        (merged.rect.x + merged.rect.width) / PX_PER_POINT,
    );
    let (bottom, top) = (
        height_pt - (merged.rect.y + merged.rect.height) / PX_PER_POINT,
        height_pt - merged.rect.y / PX_PER_POINT,
    );
    for stroke in strokes(&all_ops(&export(&doc))) {
        let x = (stroke.from.0 + stroke.to.0) / 2.0;
        let y = (stroke.from.1 + stroke.to.1) / 2.0;
        assert!(
            !(x > left + EPS && x < right - EPS && y > bottom + EPS && y < top - EPS),
            "{name}: линия {stroke:?} проходит через объединённую ячейку"
        );
    }
}

/// Текст начала вертикального объединения не дублируется на его продолжениях.
#[test]
fn v_merge_continuation_repeats_no_text() {
    let name = "tables/v_merge.docx";
    let doc = document(name);
    let layout = layout(&doc);

    let mut continues = 0;
    let mut restart_text = String::new();
    for cell in cells_of(&layout.pages[0]) {
        if cell.v_merge_continue {
            continues += 1;
            assert!(
                cell.content.is_empty(),
                "{name}: в продолжении объединения есть содержимое"
            );
        } else {
            for item in &cell.content {
                if let LayoutItem::Paragraph { text, .. } = item {
                    restart_text.push_str(text);
                }
            }
        }
    }
    assert!(
        continues >= 2,
        "{name}: продолжений объединения меньше двух"
    );
    assert!(
        !restart_text.is_empty(),
        "{name}: начало объединения без текста"
    );

    let text = page_text(&export(&doc), 0);
    assert_eq!(
        text.matches(&restart_text).count(),
        1,
        "{name}: текст объединённой ячейки повторён: {text:?}"
    );
}
