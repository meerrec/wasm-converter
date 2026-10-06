//! Внешние проверки экспортированных PDF теми же утилитами, что и приёмка
//! спринта: `qpdf --check` и `pdftotext` (poppler).
//!
//! Локально утилит может не быть — тогда тест печатает причину и
//! пропускается; в CI они ставятся шагом «pdf tooling» (только x86_64), и
//! тесты выполняются по-настоящему. `#[ignore]` не используется: пропуск
//! решает наличие утилиты, а не список задач.
//!
//! Структуру PDF разбирают внутренние тесты (`tests/export.rs`,
//! `tests/annot.rs`, `tests/image.rs`); здесь проверяет чужая реализация —
//! то, что не зависит от нашего понимания формата.

use std::path::{Path, PathBuf};
use std::process::Command;

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из общего набора фикстур.
fn open_fixture(name: &str) -> doc_converter_xlsx::Workbook {
    doc_converter_xlsx::open(
        std::fs::read(fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}")),
    )
    .unwrap_or_else(|err| panic!("{name} не открылась: {err}"))
}

/// Записать PDF во временный файл `doc-converter-<pid>-<tag>.pdf`.
///
/// Имя содержит pid: тесты идут параллельно в одном процессе, а `tag`
/// различает не только книги, но и разные сборки одной книги.
fn temp_pdf(tag: &str, pdf: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("doc-converter-{}-{tag}.pdf", std::process::id()));
    std::fs::write(&path, pdf).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    path
}

/// Экспортировать лист 0 книги с заданными настройками во временный файл.
fn export_sheet_to_temp(tag: &str, name: &str, options: PdfOptions) -> PathBuf {
    let book = open_fixture(name);
    let pdf = PdfExporter::new(options)
        .export_xlsx_sheet(&book, 0)
        .unwrap_or_else(|err| panic!("{name} не экспортировалась: {err}"));
    temp_pdf(tag, &pdf)
}

/// Экспортировать книгу целиком (закладки — по настройкам) во временный файл.
fn export_book_to_temp(tag: &str, name: &str, options: PdfOptions) -> PathBuf {
    let book = open_fixture(name);
    let pdf = PdfExporter::new(options)
        .export_xlsx_book(&book)
        .unwrap_or_else(|err| panic!("{name} не экспортировалась: {err}"));
    temp_pdf(tag, &pdf)
}

/// Экспортировать лист 0 книги настройками по умолчанию, вернуть путь.
fn export_to_temp(name: &str) -> PathBuf {
    export_sheet_to_temp(name, name, PdfOptions::default())
}

