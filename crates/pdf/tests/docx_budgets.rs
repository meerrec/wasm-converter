//! Бюджет DoD Спринта 10: DOCX на 200 страниц экспортируется в PDF быстрее 2 с.
//!
//! Крупной фикстуры в корпусе нет — самая большая даёт три страницы, — поэтому
//! документ собирается из настоящей фикстуры: тело
//! `simple/multiple_paragraphs_0.docx` повторяется [`COPIES`] раз. Приём
//! опирается только на публичную модель (`Document::body.items`), правок в
//! `crates/docx` не требует. Число страниц берётся у самой раскладки
//! (`layout_document`), а не оценивается: гейт обязан убедиться, что мерит
//! действительно большой документ, а не пустышку. Замеренные числа печатает
//! сам тест (см. [`report`]): строка попадает в лог CI и её можно цитировать в
//! отчёте по спринту, не перезапуская замер.
//!
//! Время экспорта — само число DoD, и порог здесь жёсткий (2 с), в отличие от
//! мягких гейтов `budgets.rs`: те стоят на 3× от бюджета XLSX-экспорта (300
//! мс), потому что 3 с там — потолок самого бюджета. Здесь бюджет и есть число
//! DoD, а запас защищает от шума раннера: замеры 11.10.2026 — 211 страниц,
//! 0,41–0,56 с в debug (≈4×) и 31 мс в release (≈64×). Если запас
//! когда-нибудь исчезнет, понижать порог молча нельзя: это разговор про
//! бюджет, а не про тест.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use doc_converter_docx::layout::{layout_document, LayoutOptions};
use doc_converter_docx::{BlockItem, Document};
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};

/// Бюджет DoD Спринта 10: 200 страниц — быстрее 2 с.
const TIME_BUDGET: Duration = Duration::from_millis(2_000);

/// Пол страниц: документ меньше — уже не тот, о котором говорит DoD.
const MIN_PAGES: usize = 200;

/// Сколько раз повторить тело фикстуры.
///
/// Число подобрано так, чтобы раскладка дала чуть больше [`MIN_PAGES`]:
/// документ «на 200 страниц», а не на тысячу, — иначе гейт мерил бы не то,
/// что обещано. Ожидаемый размер печатает сам тест.
const COPIES: usize = 1_350;

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

/// Тело фикстуры, повторённое `copies` раз.
///
/// Содержимое (абзацы и таблицы) копируется, а `SectPr` остаётся один, в
/// конце: это конец секции, и его дубликаты разрезали бы документ на секции,
/// которых в модели нет (`Document::body.sections` не растёт вместе с телом).
fn replicated(source: &Document, copies: usize) -> Document {
    let (content, tail): (Vec<BlockItem>, Vec<BlockItem>) = source
        .body
        .items
        .iter()
        .cloned()
        .partition(|item| !matches!(item, BlockItem::SectPr(_)));
    assert!(
        !content.is_empty(),
        "в фикстуре нет содержимого для повтора"
    );

    let mut doc = source.clone();
    doc.body.items = content
        .iter()
        .cloned()
        .cycle()
        .take(content.len() * copies)
        .chain(tail)
        .collect();
    doc
}

/// Напечатать строку так, чтобы её было видно и в логе CI.
///
/// Одного `eprintln!` (как в `budgets.rs`) мало: libtest проглатывает вывод
/// прошедшего теста, а `cargo test` в CI идёт без `--nocapture` — числа DoD
/// были бы видны только при падении, то есть ровно тогда, когда они уже не
/// нужны. Поэтому строка идёт в настоящий stderr: `/dev/stderr` — отдельный
/// дескриптор, а не буфер libtest. `eprintln!` остаётся запасным путём —
/// Windows такого файла не знает, — и тест из-за лога не падает.
fn report(line: &str) {
    use std::io::Write;

    match std::fs::OpenOptions::new().write(true).open("/dev/stderr") {
        Ok(mut stderr) => {
            let _ = writeln!(stderr, "{line}");
        }
        Err(_) => eprintln!("{line}"),
    }
}

/// Разложить документ так же, как это сделает экспорт.
fn layout(doc: &Document) -> Vec<doc_converter_docx::layout::Page> {
    layout_document(
        doc,
        &LayoutOptions::default(),
        &mut FontRegistry::new(DEFAULT_FONT_CACHE),
    )
    .expect("раскладка собирается")
    .pages
}

#[test]
fn two_hundred_pages_export_fits_time_budget() {
    let doc = replicated(&document("simple/multiple_paragraphs_0.docx"), COPIES);

    // Раскладка считается до экспорта: без этого ассерта тест «прошёл» бы и на
    // документе в три страницы, а бюджет мерился бы не на том, о чём DoD.
    let pages = layout(&doc).len();
    assert!(
        pages >= MIN_PAGES,
        "документ меньше DoD: {pages} страниц при минимуме {MIN_PAGES}; увеличь COPIES"
    );

    // Первый экспорт — холодный: в него входит раскладка и встраивание шрифтов,
    // ровно то, что увидит пользователь. Меряется он, а не прогретый повтор.
    let started = Instant::now();
    let pdf = PdfExporter::new(PdfOptions::default())
        .export_docx(&doc)
        .expect("документ экспортируется");
    let elapsed = started.elapsed();

    report(&format!(
        "200-страничный DOCX ({COPIES} копий тела): {pages} страниц, экспорт за {:.1} мс, \
         PDF {} байт ({:.1} КиБ), бюджет {TIME_BUDGET:?}",
        elapsed.as_secs_f64() * 1000.0,
        pdf.len(),
        pdf.len() as f64 / 1024.0,
    ));

    assert!(!pdf.is_empty(), "экспорт вернул пустой PDF");
    assert!(pdf.starts_with(b"%PDF"), "PDF не начинается с %PDF");
    assert!(
        elapsed < TIME_BUDGET,
        "экспорт {pages} страниц дольше бюджета DoD {TIME_BUDGET:?}: {elapsed:?}"
    );
}
