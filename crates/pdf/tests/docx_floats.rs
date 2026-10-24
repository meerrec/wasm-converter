//! Плавающие элементы DOCX (`wp:anchor`) в PDF: рисунок, плейсхолдер и
//! `behindDoc`.
//!
//! Фикстуры — те же из `test-fixtures/docx`, что читает `crates/docx`;
//! координаты берутся у раскладки, а операции страницы разбираются `lopdf` —
//! приёмы те же, что в `tests/docx.rs`. У фикстур слайса медиачасть есть, и
//! анкор занимает свой прямоугольник настоящим рисунком; плейсхолдер остаётся
//! ветке без байтов — её проверяет тот же тест, очистив медиатаблицу модели
//! (`Document::media`).

use std::path::{Path, PathBuf};

use doc_converter_docx::layout::{
    layout_document, FloatElement, LayoutItem, LayoutOptions, PageLayout,
};
use doc_converter_docx::{BlockItem, Document, Inline, Rect, RunContent};
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};
use lopdf::content::Content;
use lopdf::{Document as Pdf, Object};

/// Пикселей раскладки в точке PDF: 96 dpi против 72 dpi.
const PX_PER_POINT: f32 = 96.0 / 72.0;
/// Допуск сверки координат, точки: `f32`-арифметика раскладки против PDF.
const TOLERANCE_PT: f32 = 0.1;

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
        &mut FontRegistry::new(DEFAULT_FONT_CACHE),
    )
    .expect("раскладка собирается")
}

/// Экспортировать документ настройками по умолчанию.
fn export(doc: &Document) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_docx(doc)
        .expect("документ экспортируется")
}

/// Единственный плавающий элемент первой страницы: у фикстур слайса он один.
fn only_float(layout: &PageLayout) -> &FloatElement {
    let floats: Vec<&FloatElement> = layout.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            LayoutItem::Float(float) => Some(float),
            _ => None,
        })
        .collect();
    assert_eq!(floats.len(), 1, "у фикстуры ровно один плавающий элемент");
    floats[0]
}

/// Операции первой страницы собранного PDF.
fn page_ops(bytes: &[u8]) -> Content {
    let pdf = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    let id = *pdf
        .get_pages()
        .values()
        .next()
        .expect("в PDF есть страница");
    let content = pdf.get_page_content(id);
    Content::decode(&content).expect("операции разбираются")
}

/// Индекс первой текстовой операции (`Tj`/`TJ`).
fn first_text_op(content: &Content) -> usize {
    content
        .operations
        .iter()
        .position(|op| op.operator == "Tj" || op.operator == "TJ")
        .expect("в потоке есть текст")
}

/// Индекс обводки контура — единственного пути на страницах фикстур слайса.
fn placeholder_stroke(content: &Content) -> usize {
    content
        .operations
        .iter()
        .position(|op| op.operator == "S")
        .expect("в потоке есть обводка контура")
}

/// Индекс отсечения по прямоугольнику — путь перед `W`.
fn clip_path(content: &Content) -> usize {
    content
        .operations
        .iter()
        .position(|op| op.operator == "W")
        .expect("в потоке есть отсечение")
}

/// Индекс отрисовки плавающего рисунка в потоке: сам `XObject` (`Do`) или
/// обводка плейсхолдера (`S`) — смотря по тому, есть ли у анкора байты.
fn float_drawing(content: &Content) -> usize {
    content
        .operations
        .iter()
        .position(|op| op.operator == "Do" || op.operator == "S")
        .expect("в потоке есть рисунок или его плейсхолдер")
}

/// Вершины пути, нарисованного до операции `stroke`: пары `m`/`l`.
fn path_points(content: &Content, stroke: usize) -> Vec<(f32, f32)> {
    content.operations[..stroke]
        .iter()
        .filter(|op| op.operator == "m" || op.operator == "l")
        .map(|op| (number(&op.operands[0]), number(&op.operands[1])))
        .collect()
}

/// Операнд-число: координаты пишутся как `Real`.
#[allow(clippy::cast_precision_loss)] // координаты страницы далеки от 2^24
fn number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("координата не число: {other:?}"),
    }
}

