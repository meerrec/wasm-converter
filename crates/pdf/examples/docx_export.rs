//! Число страниц и PDF документа DOCX — вход эталонного оракула
//! (`scripts/oracle-docx-pages.sh`).
//!
//! Пример печатает число страниц раскладки, время раскладки и время экспорта и
//! (по `--out`) пишет PDF. PDF печатает LibreOffice, SSIM считает эталонный
//! скрипт — здесь только измеряемый образец:
//!
//!     cargo build --release -p doc-converter-pdf --example docx_export
//!     target/release/examples/docx_export test-fixtures/docx/tables/simple_2x2.docx
//!     target/release/examples/docx_export basic/multi_page.docx --out /tmp/our.pdf
//!
//! Формат строки — контракт с `scripts/oracle-docx-pages.sh` (скрипт разбирает
//! её, а не текст для человека):
//!
//! ```text
//! <путь>: страниц <N>, раскладка <X.XX> мс, PDF <Y.YY> мс
//! ```
//!
//! `N` — `layout.pages.len()`: сколько страниц вернул движок раскладки, столько
//! же будет в PDF (`PdfExporter::export_docx` берёт страницы у раскладки, а не
//! делит свою), поэтому сверять с эталоном по числу страниц достаточно одной
//! величины.
//!
//! «Раскладка» — один вызов `layout_document` ради `N`; «PDF» — `export_docx`
//! целиком, а он раскладывает документ ещё раз (`crates/pdf/src/docx/mod.rs`).
//! Время раскладки поэтому входит в PDF-время дважды — читать оба числа как
//! независимые части одной работы нельзя, см. в шапке выше.
//!
//! Коды выхода: 0 — строка напечатана (низкий SSIM у оракула — его исход, не
//! наш); 1 — разбор, раскладка или экспорт не удались, причина в stderr;
//! 2 — неверное употребление. Причина (а не паника) — потому что сломанные
//! фикстуры для оракула рядовые: скрипт отличает «документ не открылся» от
//! «утилита не нашлась» по коду, а по панике — не отличал бы.

use std::path::{Path, PathBuf};
use std::time::Instant;

use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};

/// Аргумент — путь до `.docx`; если файла по такому пути нет, он ищется в
/// `test-fixtures/docx`, затем в `target/fixtures/docx-large` (крупные фикстуры
/// генератора, в репозитории их нет) — как в `docx::mem_probe_docx`.
fn resolve(arg: &str) -> PathBuf {
    let given = Path::new(arg);
    if given.is_file() {
        return given.to_path_buf();
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = root.join("test-fixtures/docx").join(arg);
    if repo.is_file() {
        return repo;
    }
    root.join("target/fixtures/docx-large").join(arg)
}

fn main() {
    let mut input: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            other if other.starts_with("--") => {
                eprintln!("unknown flag: {other}");
                std::process::exit(2);
            }
            other if input.is_none() => input = Some(other.to_owned()),
            other => {
                eprintln!("unexpected argument: {other} (путь задаётся один раз)");
                std::process::exit(2);
            }
        }
    }
    let Some(input) = input else {
        eprintln!("usage: docx_export <fixture.docx | путь> [--out <путь PDF>]");
        std::process::exit(2);
    };

    let path = resolve(&input);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| fail(&path, &err.to_string()));
    let document =
        doc_converter_docx::open(bytes).unwrap_or_else(|err| fail(&path, &err.to_string()));

    // Раскладка — источник числа страниц; экспорт ниже раскладывает документ
    // ещё раз, поэтому обе величины времени в контракте стоят рядом, а не
    // складываются в «одно измерение».
    let mut fonts = FontRegistry::new(DEFAULT_FONT_CACHE);
    let started = Instant::now();
    let layout = doc_converter_docx::layout::layout_document(
        &document,
        &doc_converter_docx::LayoutOptions::default(),
        &mut fonts,
    )
    .unwrap_or_else(|err| fail(&path, &err.to_string()));
    let layout_ms = started.elapsed().as_secs_f64() * 1000.0;

    let mut exporter = PdfExporter::new(PdfOptions::default());
    let started = Instant::now();
    let pdf = exporter
        .export_docx(&document)
        .unwrap_or_else(|err| fail(&path, &err.to_string()));
    let export_ms = started.elapsed().as_secs_f64() * 1000.0;

    if let Some(out) = out {
        if let Err(err) = std::fs::write(&out, &pdf) {
            fail(&out, &err.to_string());
        }
    }

    println!(
        "{}: страниц {}, раскладка {:.2} мс, PDF {:.2} мс",
        path.display(),
        layout.pages.len(),
        layout_ms,
        export_ms,
    );
}

/// Сообщить причину в stderr и выйти кодом 1: скрипт отличает сбой документа от
/// сбоя окружения по коду, а не по тексту.
fn fail(path: &Path, message: &str) -> ! {
    eprintln!("{}: {message}", path.display());
    std::process::exit(1);
}
