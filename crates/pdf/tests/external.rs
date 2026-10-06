//! Внешние проверки экспортированных PDF теми же утилитами, что и приёмка
//! спринта: `qpdf --check` и `pdftotext` (poppler).
//!
//! Локально утилит может не быть — тогда тест печатает причину и
//! пропускается; в CI они ставятся шагом «pdf tooling» (только x86_64), и
//! тесты выполняются по-настоящему. `#[ignore]` не используется: пропуск
//! решает наличие утилиты, а не список задач.

use std::path::{Path, PathBuf};
use std::process::Command;

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Экспортировать лист 0 книги во временный файл, вернуть путь.
fn export_to_temp(name: &str) -> PathBuf {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}")),
    )
    .unwrap_or_else(|err| panic!("{name} не открылась: {err}"));
    let pdf = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .unwrap_or_else(|err| panic!("{name} не экспортировалась: {err}"));

    let path =
        std::env::temp_dir().join(format!("doc-converter-{}-{name}.pdf", std::process::id()));
    std::fs::write(&path, pdf).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    path
}

/// Есть ли утилита в `PATH`.
///
/// Проверяется только запуск процесса: у `pdftotext` нет длинной формы
/// `--version`, и ненулевой код возврата на незнакомый флаг ещё не значит, что
/// утилиты нет. Спавн не удался — единственная причина пропуска.
fn tool_available(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

/// `qpdf --check` не находит ни ошибок, ни предупреждений: код возврата 0,
/// в stderr нет слова «error».
#[test]
fn qpdf_check_accepts_exported_pdfs() {
    if !tool_available("qpdf") {
        eprintln!("пропуск: qpdf не найден в PATH (в CI ставится шагом «pdf tooling»)");
        return;
    }

    for name in ["scale-ten-pages.xlsx", "text-cyrillic-wrap.xlsx"] {
        let path = export_to_temp(name);
        let output = Command::new("qpdf")
            .arg("--check")
            .arg(&path)
            .output()
            .expect("qpdf запускается");
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        std::fs::remove_file(&path).expect("временный PDF удаляется");

        assert!(
            output.status.success(),
            "qpdf --check вернул {} для {name}: {stderr}",
            output.status
        );
        assert!(
            !stderr.to_lowercase().contains("error"),
            "qpdf --check сообщил об ошибке в {name}: {stderr}"
        );
    }
}

/// `pdftotext` возвращает исходную кириллицу: DoD «Кириллица корректна».
#[test]
fn pdftotext_extracts_cyrillic() {
    if !tool_available("pdftotext") {
        eprintln!("пропуск: pdftotext не найден в PATH (в CI ставится шагом «pdf tooling»)");
        return;
    }

    let path = export_to_temp("text-cyrillic-wrap.xlsx");
    // `-` — вывод в stdout: pdftotext отдаёт текст в UTF-8 по картам ToUnicode.
    let output = Command::new("pdftotext")
        .arg(&path)
        .arg("-")
        .output()
        .expect("pdftotext запускается");
    std::fs::remove_file(&path).expect("временный PDF удаляется");
    assert!(
        output.status.success(),
        "pdftotext вернул {}",
        output.status
    );

    let text = String::from_utf8_lossy(&output.stdout);
    for needle in ["Привет, мир", "ё", "Ђ", "№"] {
        assert!(
            text.contains(needle),
            "pdftotext не вернул {needle:?}; извлечённый текст: {text:?}"
        );
    }
}
