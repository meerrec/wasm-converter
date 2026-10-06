//! Диаграммы в PDF: вектор, а не картинка (DoD 4 спринта).
//!
//! Фикстура `charts-five-kinds` несёт пять диаграмм — по одной на каждый вид
//! (`crates/xlsx/tests/charts.rs` разбирает те же книги). Проверка
//! структурная: поток содержимого разбирается `lopdf`, внутри клипов диаграмм
//! ищутся векторные операторы (`m`/`l`/`c` — печать полигонов printpdf), а
//! `/Do`, которым рисуются XObject'ы (в том числе растровые картинки), не
//! должно быть нигде.
//!
//! Клип диаграммы — единственный клип в потоке: картинки в PDF ещё не
//! перенесены, а `chart::draw` оборачивает клип в `q`/`Q` последним слоем
//! страницы.

use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};
use lopdf::content::{Content, Operation};
use lopdf::Document;

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Экспортировать лист книги настройками по умолчанию.
fn export(name: &str, sheet: usize) -> Vec<u8> {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
    )
    .unwrap_or_else(|e| panic!("{name} не открылась: {e}"));
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, sheet)
        .unwrap_or_else(|e| panic!("{name} не экспортировалась: {e}"))
}

/// Операции всех страниц: `get_page_content` распаковывает FlateDecode.
fn all_ops(doc: &Document) -> Vec<Operation> {
    let mut ops = Vec::new();
    for &page_id in doc.get_pages().values() {
        let bytes = doc.get_page_content(page_id).expect("поток содержимого");
        let content = Content::decode(&bytes).expect("операции разбираются");
        ops.extend(content.operations);
    }
    ops
}

/// Диапазоны операций внутри пар `q … Q`, открытых клипом.
///
/// Клип сериализуется как `q  m … l … h  W n` (см. `polygon_to_stream_ops`
/// в вендоренном printpdf), за `n` идут примитивы, закрывает блок `Q`.
/// Диапазон — операции между `n` и `Q`. Клипы вкладываются друг в друга
/// штабелем: здесь он один, но сканер считает вложенность, чтобы не спутать
/// закрывающий `Q` внутреннего блока с внешним.
fn clip_blocks(ops: &[Operation]) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match op.operator.as_str() {
            "q" => stack.push(i),
            "Q" => {
                if let Some(q) = stack.pop() {
                    if let Some(pos) = open.iter().position(|&(open_q, _)| open_q == q) {
                        let (_, start) = open.remove(pos);
                        blocks.push((start, i));
                    }
                }
            }
            "W" | "W*" => {
                let ends_path = ops.get(i + 1).is_some_and(|next| next.operator == "n");
                if ends_path {
                    if let Some(&q) = stack.last() {
                        open.push((q, i + 2));
                    }
                }
            }
            _ => {}
        }
    }
    blocks
}

/// Строит ли оператор путь.
fn builds_path(operator: &str) -> bool {
    matches!(operator, "m" | "l" | "c" | "re")
}

/// Закрашивает ли оператор построенный путь.
fn paints_path(operator: &str) -> bool {
    matches!(operator, "f" | "f*" | "B" | "B*" | "S" | "s")
}

/// В области диаграммы должны быть векторные операторы и ни одного `/Do`.
fn assert_vector_block(ops: &[Operation], bounds: (usize, usize)) {
    let block = &ops[bounds.0..bounds.1];
    let builds = block.iter().filter(|op| builds_path(&op.operator)).count();
    let paints = block.iter().filter(|op| paints_path(&op.operator)).count();
    assert!(
        builds >= 2,
        "в клипе диаграммы {bounds:?} нет построения пути: {block:?}"
    );
    assert!(
        paints >= 1,
        "в клипе диаграммы {bounds:?} путь не закрашен: {block:?}"
    );
    for op in block {
        assert_ne!(
            op.operator.as_str(),
            "Do",
            "в области диаграммы {bounds:?} рисуется XObject — картинка, а не вектор"
        );
    }
}

/// Пять видов диаграмм доходят до страницы векторными примитивами.
#[test]
fn chart_is_vector_not_image() {
    let bytes = export("charts-five-kinds.xlsx", 0);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let ops = all_ops(&doc);

    let blocks = clip_blocks(&ops);
    assert_eq!(
        blocks.len(),
        5,
        "пять диаграмм — пять клипов, нашли {}",
        blocks.len()
    );
    for &bounds in &blocks {
        assert_vector_block(&ops, bounds);
    }

    // Ни одна страница не рисует XObject: диаграмма не могла обойти `chart::draw`.
    for op in &ops {
        assert_ne!(
            op.operator.as_str(),
            "Do",
            "в документе есть /Do — растровая вставка вместо вектора"
        );
    }
    assert!(
        ops.iter().any(|op| op.operator == "c"),
        "сектор круговой диаграммы должен рисоваться кривыми Безье"
    );
}

/// Лист без диаграмм экспортируется как прежде: без клипов и `/Do`.
#[test]
fn sheet_without_charts_gets_no_chart_operators() {
    let bytes = export("content-mixed-types.xlsx", 0);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let ops = all_ops(&doc);

    assert!(!ops.is_empty(), "лист с данными не может быть пустым");
    assert!(
        clip_blocks(&ops).is_empty(),
        "клип в PDF ставит только диаграмма — их на листе нет"
    );
    for op in &ops {
        assert_ne!(op.operator.as_str(), "Do");
    }
}
