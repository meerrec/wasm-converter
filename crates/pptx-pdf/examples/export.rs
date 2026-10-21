//! Экспорт PPTX в PDF одним файлом — точка входа для оракула и ручной проверки.
//!
//! Запуск: `cargo run --release -p doc-converter-pptx-pdf --example export
//! <input.pptx> <out.pdf>`.
//!
//! Работает как нативный аналог wasm-экспорта `export_pptx_pdf`: те же крейты
//! `doc-converter-pptx` (разбор) и `doc-converter-pptx-pdf` (экспортёр), те же
//! умолчания `PdfOptions` — эталонный вывод для сравнения с LibreOffice.

use std::path::PathBuf;

use doc_converter_pdf::PdfOptions;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .expect("usage: export <input.pptx> <out.pdf>");
    let output = args
        .next()
        .map(PathBuf::from)
        .expect("usage: export <input.pptx> <out.pdf>");

    let bytes = std::fs::read(&input).unwrap_or_else(|err| panic!("{input:?}: {err}"));
    let presentation =
        doc_converter_pptx::open(bytes).unwrap_or_else(|err| panic!("{input:?}: {err}"));
    let pdf = doc_converter_pptx_pdf::export(&presentation, &PdfOptions::default())
        .unwrap_or_else(|err| panic!("export {input:?}: {err}"));
    std::fs::write(&output, pdf).unwrap_or_else(|err| panic!("{output:?}: {err}"));
    println!("wrote {}", output.display());
}