/// Есть ли утилита в `PATH`.
///
/// Проверяется только запуск процесса: у `pdftotext` нет длинной формы
/// `--version`, и ненулевой код возврата на незнакомый флаг ещё не значит, что
/// утилиты нет. Спавн не удался — единственная причина пропуска.
fn tool_available(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

/// Пропустить тест с внятной причиной, если утилиты нет в `PATH`.
fn missing(tool: &str) -> bool {
    if tool_available(tool) {
        return false;
    }
    eprintln!("пропуск: {tool} не найден в PATH (в CI ставится шагом «pdf tooling»)");
    true
}

/// `qpdf --check` не находит ни ошибок, ни предупреждений: код возврата 0,
/// а в stderr нет слова «error».
///
/// Предупреждения qpdf пишет в stderr, итоговую строку «No syntax or stream
/// encoding errors found» — в stdout, поэтому «error» ищется только в stderr:
/// на чистом файле сводка не должна считаться ошибкой.
fn qpdf_check(path: &Path, what: &str) {
    let output = Command::new("qpdf")
        .arg("--check")
        .arg(path)
        .output()
        .expect("qpdf запускается");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(
        output.status.success(),
        "qpdf --check вернул {} для {what}: {stderr}",
        output.status
    );
    assert!(
        !stderr.to_lowercase().contains("error"),
        "qpdf --check сообщил об ошибке в {what}: {stderr}"
    );
}

/// Удалить временный PDF.
fn remove(path: &Path) {
    std::fs::remove_file(path).expect("временный PDF удаляется");
}

/// Текст всего PDF, извлечённый `pdftotext`: страницы разделены `\u{c}`.
fn pdftotext(path: &Path) -> String {
    // `-` — вывод в stdout: pdftotext отдаёт текст в UTF-8 по картам ToUnicode.
    let output = Command::new("pdftotext")
        .arg(path)
        .arg("-")
        .output()
        .expect("pdftotext запускается");
    assert!(
        output.status.success(),
        "pdftotext вернул {}",
        output.status
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `qpdf --check` чист на уже проверенных ранее текстовых листах — базовая
/// линия, с которой сравниваются новые артефакты.
#[test]
fn qpdf_check_accepts_exported_pdfs() {
    if missing("qpdf") {
        return;
    }

    for name in ["scale-ten-pages.xlsx", "text-cyrillic-wrap.xlsx"] {
        let path = export_to_temp(name);
        qpdf_check(&path, name);
        remove(&path);
    }
}

/// Картинки (`/XObject /Image`, PNG и JPEG) не ломают структуру файла.
#[test]
fn qpdf_check_accepts_images() {
    if missing("qpdf") {
        return;
    }

    for name in ["images-png.xlsx", "images-jpeg.xlsx"] {
        let path = export_to_temp(name);
        qpdf_check(&path, name);
        remove(&path);
    }
}

/// Гиперссылки (`/Link` c `/URI`) и закладки (`/Outlines`) — и на одном
/// листе, и в книге целиком с закладкой на каждый лист.
#[test]
fn qpdf_check_accepts_links_and_bookmarks() {
    if missing("qpdf") {
        return;
    }

    let links = export_to_temp("layout-links.xlsx");
    qpdf_check(&links, "layout-links.xlsx (ссылки и outline листа)");
    remove(&links);

    let book = export_book_to_temp(
        "sheets-three.xlsx",
        "sheets-three.xlsx",
        PdfOptions::default(),
    );
    qpdf_check(&book, "sheets-three.xlsx (книга, закладка на лист)");
    remove(&book);
}

/// Диаграммы рисуются векторно и тоже проходят независимый валидатор.
#[test]
fn qpdf_check_accepts_charts() {
    if missing("qpdf") {
        return;
    }

    let path = export_to_temp("charts-five-kinds.xlsx");
    qpdf_check(&path, "charts-five-kinds.xlsx (диаграммы)");
    remove(&path);
}

/// `pdftotext` возвращает исходную кириллицу: DoD «Кириллица корректна».
#[test]
fn pdftotext_extracts_cyrillic() {
    if missing("pdftotext") {
        return;
    }

    let path = export_to_temp("text-cyrillic-wrap.xlsx");
    let text = pdftotext(&path);
    remove(&path);

    for needle in ["Привет, мир", "ё", "Ђ", "№"] {
        assert!(
            text.contains(needle),
            "pdftotext не вернул {needle:?}; извлечённый текст: {text:?}"
        );
    }
}

/// Колонтитулы доходят до текста страницы: шапка и подвал извлекаются, а
/// `&P`/`&N` разворачиваются в номер страницы и их общее число.
#[test]
fn pdftotext_extracts_headers_and_footers() {
    if missing("pdftotext") {
        return;
    }

    let mut options = PdfOptions::default();
    options.overlay.header = "&LОтчёт за квартал&Cстр. &P из &N".to_string();
    options.overlay.footer = "&CСвод &P".to_string();
    let path = export_sheet_to_temp("overlay-scale-ten-pages", "scale-ten-pages.xlsx", options);
    let text = pdftotext(&path);
    remove(&path);

    // Число страниц берётся у самого pdftotext: сколько страниц он увидел,
    // столько `&N` и обязан был напечатать.
    let pages: Vec<&str> = text
        .split('\u{c}')
        .map(str::trim)
        .filter(|page| !page.is_empty())
        .collect();
    assert!(
        pages.len() >= 2,
        "фикстура должна быть многостраничной, а страниц {}",
        pages.len()
    );
    let total = pages.len();

    let first = pages.first().expect("страницы есть");
    assert!(
        first.contains("Отчёт за квартал"),
        "шапки нет на первой странице: {first:?}"
    );
    assert!(
        first.contains(&format!("стр. 1 из {total}")),
        "номер в шапке первой страницы: {first:?}"
    );
    assert!(
        first.contains("Свод 1"),
        "подвала нет на первой странице: {first:?}"
    );
    assert!(
        !first.contains("&P") && !first.contains("&N"),
        "коды колонтитула остались неразвёрнутыми: {first:?}"
    );

    let last = pages.last().expect("страницы есть");
    assert!(
        last.contains(&format!("стр. {total} из {total}")),
        "номер в шапке последней страницы: {last:?}"
    );
}
