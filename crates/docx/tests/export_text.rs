//! Экспорт в обычный текст: текст на месте, разметки нет, корпус не падает.

use std::path::{Path, PathBuf};

use doc_converter_docx::export::text::{paragraph_text, to_text};
use doc_converter_docx::open;

/// Каталог с фикстурами.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Все `.docx` под каталогом фикстур, по возрастанию пути.
fn fixture_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack = vec![fixtures_dir()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{} не читается: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("запись каталога").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "docx") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

/// Разбирает фикстуру по имени и экспортирует её в обычный текст.
fn text_of(name: &str) -> String {
    let path = fixtures_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} не читается: {error}", path.display()));
    let document = open(bytes).unwrap_or_else(|error| panic!("{name} не разбирается: {error}"));
    to_text(&document)
}

#[test]
fn text_keeps_the_content_and_drops_the_markup() {
    // Начертания размечены в модели, но в обычном тексте от них остаётся только текст.
    assert_eq!(
        text_of("basic/multiple_runs.docx"),
        "Plain bold italic underlined colored sized emphasized\n"
    );
    // Ссылка — текстом, без адреса и скобок.
    assert_eq!(
        text_of("fields/hyperlink_field.docx"),
        "example.com/docs\nLink: example.com/docs\n"
    );
    // Список — строками, без маркеров и номеров.
    assert_eq!(
        text_of("numbering/decimal_basic.docx"),
        "Первый пункт\nВторой пункт\nТретий пункт\n"
    );
    // Таблица — строками, ячейки через табуляцию.
    assert_eq!(text_of("tables/simple_2x2.docx"), "A1\tA2\nB1\tB2\n");
    // Заголовок — обычной строкой.
    assert_eq!(text_of("formatting/heading_1.docx"), "Heading 1\n");
}

#[test]
fn text_has_no_markdown_markers_and_keeps_every_paragraph() {
    let paths = fixture_files();
    assert!(
        !paths.is_empty(),
        "в test-fixtures/docx нет ни одного .docx"
    );
    let mut exported = 0;
    let mut skipped = 0;
    for path in &paths {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|error| panic!("{} не читается: {error}", path.display()));
        let Ok(document) = open(bytes) else {
            skipped += 1;
            continue;
        };
        let text = to_text(&document);
        for marker in ["**", "<u>", "](http", "| --- |"] {
            assert!(
                !text.contains(marker),
                "{}: в тексте остался маркер Markdown {marker:?}:\n{text}",
                path.display()
            );
        }
        // Текст каждого абзаца тела доезжает до вывода целиком.
        for item in &document.body.items {
            let doc_converter_docx::BlockItem::Paragraph(paragraph) = item else {
                continue;
            };
            let plain = paragraph_text(paragraph);
            if !plain.trim().is_empty() {
                assert!(
                    text.contains(&plain),
                    "{}: в тексте нет абзаца {plain:?}:\n{text}",
                    path.display()
                );
            }
        }
        exported += 1;
    }
    println!(
        "Текст: {exported} фикстур экспортировано, {skipped} не разбирается из {}",
        paths.len()
    );
    assert_eq!(exported + skipped, paths.len());
    assert!(exported >= 50, "экспортировано всего {exported} фикстур");
}