/// Вершины прямоугольника `rect` в точках PDF.
///
/// Экспорт переводит пиксели 96 dpi в точки и переворачивает `y` (начало
/// координат PDF — левый нижний угол страницы); здесь то же самое
/// арифметикой теста — эталон считается от раскладки, а не подглядывается у
/// экспорта.
fn expected_corners(rect: Rect, page_height_px: f32) -> [(f32, f32); 4] {
    let left = rect.x / PX_PER_POINT;
    let right = (rect.x + rect.width) / PX_PER_POINT;
    let top = (page_height_px - rect.y) / PX_PER_POINT;
    let bottom = (page_height_px - rect.y - rect.height) / PX_PER_POINT;
    [(left, bottom), (right, bottom), (right, top), (left, top)]
}

/// Переключить `behind_text` у всех плавающих анкоров модели; вернуть число
/// изменённых.
fn set_behind_text(doc: &mut Document, behind: bool) -> usize {
    let mut changed = 0;
    for block in &mut doc.body.items {
        let BlockItem::Paragraph(paragraph) = block else {
            continue;
        };
        for inline in &mut paragraph.runs {
            let Inline::Run(run) = inline else {
                continue;
            };
            for content in &mut run.content {
                let RunContent::Drawing(drawing) = content else {
                    continue;
                };
                if let Some(anchor) = drawing.anchor.as_mut() {
                    anchor.behind_text = behind;
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// Рисунок (или его плейсхолдер) встаёт в прямоугольник анкера: вершины контура
/// в PDF совпадают с `FloatElement::rect`, переведённым в точки.
///
/// У фикстур медиачасть есть, поэтому рисуется сам `XObject`, отсечённый по
/// прямоугольнику анкера; ветка без байтов — та же фикстура с очищенной
/// медиатаблицей: рисунку нечего показать, и на его месте остаётся контур.
#[test]
fn float_drawing_lands_on_the_anchor_rect() {
    for name in [
        "images/anchor_wrap_square.docx",
        "images/anchor_behind_text.docx",
    ] {
        let mut doc = document(name);
        let with_media = layout(&doc);
        let float = only_float(&with_media).clone();

        let content = page_ops(&export(&doc));
        assert_corners(
            name,
            &path_points(&content, clip_path(&content)),
            expected_corners(float.rect, with_media.pages[0].height),
        );

        doc.media.clear();
        let without_media = layout(&doc);
        let float = only_float(&without_media).clone();
        let content = page_ops(&export(&doc));
        assert_corners(
            name,
            &path_points(&content, placeholder_stroke(&content)),
            expected_corners(float.rect, without_media.pages[0].height),
        );
    }
}

/// Вершины `points` — те же, что у прямоугольника `expected`.
fn assert_corners(name: &str, points: &[(f32, f32)], expected: [(f32, f32); 4]) {
    assert_eq!(points.len(), expected.len(), "{name}: вершин контура");
    for corner in expected {
        assert!(
            points
                .iter()
                .any(|&(x, y)| (x - corner.0).abs() <= TOLERANCE_PT
                    && (y - corner.1).abs() <= TOLERANCE_PT),
            "{name}: вершины {corner:?} нет среди {points:?}"
        );
    }
}

/// `behindDoc`: рисунок под текстом рисуется до текста, передний — после.
///
/// Только фикстура `anchor_behind_text.docx` несёт и текст, и плавающий
/// рисунок; передний случай получается переключением `behind_text` на той же
/// модели — у остальных фикстур с `Float` текста нет, и порядок в них
/// непроверяем.
#[test]
fn behind_text_draws_before_the_text_and_front_after() {
    let mut doc = document("images/anchor_behind_text.docx");

    let under = page_ops(&export(&doc));
    assert!(
        float_drawing(&under) < first_text_op(&under),
        "рисунок под текстом нарисован после него"
    );

    assert_eq!(
        set_behind_text(&mut doc, false),
        1,
        "в фикстуре ровно один плавающий анкор"
    );
    let over = page_ops(&export(&doc));
    assert!(
        first_text_op(&over) < float_drawing(&over),
        "передний рисунок нарисован до текста"
    );
}
