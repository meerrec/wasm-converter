//! Бюджеты спринта 6 (ROADMAP §9): 1000 ячеек → PDF меньше 200 КиБ,
//! 10 страниц быстрее 300 мс. Здесь — фактические числа; жёсткий гейт на
//! размер и мягкий на время живут в `tests/budgets.rs`.
//!
//! Разбивка D3 печатается один раз за прогон: `xlsx::open` (разбор книги),
//! `export_xlsx_sheet` (раскладка, сборка документа и `save` printpdf) и
//! вариант `compress: false`. Отделить `save` от сборки публичным API нельзя:
//! он целиком внутри `export`. Сжатие несёт вендоренный форк printpdf
//! (ADR-0010), поэтому `compress: false` — база сравнения: на `content-dense`
//! сжатие даёт ≈14,5×. Бенч `size_compressed` (DoD 9) печатает оба числа
//! и множитель.
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

/// Тяжёлая фикстура (500 страниц) в репозитории не лежит: генератор пишет её
/// в `target/fixtures/` — книга на 23 000 строк растёт в git, а её ценность в
/// размере, не в разборе (см. `scripts/gen-fixtures.ts`). `test-fixtures/xlsx`
/// проверяется первым: книга, положенная туда руками, тоже найдётся.
fn heavy_fixture(name: &str) -> PathBuf {
    let direct = fixture(name);
    if direct.is_file() {
        return direct;
    }
    let heavy = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures")
        .join(name);
    assert!(
        heavy.is_file(),
        "{} нет — сначала `pnpm gen:fixtures` (или `node scripts/gen-fixtures.ts`)",
        heavy.display()
    );
    heavy
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

/// DoD 9 (спринт 7): выигрыш сжатия на плотной книге (2000 ячеек) — там, где
/// поток содержимого окупает `FlateDecode`. Порог «≥1,5×» проверяет
/// `tests/compression.rs`; здесь — фактические числа для отчёта.
fn bench_pdf_size_compressed(c: &mut Criterion) {
    let book = open("content-dense.xlsx");
    let packed = export(&book);
    let raw = export_with(
        &book,
        PdfOptions {
            compress: false,
            ..PdfOptions::default()
        },
    );
    println!(
        "content-dense.xlsx: {} байт без сжатия, {} байт со сжатием ({:.1}×)",
        raw.len(),
        packed.len(),
        raw.len() as f64 / packed.len() as f64
    );

    let mut group = c.benchmark_group("pdf");
    group.throughput(Throughput::Bytes(packed.len() as u64));
    group.bench_function("size_compressed", |b| {
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

/// DoD 1: 500 страниц быстрее 3 с. Печатается медиана пяти прогонов: criterion
/// считает выборочное среднее и печатает интервал, а гейту нужно одно число,
/// устойчивое к шуму общего раннера.
///
/// `assert` числа страниц — контракт фикстуры, а не свойство бенча: DoD 1
/// требует ровно 500 страниц, столько должен давать генератор
/// (`scripts/gen-fixtures.ts`; при 48 строках на A4 это 24 000 строк).
/// Порог не понижается под фактический вывод: фикстура, отставшая от
/// контракта (23 000 строк — 480 страниц), обязана уронить бенч, иначе замер
/// молча превращается в замер другой книги.
fn bench_pdf_time_500_pages(c: &mut Criterion) {
    let path = heavy_fixture("scale-500-pages.xlsx");
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let input_kib = bytes.len() as f64 / 1024.0;
    let book =
        doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let pages = page_count(&export(&book));
    assert!(
        pages >= 500,
        "фикстура дала {pages} страниц — DoD 1 требует не меньше 500; \
         генератор должен писать 24 000 строк (48 строк на A4)"
    );
    let median = median_ms(5, || {
        export(&book);
    });
    println!(
        "500 страниц: {pages} стр., вход {input_kib:.0} КиБ, медиана экспорта {median:.0} мс \
         (порог 3000 мс)"
    );

    let mut group = c.benchmark_group("pdf");
    group.throughput(Throughput::Elements(pages as u64));
    // Прогон — сотни миллисекунд; сотня сэмплов criterion растянула бы бенч
    // на минуты, десяти хватает для порядка числа.
    group.sample_size(10);
    group.bench_function("bench_pdf_time_500_pages", |b| {
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
             compress=false {uncompressed_ms:.1} мс ({:.1} КиБ) — сжатый PDF в {:.1}× меньше",
            compressed.len() as f64 / 1024.0,
            page_count(&compressed),
            uncompressed.len() as f64 / 1024.0,
            uncompressed.len() as f64 / compressed.len() as f64,
        );
    }
    println!("printpdf `save` отдельно от сборки не меряется: он внутри export_xlsx_sheet");
}

criterion_group!(
    benches,
    phases,
    bench_pdf_size_compressed,
    bench_pdf_size_1000_cells,
    bench_pdf_time_10_pages,
    bench_pdf_time_1000_cells,
    bench_pdf_time_500_pages
);
criterion_main!(benches);
