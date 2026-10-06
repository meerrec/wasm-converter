//! Замер памяти и времени PDF-экспорта (B4 спринта 6; критерий DoD 8 спринта 7).
//!
//! Пример печатает размеры, число страниц и время; пиковую память процесса
//! снимает внешний `/usr/bin/time -l` (maximum resident set size). Свой код для
//! RSS потребовал бы `unsafe` либо чтения `/proc`, которого на macOS нет:
//!
//!     cargo build --release -p doc-converter-pdf --example mem_probe
//!     /usr/bin/time -l target/release/examples/mem_probe target/fixtures/scale-500-pages.xlsx
//!     /usr/bin/time -l target/release/examples/mem_probe target/fixtures/scale-500-pages.xlsx --sink
//!
//! `--open-only` завершается сразу после разбора книги: разница пиков двух
//! запусков показывает, сколько сверх разобранной книги стоит сам экспорт.
//!
//! `--sink` гоняет путь `export_xlsx_sheet_to` — PDF уходит в приёмник и не
//! буферизуется. Именно им меряется DoD 8 («пик на 500 страницах не больше
//! 1,5× пика на 50»): на `Vec`-пути буфер файла линеен по числу страниц, и
//! критерий недостижим по построению.
//!
//! `--out <путь>` пишет PDF `Vec`-пути в файл — для сравнения содержимого
//! страниц до и после правок конвейера.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Приёмник, считающий байты: столько PDF получили бы файл или сокет.
#[derive(Default)]
struct CountingSink {
    bytes: usize,
}

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

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
        eprintln!("usage: mem_probe <fixture.xlsx | путь> [--open-only] [--sink] [--out <путь>]");
        std::process::exit(2);
    };
    let mut open_only = false;
    let mut sink = false;
    let mut out_path: Option<PathBuf> = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--open-only" => open_only = true,
            "--sink" => sink = true,
            "--out" => out_path = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown flag: {other}");
                std::process::exit(2);
            }
        }
    }

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

    let mut exporter = PdfExporter::new(PdfOptions::default());
    let started = Instant::now();
    if sink {
        let mut sink = CountingSink::default();
        exporter
            .export_xlsx_sheet_to(&book, 0, &mut sink)
            .expect("PDF собирается в sink");
        println!(
            "{}: вход {input} Б, PDF {} Б в sink, страниц не считаем, экспорт {:.1} мс",
            path.display(),
            sink.bytes,
            started.elapsed().as_secs_f64() * 1000.0,
        );
        return;
    }

    let pdf = exporter
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");
    let elapsed = started.elapsed();
    if let Some(out) = &out_path {
        std::fs::write(out, &pdf).unwrap_or_else(|err| panic!("{}: {err}", out.display()));
    }

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
