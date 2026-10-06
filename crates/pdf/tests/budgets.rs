//! Производительные гейты спринта 6: 1000 ячеек → PDF меньше 200 КиБ,
//! 10 страниц быстрее 300 мс (ROADMAP §9). Спринт 7 добавляет бюджет размера
//! сжатого PDF (DoD 9): плотная книга 2000 ячеек — меньше 32 КиБ, — и мягкий
//! гейт времени на 500 страницах (DoD 1): 9 с = 3× от бюджета 3 с. Тяжёлая
//! фикстура в git не лежит (её пишет `pnpm gen:fixtures` в `target/fixtures`),
//! поэтому тест без неё пропускается с причиной, а не падает.
//!
//! Размер PDF детерминирован при фиксированных фикстурах и настройках, поэтому
//! гейт жёсткий. Время в CI нестабильно — общий раннер, разогрев, соседние
//! джобы, — поэтому гейт мягкий: 900 мс = 3× бюджета. Жёсткие 300 мс ловили бы
//! шум, а не регрессии; фактические числа фиксируют `cargo bench -p
//! doc-converter-pdf` и отчёт спринта, а 300 мс остаются целью, за которой
//! следит человек.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Бюджет размера из ROADMAP §9: 200 КиБ.
const SIZE_BUDGET_BYTES: usize = 200 * 1024;

/// Бюджет размера сжатого PDF (DoD 9): плотная книга 2000 ячеек — 32 КиБ.
///
/// Замер 06.10.2026: `content-dense.xlsx` — 14 510 Б со сжатием против
/// 211 559 Б без него. Порог в 2,2× от замера ловит откат любой из трёх правок
/// форка (без них файл возвращается к сырым ≈211 КиБ) и оставляет запас на рост
/// фикстуры. Факт и степень сжатия (фильтры, отношение ≥1,5×) проверяет
/// `compression.rs`; здесь — только абсолютный потолок, как у гейта 1000 ячеек.
const COMPRESSED_SIZE_BUDGET_BYTES: usize = 32 * 1024;

/// Мягкий порог времени: 3× от бюджета 300 мс.
const TIME_SOFT_GATE: Duration = Duration::from_millis(900);

/// Мягкий порог времени тяжёлого экспорта: 3× от бюджета DoD 1 (3000 мс).
///
/// Порог мягкий по той же причине, что и 900 мс выше: жёсткие 3 с на общем
/// раннере ловили бы разогрев, соседние джобы и планировщик, а не регрессию
/// (в Спринте 6 ровно так жёсткие 300 мс заменили мягкими 900 —
/// `docs/sprint-6/epic-d-report.md:43-44`). Цель DoD 1 — 3 с, медиану
/// фиксирует `cargo bench -p doc-converter-pdf -- bench_pdf_time_500_pages`.
const HEAVY_TIME_SOFT_GATE: Duration = Duration::from_millis(9_000);

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Тяжёлая фикстура (500 страниц): в git её нет, генератор пишет её в
/// `target/fixtures` — книга на 24 000 строк весит сотни килобайт, а ценность
/// её в размере, не в разборе. `test-fixtures/xlsx` проверяется первым: книга,
/// положенная туда руками, тоже найдётся. `None` — фикстуры нет нигде, гейт
/// пропускается (как `external.rs` без `qpdf`), а не падает.
fn heavy_fixture(name: &str) -> Option<PathBuf> {
    let direct = fixture(name);
    if direct.is_file() {
        return Some(direct);
    }
    let heavy = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures")
        .join(name);
    heavy.is_file().then_some(heavy)
}

/// Открыть книгу из фикстуры.
fn open(name: &str) -> doc_converter_xlsx::Workbook {
    let bytes = std::fs::read(fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name} не открылась: {err}"))
}

