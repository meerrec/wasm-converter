//! Таблицы DOCX в PDF: рамки ячеек и текст внутри них.
//!
//! Проверки структурные, теми же приёмами, что у `tests/docx.rs` и
//! `tests/grid.rs`: PDF разбирается `lopdf`, текст восстанавливается картами
//! `ToUnicode`, штрихи берутся из операций потока содержимого. Эталон числа
//! рамок и их координат считается из раскладки (`layout_document`), а не
//! вписывается в тест числами: совпавший с отрисовкой дефект иначе остался бы
//! незамеченным.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_docx::layout::{
    layout_document, LayoutItem, LayoutOptions, Page, PageLayout, TableCellLayout,
};
use doc_converter_docx::Document;
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::FontRegistry;
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

/// Операции всех страниц: `get_page_content` распаковывает FlateDecode.
fn all_ops(bytes: &[u8]) -> Vec<Operation> {
    let doc = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    let mut ops = Vec::new();
    for &page_id in doc.get_pages().values() {
        let bytes = doc.get_page_content(page_id).expect("поток содержимого");
        let content = Content::decode(&bytes).expect("операции разбираются");
        ops.extend(content.operations);
    }
    ops
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
#[derive(Debug, Clone, Copy)]
struct Stroke {
    from: (f32, f32),
    to: (f32, f32),
    rgb: Option<(f32, f32, f32)>,
    width: f32,
}

/// Штрихи потока: пути из двух точек, замкнутые оператором `S`.
///
/// Состояние кисти (`RG`, `w`) переносится на следующие за ним пути: printpdf
/// не повторяет цвет перед каждой линией.
fn strokes(ops: &[Operation]) -> Vec<Stroke> {
    let mut out = Vec::new();
    let mut rgb = None;
    let mut width = 1.0;
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

/// Рёбра ячейки в точках PDF: четыре стороны её прямоугольника.
///
/// Пересчёт повторяет геометрию крейта, но считается в тесте руками: разойдись
/// экспорт и раскладка одинаково, сверка с раскладкой этого не поймала бы.
fn cell_edges(cell: &TableCellLayout, page: &Page) -> [[(f32, f32); 2]; 4] {
    let height_pt = page.height / PX_PER_POINT;
    let x0 = cell.rect.x / PX_PER_POINT;
    let x1 = (cell.rect.x + cell.rect.width) / PX_PER_POINT;
    // У printpdf начало координат в левом нижнем углу, у раскладки — в левом
    // верхнем: `y` прямоугольника — верхняя кромка.
    let y1 = height_pt - cell.rect.y / PX_PER_POINT;
    let y0 = height_pt - (cell.rect.y + cell.rect.height) / PX_PER_POINT;
    [
        [(x0, y0), (x1, y0)],
        [(x1, y0), (x1, y1)],
        [(x1, y1), (x0, y1)],
        [(x0, y1), (x0, y0)],
    ]
}

/// Совпадает ли штрих с ребром: концы — как множество, с допуском.
///
/// Порядок концов в потоке свой, а сложение координат в геометрии крейта и в
/// тесте даёт разные последние разряды (`(x + w) / k` против `x / k + w / k`) —
/// поэтому концы сверяются в любом порядке и с [`EPS`].
fn matches_edge(stroke: &Stroke, edge: [(f32, f32); 2]) -> bool {
    let same = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS;
    (same(stroke.from, edge[0]) && same(stroke.to, edge[1]))
        || (same(stroke.from, edge[1]) && same(stroke.to, edge[0]))
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

/// Рамок ровно по одной на ячейку: четыре стороны на каждую.
///
/// Число штрихов потока сверяется с числом ячеек раскладки, а их координаты —
/// с рёбрами прямоугольников ячеек: рамка обязана обвести ячейку, а не лечь
/// рядом.
#[test]
fn every_cell_border_is_drawn() {
    for name in ["tables/simple_2x2.docx", "complex/text_and_table.docx"] {
        let doc = document(name);
        let layout = layout(&doc);

        let mut cells = 0;
        let mut expected: Vec<[(f32, f32); 2]> = Vec::new();
        for page in &layout.pages {
            let page_cells = cells_of(page);
            cells += page_cells.len();
            for cell in page_cells {
                expected.extend(cell_edges(cell, page));
            }
        }
        assert!(cells > 0, "{name}: в раскладке нет ни одной ячейки");

        let mut drawn = strokes(&all_ops(&export(&doc)));
        assert_eq!(
            drawn.len(),
            4 * cells,
            "{name}: штрихов должно быть по четыре на ячейку"
        );

        // Каждому ребру — свой штрих: порядок в потоке не зафиксирован, а общее
        // ребро соседей рисуется дважды, поэтому сверяется весь набор целиком.
        for edge in &expected {
            let found = drawn
                .iter()
                .position(|stroke| matches_edge(stroke, *edge))
                .unwrap_or_else(|| panic!("{name}: рамки {edge:?} в потоке нет"));
            drawn.remove(found);
        }
        assert!(drawn.is_empty(), "{name}: лишние штрихи: {drawn:?}");
    }
}

/// Рамка ячейки — чёрная, толщиной в один пиксель раскладки (0.75 pt).
#[test]
fn cell_border_is_a_black_pixel() {
    let doc = document("tables/simple_2x2.docx");
    let strokes = strokes(&all_ops(&export(&doc)));
    assert!(!strokes.is_empty(), "рамок нет");

    for stroke in &strokes {
        let rgb = stroke.rgb.expect("у штриха нет цвета");
        for channel in [rgb.0, rgb.1, rgb.2] {
            close(channel, 0.0);
        }
        close(stroke.width, 1.0 / PX_PER_POINT);
    }
}
