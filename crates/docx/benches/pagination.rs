//! Постраничная раскладка DOCX с таблицами.
//!
//! Бюджет DoD Спринта 9 — «Pagination 100 страниц < 500 мс» — до сих пор был
//! декларацией: крупная фикстура (`tables/table_100_pages.docx`, ~100 страниц
//! A4 при печати по умолчанию) лежит в репозитории, а мерит по ней этот бенч.
//! Раскладка меряется отдельно от разбора: разбор в бюджет не входит, и
//! смешивать их значило бы показывать число, к делу не относящееся.
//!
//!     cargo bench -p doc-converter-docx --bench pagination
//!
//! Медиана печатается строкой — так же, как у бенча `open`, — чтобы её можно
//! было взять из лога, не перезапуская замер.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use doc_converter_core::ZipLimits;
use doc_converter_docx::layout::{layout_document, LayoutOptions, PageLayout};
use doc_converter_docx::{parse_docx, Document};
use doc_converter_render::font::FontRegistry;

/// Бюджет DoD Спринта 9: раскладка 100 страниц — быстрее 500 мс.
const BUDGET: Duration = Duration::from_millis(500);

/// Фикстура оракула пагинации: таблица на ~100 страниц.
const HUNDRED_PAGES: &str = "tables/table_100_pages.docx";

/// Малая фикстура: 60 строк, заведомо больше страницы — видно поведение
/// раскладки на таблице, где страница пересекается несколько раз.
const LONG_TABLE: &str = "tables/long_table.docx";

/// Размер LRU-кэша метрик — как у крейта и как в `crates/pdf/tests/docx_budgets.rs`.
const FONT_CACHE: usize = 4096;

/// Прогонов ручного замера медианы — как в бенче `open`. В release раскладка
/// сотни страниц идёт десятками миллисекунд, в debug — сотнями; сотня прогонов
/// criterion растянула бы бенч на минуты.
const MEDIAN_RUNS: usize = 5;

/// Путь к коммитируемой фикстуре.
fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/docx")
        .join(name)
}

/// Байты фикстуры. Отсутствие файла — падение, а не пропуск кейса: молча
/// пропущенный замер выглядит как зелёный прогон, а числа бюджета никто не
/// увидит.
fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "{}: {err} — фикстуры пишет `npx tsx scripts/generate_docx_fixtures.ts`",
            path.display()
        )
    })
}

/// Один разбор фикстуры теми же лимитами, что и у `open`.
fn parse(bytes: &[u8]) -> Document {
    parse_docx(bytes, ZipLimits::default())
        .expect("фикстура разбирается в пределах ZipLimits::default()")
}

/// Раскладка документа; ошибка роняет бенч, а не превращается в замер `Err`.
fn layout(document: &Document, fonts: &mut FontRegistry) -> PageLayout {
    layout_document(document, &LayoutOptions::default(), fonts)
        .expect("раскладка фикстуры проходит")
}

/// Медиана `MEDIAN_RUNS` раскладок в целых миллисекундах (`open.rs`).
fn median_layout_ms(document: &Document, fonts: &mut FontRegistry) -> u64 {
    let mut times: Vec<Duration> = (0..MEDIAN_RUNS)
        .map(|_| {
            let started = Instant::now();
            black_box(layout(document, fonts));
            started.elapsed()
        })
        .collect();
    times.sort_unstable();
    (times[MEDIAN_RUNS / 2].as_secs_f64() * 1000.0).round() as u64
}

/// Разбор и раскладка 100 страниц вместе — то, что видит вызывающий, открывший
/// файл: бюджет относится к раскладке, но вместе с разбором видно её долю.
fn bench_pagination_hundred_pages(c: &mut Criterion) {
    let bytes = read_fixture(HUNDRED_PAGES);
    let document = parse(&bytes);
    let mut fonts = FontRegistry::new(FONT_CACHE);
    let pages = layout(&document, &mut fonts).pages.len();

    let mut group = c.benchmark_group("pagination");
    // Раскладка сотни страниц идёт сотнями миллисекунд; десяти сэмплов хватает
    // для порядка числа (как в `open` на 50 МиБ).
    group.sample_size(10);
    group.throughput(Throughput::Elements(pages as u64));
    // Разбор внутри цикла: это честная цена «открыть и разложить», и парные
    // байты читаются с диска один раз, до цикла.
    group.bench_function("parse_and_layout/table_100_pages", |b| {
        b.iter(|| {
            let document = parse(black_box(&bytes));
            black_box(layout(&document, &mut fonts))
        });
    });
    group.bench_function("layout_only/table_100_pages", |b| {
        b.iter(|| black_box(layout(&document, &mut fonts)));
    });
    group.finish();

    let median = median_layout_ms(&document, &mut fonts);
    let budget_ok = u128::from(median) * 1_000_000 < BUDGET.as_nanos();
    println!(
        "медиана раскладки 100 страниц: {median} мс (страниц {pages}, бюджет {} мс, {})",
        BUDGET.as_millis(),
        if budget_ok {
            "в бюджете"
        } else {
            "сверх бюджета"
        },
    );
}

/// Раскладка длинной, но небольшой таблицы: 60 строк — несколько страниц.
fn bench_pagination_long_table(c: &mut Criterion) {
    let bytes = read_fixture(LONG_TABLE);
    let document = parse(&bytes);
    let mut fonts = FontRegistry::new(FONT_CACHE);
    let pages = layout(&document, &mut fonts).pages.len();

    let mut group = c.benchmark_group("pagination_long_table");
    group.bench_function("layout_only/long_table", |b| {
        b.iter(|| black_box(layout(&document, &mut fonts)));
    });
    group.finish();

    let median = median_layout_ms(&document, &mut fonts);
    println!("{LONG_TABLE}: {pages} страниц, медиана раскладки {median} мс");
}

criterion_group!(
    benches,
    bench_pagination_long_table,
    bench_pagination_hundred_pages
);
criterion_main!(benches);
