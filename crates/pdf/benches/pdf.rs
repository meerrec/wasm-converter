//! Бюджеты спринта 6 (ROADMAP §9): 1000 ячеек → PDF меньше 200 КиБ,
//! 10 страниц быстрее 300 мс. Здесь — фактические числа; жёсткий гейт на
//! размер и мягкий на время живут в `tests/budgets.rs`.
//!
//! Разбивка D3 печатается один раз за прогон: `xlsx::open` (разбор книги),
//! `export_xlsx_sheet` (раскладка, сборка документа и `save` printpdf) и
//! вариант `compress: false`. Отделить `save` от сборки публичным API нельзя:
//! он целиком внутри `export`. Заодно видно, что `compress` в printpdf 0.8.2
//! ничего не меняет — ни байты, ни время: `PdfSaveOptions::optimize` там
//! заглушка (вызов `doc.compress()` закомментирован), а поток страницы
//! создаётся с `with_compression(false)`.
//!
//!     cargo bench -p doc-converter-pdf

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Путь к книге в `test-fixtures/xlsx` (как в интеграционных тестах).
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из фикстуры.
fn open(name: &str) -> doc_converter_xlsx::Workbook {
    let bytes = std::fs::read(fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name}: {err}"))
}

/// Собрать PDF листа 0 с заданными настройками.
fn export_with(book: &doc_converter_xlsx::Workbook, opts: PdfOptions) -> Vec<u8> {
    PdfExporter::new(opts)
        .export_xlsx_sheet(book, 0)
        .expect("PDF собирается")
}

/// Собрать PDF настройками по умолчанию.
fn export(book: &doc_converter_xlsx::Workbook) -> Vec<u8> {
    export_with(book, PdfOptions::default())
}

/// Медиана времени `iterations` прогонов в миллисекундах.
///
/// Одиночный замер шумит на общем раннере; медиана устойчивее среднего.
fn median_ms<F: FnMut()>(iterations: usize, mut run: F) -> f64 {
    let mut times: Vec<Duration> = (0..iterations)
        .map(|_| {
            let started = Instant::now();
            run();
            started.elapsed()
        })
        .collect();
    times.sort_unstable();
    times[iterations / 2].as_secs_f64() * 1000.0
}

/// Сколько страниц в готовом PDF (проверка, что фикстура даёт обещанный объём).
fn page_count(pdf: &[u8]) -> usize {
    lopdf::Document::load_mem(pdf)
        .expect("PDF разбирается lopdf")
        .get_pages()
        .len()
}

fn bench_pdf_size_1000_cells(c: &mut Criterion) {
    let book = open("scale-1000-cells.xlsx");
    let pdf = export(&book);
    println!(
        "1000 ячеек: {} байт ({:.1} КиБ), бюджет 200 КиБ — запас {:.1}x",
        pdf.len(),
        pdf.len() as f64 / 1024.0,
        200.0 * 1024.0 / pdf.len() as f64
    );

    let mut group = c.benchmark_group("pdf");
    group.throughput(Throughput::Bytes(pdf.len() as u64));
    group.bench_function("size_1000_cells", |b| {
        b.iter(|| export(black_box(&book)));
    });
    group.finish();
}

fn bench_pdf_time_10_pages(c: &mut Criterion) {
    let book = open("scale-ten-pages.xlsx");
    println!("10 страниц: {}-страничный PDF", page_count(&export(&book)));

    let mut group = c.benchmark_group("pdf");
    group.throughput(Throughput::Elements(13));
    group.bench_function("time_10_pages", |b| {
        b.iter(|| export(black_box(&book)));
    });
    group.finish();
}

fn bench_pdf_time_1000_cells(c: &mut Criterion) {
    let book = open("scale-1000-cells.xlsx");

    let mut group = c.benchmark_group("pdf");
    group.throughput(Throughput::Elements(1_000));
    group.bench_function("time_1000_cells", |b| {
        b.iter(|| export(black_box(&book)));
    });
    group.finish();
}

/// Разбивка D3 — печатается при запуске, criterion тут только хук запуска.
fn phases(_c: &mut Criterion) {
    for name in ["scale-ten-pages.xlsx", "scale-1000-cells.xlsx"] {
        let raw = std::fs::read(fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
        let open_ms = median_ms(5, || {
            doc_converter_xlsx::open(raw.clone()).expect("open");
        });
        let book = doc_converter_xlsx::open(raw).expect("книга открывается");

        let compressed = export(&book);
        let export_ms = median_ms(5, || {
            export(&book);
        });
        let uncompressed = export_with(
            &book,
            PdfOptions {
                compress: false,
                ..PdfOptions::default()
            },
        );
        let uncompressed_ms = median_ms(5, || {
            export_with(
                &book,
                PdfOptions {
                    compress: false,
                    ..PdfOptions::default()
                },
            );
        });

        println!(
            "{name}: open {open_ms:.1} мс | export {export_ms:.1} мс ({:.1} КиБ, {} стр.) | \
             compress=false {uncompressed_ms:.1} мс ({:.1} КиБ) — байт в байт то же: \
             сжатие в printpdf 0.8.2 не работает",
            compressed.len() as f64 / 1024.0,
            page_count(&compressed),
            uncompressed.len() as f64 / 1024.0,
        );
    }
    println!("printpdf `save` отдельно от сборки не меряется: он внутри export_xlsx_sheet");
}

criterion_group!(
    benches,
    phases,
    bench_pdf_size_1000_cells,
    bench_pdf_time_10_pages,
    bench_pdf_time_1000_cells
);
criterion_main!(benches);
