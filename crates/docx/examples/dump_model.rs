//! Выгрузка нормализованного текста разобранной модели в JSON.
//!
//! Это вход differential-теста против mammoth: одна и та же фикстура
//! нормализуется здесь и на стороне mammoth, а расхождение списков абзацев
//! показывает, где разошлись парсеры.
//!
//!     cargo run -q -p doc-converter-docx --example dump_model > dump.json
//!     cargo run -q -p doc-converter-docx --example dump_model -- --only basic/multiple_runs
//!
//! Формат и нормализация — контракт: обе стороны сравнения повторяют его дословно.
//!
//! Нормализация:
//! - обход `body.items` в порядке документа; таблица — рекурсивно: строки по
//!   порядку, ячейки по порядку, их `items` — в тот же плоский список;
//! - текст абзаца — конкатенация по порядку:
//!   - `RunContent::Text` → как есть; `RunContent::Tab` → `"\t"`;
//!     `RunContent::Break`/`Symbol`/`Drawing`/`Unknown` → ничего;
//!   - `Inline::Run` → по его `content`; `Inline::Hyperlink` → рекурсивно по
//!     `runs`; `Inline::Field` → рекурсивно по `result` (кэш поля — то, что
//!     видно в документе);
//!   - `Inline::Bookmark`/`Break`/`Tab`/`Symbol`/`Drawing`/`Unknown` → ничего;
//! - `trim()`, пустые строки выбрасываются;
//! - сноски, концевые сноски, комментарии и колонтитулы в список не входят
//!   (mammoth выносит их отдельно).
//!
//! Вывод — pretty JSON (отступ 2) в stdout, ключи по возрастанию:
//!
//! ```json
//! {
//!   "basic/hello": {
//!     "paragraphs": ["Hello", "World"],
//!     "tables": 1,
//!     "list_items": 2,
//!     "hyperlinks": 1,
//!     "images": 3
//!   }
//! }
//! ```
//!
//! Ключ — имя фикстуры относительно `test-fixtures/docx` без расширения.
//! `tables` — число таблиц (`BlockItem::Table`), включая вложенные;
//! `list_items` — число абзацев с `numbering_ref.is_some()`; `hyperlinks` —
//! число `Inline::Hyperlink` (включая вложенные в поля и ссылки); `images` —
//! число `Drawing` в `RunContent` и в `Inline`, включая вложенные.
//! Фикстура, которая не разбирается, даёт `null` — пример из-за неё не падает.
//!
//! Коды возврата: 0 — успех (в том числе когда часть фикстур дала `null`),
//! 1 — неизвестное имя в `--only`, 2 — неизвестный флаг или недоступный
//! корень фикстур.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use doc_converter_docx::{BlockItem, Document, Inline, RunContent};
use serde::Serialize;

/// Нормализованный текст одной фикстуры и счётчики — схема выходного JSON.
#[derive(Debug, Default, Serialize)]
struct FixtureReport {
    /// Абзацы в порядке документа (тело и ячейки таблиц вместе).
    paragraphs: Vec<String>,
    /// Таблицы, включая вложенные.
    tables: usize,
    /// Абзацы с `numbering_ref`.
    list_items: usize,
    /// Ссылки, включая вложенные в поля и в другие ссылки.
    hyperlinks: usize,
    /// Рисунки в содержимом run'ов, включая вложенные.
    images: usize,
}

impl FixtureReport {
    /// Обойти блоки в порядке документа, накапливая текст и счётчики.
    fn walk_blocks(&mut self, items: &[BlockItem]) {
        for item in items {
            match item {
                BlockItem::Paragraph(paragraph) => {
                    let mut text = String::new();
                    text_of_inlines(&paragraph.runs, &mut text);
                    let text = text.trim();
                    if !text.is_empty() {
                        self.paragraphs.push(text.to_owned());
                    }
                    if paragraph.numbering_ref.is_some() {
                        self.list_items += 1;
                    }
                    self.count_inlines(&paragraph.runs);
                }
                BlockItem::Table(table) => {
                    self.tables += 1;
                    for row in &table.rows {
                        for cell in &row.cells {
                            self.walk_blocks(&cell.items);
                        }
                    }
                }
                BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
            }
        }
    }

    /// Сосчитать ссылки и рисунки во вложенном inline-содержимом.
    fn count_inlines(&mut self, items: &[Inline]) {
        for item in items {
            match item {
                Inline::Run(run) => {
                    self.images += run
                        .content
                        .iter()
                        .filter(|content| matches!(content, RunContent::Drawing(_)))
                        .count();
                }
                Inline::Hyperlink(link) => {
                    self.hyperlinks += 1;
                    self.count_inlines(&link.runs);
                }
                Inline::Field(field) => self.count_inlines(&field.result),
                Inline::Drawing(_) => self.images += 1,
                Inline::Bookmark(_)
                | Inline::Break(_)
                | Inline::Tab
                | Inline::Symbol { .. }
                | Inline::Unknown { .. } => {}
            }
        }
    }
}

