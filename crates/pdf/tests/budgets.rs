//! Производительные гейты спринта 6: 1000 ячеек → PDF меньше 200 КиБ,
//! 10 страниц быстрее 300 мс (ROADMAP §9).
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

/// Мягкий порог времени: 3× от бюджета 300 мс.
const TIME_SOFT_GATE: Duration = Duration::from_millis(900);

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
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
