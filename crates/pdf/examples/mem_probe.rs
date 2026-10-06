//! Замер памяти PDF-экспорта (пункт B4 спринта 6).
//!
//! Пример печатает размеры и число страниц; пиковую память процесса снимает
//! внешний `/usr/bin/time -l` (maximum resident set size). Свой код для RSS
//! потребовал бы `unsafe` либо чтения `/proc`, которого на macOS нет:
//!
//!     cargo build --release -p doc-converter-pdf --example mem_probe
//!     /usr/bin/time -l target/release/examples/mem_probe scale-ten-pages.xlsx
//!
//! Флаг `--open-only` завершается сразу после разбора книги: разница пиков
//! двух запусков показывает, сколько сверх разобранной книги стоит сам экспорт.

use std::path::{Path, PathBuf};
use std::time::Instant;

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Аргумент — имя фикстуры из `test-fixtures/xlsx`; если файл по указанному
/// пути существует, берётся он (так мериются разовые книги вне репозитория).
fn resolve(arg: &str) -> PathBuf {
    let given = Path::new(arg);
    if given.is_file() {
        return given.to_path_buf();
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(arg)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(arg) = args.next() else {
        eprintln!("usage: mem_probe <fixture.xlsx | путь> [--open-only]");
        std::process::exit(2);
    };
    let open_only = args.next().as_deref() == Some("--open-only");

    let path = resolve(&arg);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let input = bytes.len();
    let book =
        doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));

    if open_only {
        println!(
            "{}: вход {input} Б, книга разобрана, экспорта не было",
            path.display()
        );
        return;
    }

    let started = Instant::now();
    let pdf = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");
    let elapsed = started.elapsed();

    let pages = lopdf::Document::load_mem(&pdf)
        .expect("PDF разбирается lopdf")
        .get_pages()
        .len();

    println!(
        "{}: вход {input} Б, PDF {} Б ({:.1} КиБ), страниц {pages}, экспорт {:.1} мс",
        path.display(),
        pdf.len(),
        pdf.len() as f64 / 1024.0,
        elapsed.as_secs_f64() * 1000.0,
    );
}
