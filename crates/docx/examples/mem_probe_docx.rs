//! Замер памяти и времени разбора DOCX (B2 спринта 8; бюджет — ROADMAP §9).
//!
//! Пример печатает размер входа, счётчики узлов модели и время разбора;
//! пиковую память процесса снимает внешний `/usr/bin/time -l` (maximum
//! resident set size). Свой код для RSS потребовал бы `unsafe` либо чтения
//! `/proc`, которого на macOS нет:
//!
//!     cargo build --release -p doc-converter-docx --example mem_probe_docx
//!     /usr/bin/time -l target/release/examples/mem_probe_docx many_paragraphs
//!     /usr/bin/time -l target/release/examples/mem_probe_docx --open-only test-fixtures/docx/tables/simple_2x2.docx
//!
//! Аргумент — путь до `.docx`; если файла по такому пути нет, он ищется в
//! `target/fixtures/docx-large` (крупные фикстуры генератора, в репозитории их
//! нет) — как с расширением, так и без.
//!
//! `--open-only` завершается сразу после разбора. Экспорта в PDF у DOCX ещё
//! нет, поэтому это единственный режим; флаг принимается, чтобы скрипты гейта
//! не пришлось переписывать, когда режим появится.
//!
//! Формат строки — контракт с `scripts/check-docx-memory.sh` (гейт разбирает
//! её, а не текст для человека):
//!
//! ```text
//! <путь>: вход <X.XX> МиБ, абзацев <N>, таблиц <T>, изображений <I>, run <R>, разбор <Y.Y> мс
//! ```
//!
//! `N` — абзацы, включая абзацы ячеек таблиц; `T` — таблицы, включая
//! вложенные; `I` — `Drawing` в содержимом run'ов и на уровне inline, включая
//! вложенные в ссылки и поля; `R` — run'ы. Обходятся только узлы
//! `Document::body`: сноски, комментарии и колонтитулы в счётчики не входят.
//!
//! «Разбор» — это чтение файла и `open` целиком; обход модели ради счётчиков
//! в него не входит. Паника с путём и текстом ошибки — тоже часть контракта:
//! по ней гейт отличает «фикстура не та» от «бюджет превышен».

use std::path::{Path, PathBuf};
use std::time::Instant;

use doc_converter_docx::{BlockItem, Inline, RunContent};

/// Счётчики узлов модели — то, что печатает строка контракта.
#[derive(Debug, Default)]
struct Counts {
    paragraphs: usize,
    tables: usize,
    images: usize,
    runs: usize,
}

impl Counts {
    /// Обойти блоки в порядке документа; вложенные таблицы — рекурсией по ячейкам.
    fn blocks(&mut self, items: &[BlockItem]) {
        for item in items {
            match item {
                BlockItem::Paragraph(paragraph) => {
                    self.paragraphs += 1;
                    self.inlines(&paragraph.runs);
                }
                BlockItem::Table(table) => {
                    self.tables += 1;
                    for row in &table.rows {
                        for cell in &row.cells {
                            self.blocks(&cell.items);
                        }
                    }
                }
                BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
            }
        }
    }

    /// Обойти inline-содержимое: ссылки и поля вложены, рисунки — листья.
    fn inlines(&mut self, items: &[Inline]) {
        for item in items {
            match item {
                Inline::Run(run) => {
                    self.runs += 1;
                    self.images += run
                        .content
                        .iter()
                        .filter(|content| matches!(content, RunContent::Drawing(_)))
                        .count();
                }
                Inline::Hyperlink(link) => self.inlines(&link.runs),
                Inline::Field(field) => self.inlines(&field.result),
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

/// Аргумент — путь до `.docx`; если файл по указанному пути есть, берётся он
/// (так меряются разовые документы вне репозитория).
fn resolve(arg: &str) -> PathBuf {
    let given = Path::new(arg);
    if given.is_file() {
        return given.to_path_buf();
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/docx-large");
    let named = dir.join(arg);
    if named.is_file() || arg.ends_with(".docx") {
        return named;
    }
    dir.join(format!("{arg}.docx"))
}

fn main() {
    // Путь и флаги разбираются в любом порядке: гейт зовёт `--open-only <путь>`,
    // а рука — `<путь> --open-only`, как в PDF-примере.
    let mut arg: Option<String> = None;
    for flag in std::env::args().skip(1) {
        match flag.as_str() {
            // Разбор — вся работа примера, поэтому флаг ни на что не влияет;
            // он есть, чтобы вызовы гейта пережили появление экспорта в PDF.
            "--open-only" => {}
            _ if flag.starts_with("--") => {
                eprintln!("unknown flag: {flag}");
                std::process::exit(2);
            }
            _ if arg.is_none() => arg = Some(flag),
            other => {
                eprintln!("unexpected argument: {other} (путь задаётся один раз)");
                std::process::exit(2);
            }
        }
    }
    let Some(arg) = arg else {
        eprintln!("usage: mem_probe_docx [--open-only] <fixture.docx | путь>");
        std::process::exit(2);
    };

    let path = resolve(&arg);
    let started = Instant::now();
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let input = bytes.len();
    let document =
        doc_converter_docx::open(bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let elapsed = started.elapsed();

    let mut counts = Counts::default();
    counts.blocks(&document.body.items);

    println!(
        "{}: вход {:.2} МиБ, абзацев {}, таблиц {}, изображений {}, run {}, разбор {:.1} мс",
        path.display(),
        input as f64 / (1024.0 * 1024.0),
        counts.paragraphs,
        counts.tables,
        counts.images,
        counts.runs,
        elapsed.as_secs_f64() * 1000.0,
    );
}