/// Текст inline-содержимого по правилам нормализации (см. модульную документацию).
fn text_of_inlines(items: &[Inline], out: &mut String) {
    for item in items {
        match item {
            Inline::Run(run) => {
                for content in &run.content {
                    match content {
                        RunContent::Text(text) => out.push_str(text),
                        RunContent::Tab => out.push('\t'),
                        RunContent::Break(_)
                        | RunContent::Symbol { .. }
                        | RunContent::Drawing(_)
                        | RunContent::Unknown { .. } => {}
                    }
                }
            }
            Inline::Hyperlink(link) => text_of_inlines(&link.runs, out),
            Inline::Field(field) => text_of_inlines(&field.result, out),
            Inline::Bookmark(_)
            | Inline::Break(_)
            | Inline::Tab
            | Inline::Symbol { .. }
            | Inline::Drawing(_)
            | Inline::Unknown { .. } => {}
        }
    }
}

/// Собрать все `.docx` под `dir`; ключ — путь относительно `dir` без расширения.
fn discover(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let nested = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            discover(&path, &nested, out)?;
        } else if path.extension().is_some_and(|ext| ext == "docx") {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let key = if prefix.is_empty() {
                stem.into_owned()
            } else {
                format!("{prefix}/{stem}")
            };
            out.push((key, path));
        }
    }
    Ok(())
}

/// Разобрать и нормализовать одну фикстуру.
fn load(path: &Path) -> Result<FixtureReport, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let document: Document = doc_converter_docx::open(bytes)?;
    let mut report = FixtureReport::default();
    report.walk_blocks(&document.body.items);
    Ok(report)
}

/// Разобрать аргументы: `(задан ли --only, имена)`.
fn parse_args() -> (bool, Vec<String>) {
    let mut only_given = false;
    let mut only = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--only" => only_given = true,
            _ if arg.starts_with("--") => {
                eprintln!("unknown flag: {arg}");
                eprintln!("usage: dump_model [--only <fixture> [<fixture>...]]");
                std::process::exit(2);
            }
            _ if only_given => only.push(arg),
            _ => {
                eprintln!("unexpected argument: {arg} (fixtures are selected with --only)");
                std::process::exit(2);
            }
        }
    }
    (only_given, only)
}

/// Найти фикстуру по аргументу `--only`: имя без расширения (`basic/hello`) или
/// путь до `.docx` (в том числе абсолютный).
fn resolve(
    arg: &str,
    by_key: &BTreeMap<&str, usize>,
    by_path: &BTreeMap<PathBuf, usize>,
) -> Option<usize> {
    let stem = arg
        .strip_suffix(".docx")
        .or_else(|| arg.strip_suffix(".DOCX"))
        .unwrap_or(arg);
    if let Some(&index) = by_key.get(stem) {
        return Some(index);
    }
    by_path.get(&Path::new(arg).canonicalize().ok()?).copied()
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx");
    let mut fixtures = Vec::new();
    if let Err(err) = discover(&root, "", &mut fixtures) {
        eprintln!("{}: {err}", root.display());
        std::process::exit(2);
    }
    fixtures.sort_by(|left, right| left.0.cmp(&right.0));

    let (only_given, only) = parse_args();
    if only_given {
        let by_key: BTreeMap<&str, usize> = fixtures
            .iter()
            .enumerate()
            .map(|(index, (key, _))| (key.as_str(), index))
            .collect();
        let by_path: BTreeMap<PathBuf, usize> = fixtures
            .iter()
            .enumerate()
            .filter_map(|(index, (_, path))| path.canonicalize().ok().map(|abs| (abs, index)))
            .collect();
        let mut chosen = BTreeSet::new();
        let mut missing = Vec::new();
        for name in &only {
            match resolve(name, &by_key, &by_path) {
                Some(index) => {
                    chosen.insert(index);
                }
                None => missing.push(name.as_str()),
            }
        }
        if !missing.is_empty() {
            for name in missing {
                eprintln!("unknown fixture: {name}");
            }
            std::process::exit(1);
        }
        fixtures = fixtures
            .into_iter()
            .enumerate()
            .filter(|(index, _)| chosen.contains(index))
            .map(|(_, fixture)| fixture)
            .collect();
    }

    let mut dump: BTreeMap<String, Option<FixtureReport>> = BTreeMap::new();
    for (key, path) in fixtures {
        let report = match load(&path) {
            Ok(report) => Some(report),
            Err(err) => {
                // Фикстура не разобралась: ключ остаётся в выводе с `null`, чтобы
                // differential-тест отличил расхождение разбора от расхождения текста.
                eprintln!("{key}: {err}");
                None
            }
        };
        dump.insert(key, report);
    }

    let json = serde_json::to_string_pretty(&dump).expect("the dump is JSON-serializable");
    println!("{json}");
}