#[test]
fn thousand_cells_pdf_fits_under_200_kib() {
    let book = open("scale-1000-cells.xlsx");
    let pdf = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");

    eprintln!(
        "scale-1000-cells.xlsx: PDF {} байт ({:.1} КиБ), бюджет {SIZE_BUDGET_BYTES} байт",
        pdf.len(),
        pdf.len() as f64 / 1024.0,
    );
    assert!(
        pdf.len() < SIZE_BUDGET_BYTES,
        "PDF из 1000 ячеек не влез в бюджет: {} байт ({:.1} КиБ) при пороге {SIZE_BUDGET_BYTES} байт",
        pdf.len(),
        pdf.len() as f64 / 1024.0,
    );
}

#[test]
fn content_dense_compressed_pdf_fits_under_32_kib() {
    let book = open("content-dense.xlsx");
    // `compress: true` задан явно, а не взят из дефолта: гейт обязан ломаться,
    // если сжатие выключат, а не молча мерить несжатый файл.
    let pdf = PdfExporter::new(PdfOptions {
        compress: true,
        ..PdfOptions::default()
    })
    .export_xlsx_sheet(&book, 0)
    .expect("PDF собирается");

    eprintln!(
        "content-dense.xlsx: PDF со сжатием {} байт ({:.1} КиБ), бюджет {COMPRESSED_SIZE_BUDGET_BYTES} байт",
        pdf.len(),
        pdf.len() as f64 / 1024.0,
    );
    assert!(
        pdf.len() < COMPRESSED_SIZE_BUDGET_BYTES,
        "сжатый PDF плотной книги не влез в бюджет: {} байт ({:.1} КиБ) при пороге {COMPRESSED_SIZE_BUDGET_BYTES} байт",
        pdf.len(),
        pdf.len() as f64 / 1024.0,
    );
}

#[test]
fn ten_pages_export_fits_soft_time_gate() {
    let book = open("scale-ten-pages.xlsx");

    // Первый экспорт — холодный: в него входит разбор и встраивание шрифтов,
    // ровно то, что увидит пользователь. Меряется он, а не прогретый повтор.
    let started = Instant::now();
    let pdf = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");
    let elapsed = started.elapsed();

    eprintln!(
        "scale-ten-pages.xlsx: экспорт за {:.1} мс, PDF {} байт, мягкий порог {TIME_SOFT_GATE:?}",
        elapsed.as_secs_f64() * 1000.0,
        pdf.len(),
    );
    assert!(
        elapsed < TIME_SOFT_GATE,
        "экспорт 13 страниц дольше мягкого порога {TIME_SOFT_GATE:?}: {elapsed:?}"
    );
}

/// DoD 1: 500 страниц быстрее 3 с — на деле 9 с, см. `HEAVY_TIME_SOFT_GATE`.
///
/// Меряется первый (холодный) экспорт, как у десяти страниц: в него входит
/// разбор книги и встраивание шрифтов. Медиану без порога печатает
/// `cargo bench -p doc-converter-pdf -- bench_pdf_time_500_pages`; здесь —
/// гейт, чтобы регрессии ловил `cargo test`, а не человек с бенчем.
#[test]
fn five_hundred_pages_export_fits_soft_time_gate() {
    let Some(path) = heavy_fixture("scale-500-pages.xlsx") else {
        eprintln!(
            "scale-500-pages.xlsx нет ни в test-fixtures/xlsx, ни в target/fixtures — \
             гейт пропущен; фикстуру собирает `pnpm gen:fixtures`"
        );
        return;
    };
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let book = doc_converter_xlsx::open(bytes)
        .unwrap_or_else(|err| panic!("{} не открылась: {err}", path.display()));

    let started = Instant::now();
    let pdf = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");
    let elapsed = started.elapsed();

    eprintln!(
        "scale-500-pages.xlsx: экспорт за {:.1} мс, PDF {} байт, мягкий порог {HEAVY_TIME_SOFT_GATE:?}",
        elapsed.as_secs_f64() * 1000.0,
        pdf.len(),
    );
    assert!(
        elapsed < HEAVY_TIME_SOFT_GATE,
        "экспорт 500 страниц дольше мягкого порога {HEAVY_TIME_SOFT_GATE:?}: {elapsed:?}"
    );
}
