//! Обход всех коммитимых фикстур DOCX: разбор и сверка с сайдкаром.
//!
//! `test-fixtures/docx/**` собирает `scripts/generate_docx_fixtures.ts`: рядом с
//! каждым `.docx` он кладёт `.json` с ожиданиями — числом абзацев и таблиц тела,
//! текстами абзацев, видами предупреждений и (у битых) вариантом фатальной
//! ошибки (ADR-0016). Здесь каждый пакет разбирается публичным входом крейта, а
//! расхождения копятся списком: падение в конце перечисляет их все, а не
//! обрывается на первом.
//!
//! Дополнительно проверяются два свойства `NodeId` (ADR-0019): повторный разбор
//! даёт то же дерево, и идентификаторы внутри документа не повторяются.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use doc_converter_core::{Error as CoreError, NodeId, WarningKind, ZipLimits};
use doc_converter_docx::{
    parse_docx, BlockItem, Body, BreakKind, Comment, Document, Error, Footnote, Inline,
    InlineOrAnchor, Row, Run, RunContent,
};
use serde_json::Value;

/// Каталог с фикстурами.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Все файлы с расширением `extension` под каталогом фикстур, по возрастанию пути.
fn fixture_files(extension: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack = vec![fixtures_dir()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{} не читается: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("запись каталога").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == extension) {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

/// Имя фикстуры относительно каталога — так, как оно записано в сайдкаре.
fn fixture_name(path: &Path) -> String {
    path.strip_prefix(fixtures_dir())
        .expect("путь внутри каталога фикстур")
        .to_string_lossy()
        .into_owned()
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{} не читается: {e}", path.display()))
}

/// Сайдкар фикстуры — эталон ожиданий.
fn sidecar(path: &Path) -> Value {
    let json_path = path.with_extension("json");
    let text = std::fs::read_to_string(&json_path)
        .unwrap_or_else(|e| panic!("{} не читается: {e}", json_path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} — некорректный JSON: {e}", json_path.display()))
}

/// Необязательное число из `metadata`; отсутствие или не-число — расхождение.
fn meta_u64(metadata: &Value, key: &str, name: &str, failures: &mut Vec<String>) -> Option<u64> {
    match metadata.get(key).and_then(Value::as_u64) {
        Some(value) => Some(value),
        None => {
            failures.push(format!("{name}: в сайдкаре нет `{key}` или это не число"));
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Плоский текст тела
// ---------------------------------------------------------------------------

/// Плоский текст тела: абзацы разделяются `\n`, вложенные таблицы обходятся.
///
/// `w:br` даёт `\n`, `w:tab` — `\t`: ровно так те же места записаны в текстах
/// сайдкара, поэтому ожидаемый абзац ищется подстрокой, а не по словам.
fn body_text(body: &Body) -> String {
    let mut out = String::new();
    blocks_text(&body.items, &mut out);
    out
}

fn blocks_text(items: &[BlockItem], out: &mut String) {
    for item in items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                inlines_text(&paragraph.runs, out);
                out.push('\n');
            }
            BlockItem::Table(table) => rows_text(&table.rows, out),
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
}

fn rows_text(rows: &[Row], out: &mut String) {
    for row in rows {
        for cell in &row.cells {
            blocks_text(&cell.items, out);
        }
    }
}

fn inlines_text(items: &[Inline], out: &mut String) {
    for item in items {
        match item {
            Inline::Run(run) => run_text(run, out),
            Inline::Hyperlink(link) => inlines_text(&link.runs, out),
            Inline::Field(field) => inlines_text(&field.result, out),
            Inline::Break(BreakKind::Line) => out.push('\n'),
            Inline::Tab => out.push('\t'),
            Inline::Symbol { char: symbol, .. } => out.push(*symbol),
            Inline::Bookmark(_)
            | Inline::Break(_)
            | Inline::Drawing(_)
            | Inline::Unknown { .. } => {}
        }
    }
}

fn run_text(run: &Run, out: &mut String) {
    for content in &run.content {
        match content {
            RunContent::Text(text) => out.push_str(text),
            RunContent::Tab => out.push('\t'),
            RunContent::Break(BreakKind::Line) => out.push('\n'),
            RunContent::Symbol { char: symbol, .. } => out.push(*symbol),
            RunContent::Break(_) | RunContent::Drawing(_) | RunContent::Unknown { .. } => {}
        }
    }
}

/// Число абзацев и таблиц на верхнем уровне тела: колонтитулы, сноски и
/// содержимое ячеек в счёт не идут — так их считает генератор фикстур.
fn block_counts(body: &Body) -> (usize, usize) {
    let mut paragraphs = 0;
    let mut tables = 0;
    for item in &body.items {
        match item {
            BlockItem::Paragraph(_) => paragraphs += 1,
            BlockItem::Table(_) => tables += 1,
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
    (paragraphs, tables)
}

// ---------------------------------------------------------------------------
// Сверка одной фикстуры
// ---------------------------------------------------------------------------

/// `CyclicBasedOn` → `cyclic_based_on`: сайдкар хранит имя варианта, а
/// [`WarningKind::as_str`] — ту же строку в snake_case.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut previous_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() && previous_lower {
            out.push('_');
        }
        previous_lower = ch.is_lowercase();
        out.extend(ch.to_lowercase());
    }
    out
}

fn kind_by_name(name: &str) -> Option<WarningKind> {
    let wanted = snake_case(name);
    WarningKind::ALL
        .into_iter()
        .find(|kind| kind.as_str() == wanted)
}

/// Совпадает ли ошибка с вариантом из `metadata.fatal`.
fn fatal_matches(error: &Error, fatal: &str) -> bool {
    match fatal {
        "Zip" => matches!(error, Error::Core(CoreError::Zip(_))),
        "MissingDocumentXml" => matches!(error, Error::MissingDocumentXml),
        "MacroEnabledDocument" => matches!(error, Error::MacroEnabledDocument),
        _ => false,
    }
}

/// Сводка по корпусу: печатается после прогона.
#[derive(Default)]
struct Corpus {
    fixtures: usize,
    parsed: usize,
    fatal: usize,
    warnings: usize,
    unexpected: usize,
    unexpected_kinds: BTreeMap<String, usize>,
    unexpected_fixtures: BTreeMap<String, usize>,
    texts_checked: usize,
}

fn check_fixture(path: &Path, corpus: &mut Corpus, failures: &mut Vec<String>) {
    let name = fixture_name(path);
    let sidecar = sidecar(path);
    let metadata = &sidecar["metadata"];
    let result = parse_docx(&read(path), ZipLimits::default());

    if let Some(fatal) = metadata.get("fatal").and_then(Value::as_str) {
        corpus.fatal += 1;
        match &result {
            Ok(_) => failures.push(format!(
                "{name}: ожидалась фатальная `{fatal}`, документ разобран"
            )),
            Err(error) if fatal_matches(error, fatal) => {}
            Err(error) => failures.push(format!(
                "{name}: ожидалась фатальная `{fatal}`, получена `{error:?}`"
            )),
        }
        return;
    }

    corpus.parsed += 1;
    let document = match result {
        Ok(document) => document,
        Err(error) => {
            failures.push(format!(
                "{name}: должен разбираться, получена ошибка `{error:?}`"
            ));
            return;
        }
    };

    let (paragraphs, tables) = block_counts(&document.body);
    if let Some(expected) = meta_u64(metadata, "expectedParagraphs", &name, failures) {
        if u64::try_from(paragraphs).ok() != Some(expected) {
            failures.push(format!(
                "{name}: абзацев в теле {paragraphs}, в сайдкаре {expected}"
            ));
        }
    }
    if let Some(expected) = meta_u64(metadata, "expectedTables", &name, failures) {
        if u64::try_from(tables).ok() != Some(expected) {
            failures.push(format!(
                "{name}: таблиц в теле {tables}, в сайдкаре {expected}"
            ));
        }
    }

    let expected = match metadata.get("expectedWarnings").and_then(Value::as_array) {
        Some(values) => values,
        None => {
            failures.push(format!("{name}: в сайдкаре нет `expectedWarnings`"));
            return;
        }
    };
    let mut expected_kinds = Vec::new();
    for value in expected {
        let Some(variant) = value.as_str() else {
            failures.push(format!(
                "{name}: `{value}` в `expectedWarnings` — не строка"
            ));
            continue;
        };
        match kind_by_name(variant) {
            Some(kind) => expected_kinds.push(kind),
            None => failures.push(format!(
                "{name}: `{variant}` — не вариант WarningKind; известны: {:?}",
                WarningKind::ALL.map(WarningKind::as_str)
            )),
        }
    }
    let actual: Vec<&str> = document
        .warnings()
        .iter()
        .map(|w| w.kind.as_str())
        .collect();
    for kind in &expected_kinds {
        if document.warnings_by_kind(*kind).is_empty() {
            failures.push(format!(
                "{name}: нет предупреждения `{}` из сайдкара; есть {actual:?}",
                kind.as_str()
            ));
        }
    }
    for warning in document.warnings() {
        corpus.warnings += 1;
        if !expected_kinds.contains(&warning.kind) {
            corpus.unexpected += 1;
            *corpus
                .unexpected_kinds
                .entry(warning.kind.as_str().to_owned())
                .or_default() += 1;
            *corpus.unexpected_fixtures.entry(name.clone()).or_default() += 1;
        }
    }

    // Сайдкар перечисляет тексты не у всех фикстур: там, где категория несёт
    // свои ожидания (`headers`, `revisions`, `footnotes`), ключа нет вовсе.
    let text = body_text(&document.body);
    let paragraphs = sidecar["content"]["paragraphs"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    for paragraph in paragraphs {
        let Some(expected_text) = paragraph.get("text").and_then(Value::as_str) else {
            failures.push(format!("{name}: у абзаца сайдкара нет строкового `text`"));
            continue;
        };
        if expected_text.is_empty() {
            continue;
        }
        corpus.texts_checked += 1;
        if !text.contains(expected_text) {
            failures.push(format!(
                "{name}: текста абзаца `{expected_text}` нет в теле; тело: `{}`",
                truncate(&text)
            ));
        }
    }
}

/// Начало длинного текста для сообщения о падении.
fn truncate(text: &str) -> String {
    const LIMIT: usize = 240;
    if text.chars().count() <= LIMIT {
        return text.to_owned();
    }
    let head: String = text.chars().take(LIMIT).collect();
    format!("{head}…")
}

/// Пройти все фикстуры: каждая либо разбирается и сходится с сайдкаром, либо
/// падает ровно так, как предсказано в `metadata.fatal`.
#[test]
fn every_fixture_matches_its_sidecar() {
    let paths = fixture_files("docx");
    assert!(
        !paths.is_empty(),
        "в test-fixtures/docx нет ни одного .docx"
    );
    assert_eq!(
        paths.len(),
        fixture_files("json").len(),
        "у каждой фикстуры должен быть парный .json"
    );

    let mut corpus = Corpus::default();
    let mut failures = Vec::new();
    for path in &paths {
        corpus.fixtures += 1;
        check_fixture(path, &mut corpus, &mut failures);
    }

    println!(
        "Фикстур: {} (разобрано {}, фатальных {}); сверено текстов абзацев: {}; предупреждений суммарно: {} (лишних, сверх сайдкара, {} по видам {:?}, по фикстурам {:?})",
        corpus.fixtures,
        corpus.parsed,
        corpus.fatal,
        corpus.texts_checked,
        corpus.warnings,
        corpus.unexpected,
        corpus.unexpected_kinds,
        corpus.unexpected_fixtures,
    );
    assert!(
        failures.is_empty(),
        "{} фикстур(ы) разошлись с сайдкаром:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Детерминизм и уникальность NodeId (ADR-0019)
// ---------------------------------------------------------------------------

/// Повторный разбор того же файла даёт то же дерево и те же предупреждения.
#[test]
fn parsing_is_deterministic() {
    let mut failures = Vec::new();
    let mut compared = 0;
    for path in fixture_files("docx") {
        let name = fixture_name(&path);
        let bytes = read(&path);
        let (first, second) = (
            parse_docx(&bytes, ZipLimits::default()),
            parse_docx(&bytes, ZipLimits::default()),
        );
        match (first, second) {
            (Ok(first), Ok(second)) => {
                compared += 1;
                let body = serde_json::to_value(&first.body).expect("Body сериализуется");
                let repeat = serde_json::to_value(&second.body).expect("Body сериализуется");
                if body != repeat {
                    failures.push(format!("{name}: повторный разбор дал другое дерево"));
                }
                if first.warnings() != second.warnings() {
                    failures.push(format!(
                        "{name}: повторный разбор дал другие предупреждения"
                    ));
                }
            }
            (Err(_), Err(_)) => {}
            _ => failures.push(format!("{name}: разбор перестал быть воспроизводимым")),
        }
    }
    println!("Детерминизм: сравнено фикстур — {compared}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Все `NodeId` документа различны: один аллокатор выдаёт каждому узлу свой номер.
#[test]
fn node_ids_are_unique() {
    let mut failures = Vec::new();
    let mut total = 0;
    for path in fixture_files("docx") {
        let name = fixture_name(&path);
        let Ok(document) = parse_docx(&read(&path), ZipLimits::default()) else {
            continue;
        };
        let mut ids = Vec::new();
        collect_ids(&document, &mut ids);
        let mut unique = BTreeSet::new();
        let mut duplicates = BTreeSet::new();
        for id in ids {
            if !unique.insert(id) {
                duplicates.insert(id);
            }
        }
        if !duplicates.is_empty() {
            failures.push(format!("{name}: повторяются NodeId {duplicates:?}"));
        }
        total += unique.len();
    }
    println!("NodeId: проверено идентификаторов — {total}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn collect_ids(document: &Document, out: &mut Vec<NodeId>) {
    out.push(document.id);
    body_ids(&document.body, out);
    for footnote in document.footnotes.iter().chain(&document.endnotes) {
        note_ids(footnote, out);
    }
    for comment in &document.comments {
        comment_ids(comment, out);
    }
    for header_footer in document.headers.values().chain(document.footers.values()) {
        out.push(header_footer.id);
        body_ids(&header_footer.body, out);
    }
}

fn note_ids(footnote: &Footnote, out: &mut Vec<NodeId>) {
    out.push(footnote.id);
    blocks_ids(&footnote.body, out);
}

fn comment_ids(comment: &Comment, out: &mut Vec<NodeId>) {
    out.push(comment.id);
    blocks_ids(&comment.body, out);
}

fn body_ids(body: &Body, out: &mut Vec<NodeId>) {
    out.push(body.id);
    blocks_ids(&body.items, out);
    for section in &body.sections {
        out.push(section.id);
    }
}

fn blocks_ids(items: &[BlockItem], out: &mut Vec<NodeId>) {
    for item in items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                out.push(paragraph.id);
                inlines_ids(&paragraph.runs, out);
            }
            BlockItem::Table(table) => {
                out.push(table.id);
                for row in &table.rows {
                    out.push(row.id);
                    for cell in &row.cells {
                        out.push(cell.id);
                        blocks_ids(&cell.items, out);
                    }
                }
            }
            BlockItem::Unknown { id, .. } => out.push(*id),
            BlockItem::SectPr(_) => {}
        }
    }
}

fn inlines_ids(items: &[Inline], out: &mut Vec<NodeId>) {
    for item in items {
        match item {
            Inline::Run(run) => {
                out.push(run.id);
                for content in &run.content {
                    match content {
                        RunContent::Drawing(drawing) => drawing_ids(drawing, out),
                        RunContent::Unknown { id, .. } => out.push(*id),
                        RunContent::Text(_)
                        | RunContent::Tab
                        | RunContent::Break(_)
                        | RunContent::Symbol { .. } => {}
                    }
                }
            }
            Inline::Hyperlink(link) => {
                out.push(link.id);
                inlines_ids(&link.runs, out);
            }
            Inline::Bookmark(bookmark) => out.push(bookmark.id),
            Inline::Field(field) => {
                out.push(field.id);
                inlines_ids(&field.result, out);
            }
            Inline::Drawing(drawing) => drawing_ids(drawing, out),
            Inline::Unknown { id, .. } => out.push(*id),
            Inline::Break(_) | Inline::Tab | Inline::Symbol { .. } => {}
        }
    }
}

fn drawing_ids(drawing: &InlineOrAnchor, out: &mut Vec<NodeId>) {
    out.push(drawing.id);
    if let Some(inline) = &drawing.inline {
        out.push(inline.id);
    }
    if let Some(anchor) = &drawing.anchor {
        out.push(anchor.id);
        out.push(anchor.image.id);
    }
}
