//! Бюджет памяти модели (ROADMAP §9): размер узлов в байтах.
//!
//! `Paragraph` — самый массовый узел документа, поэтому у него бюджет жёсткий: ≤ 256 байт.
//! Остальные типы печатаются информационно: по ним бюджет ещё не закрыт, и слайс, который
//! возьмётся за следующий, должен видеть точку отсчёта.
//!
//! Второй замер — на живых фикстурах: [`prints_memory_table_for_fixtures`] обходит
//! весь корпус обходчиком [`stats`] и печатает таблицы, из которых собирается
//! `docs/sprint-8/memory-report.md`.
//!
//! Запуск с числами: `cargo test -p doc-converter-docx --test memory -- --nocapture`.

use std::collections::BTreeMap;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use doc_converter_docx::{
    Cell, CellBorders, Inline, Paragraph, RawPPr, RawRPr, Run, SectionProperties,
};

#[path = "common/stats.rs"]
mod stats;

/// Размер типа в байтах.
const fn bytes<T>() -> usize {
    size_of::<T>()
}

/// Печатает размер типа — размеры соседних типов нужны следующим слайсам как точка отсчёта.
fn report<T>(name: &str) {
    println!("{name:<20} {} байт", bytes::<T>());
}

/// `Paragraph` укладывается в бюджет ROADMAP §9 «≤ 256 байт на абзац».
///
/// Бюджет держится тем, что три тяжёлых поля лежат по указателю: `RawPPr`, `RawRPr` и
/// `SectionProperties` заметно больше 256 байт каждое, и по значению они бы не влезли.
#[test]
fn paragraph_stays_within_budget() {
    let size = bytes::<Paragraph>();
    println!("size_of::<Paragraph>() = {size} байт");
    assert!(
        size <= 256,
        "Paragraph занимает {size} байт, бюджет ROADMAP §9 — 256"
    );
}

/// `Run` укладывается в бюджет ROADMAP §9 «≤ 128 байт на run».
///
/// Бюджет держится тем, что `RawRPr` лежит по указателю: по значению он вдвое тяжелее бюджета,
/// а run'ов в документе много.
#[test]
fn run_stays_within_budget() {
    let size = bytes::<Run>();
    println!("size_of::<Run>() = {size} байт");
    assert!(
        size <= 128,
        "Run занимает {size} байт, бюджет ROADMAP §9 — 128"
    );
}

/// `Cell` укладывается в бюджет ROADMAP §9 «≤ 192 байта на ячейку».
///
/// Бюджет держится тем, что `CellBorders` лежит по указателю: по значению он занимает
/// 192 байта — столько же, сколько весь бюджет, — и ячейка в него не влезала.
#[test]
fn cell_stays_within_budget() {
    let size = bytes::<Cell>();
    println!("size_of::<Cell>() = {size} байт");
    assert!(
        size <= 192,
        "Cell занимает {size} байт, бюджет ROADMAP §9 — 192"
    );
}

/// Размеры соседних типов модели — для следующих слайсов, бюджет пока не проверяется.
#[test]
fn reports_neighbouring_sizes() {
    report::<Run>("Run");
    report::<Cell>("Cell");
    report::<Inline>("Inline");
    report::<RawPPr>("RawPPr");
    report::<RawRPr>("RawRPr");
    report::<SectionProperties>("SectionProperties");
    report::<CellBorders>("CellBorders");
}

// ---------------------------------------------------------------------------
// Замер на фикстурах
// ---------------------------------------------------------------------------

/// Каталог с фикстурами.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Все файлы с расширением `extension` под каталогом фикстур, по возрастанию пути.
///
/// Обход свой, а не из `fixtures.rs`: тесты — разные крейты, а общий код в
/// `tests/common/` заведён только для обходчика модели. Сортировка делает порядок
/// таблиц независимым от порядка `read_dir`.
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

/// Имя фикстуры относительно каталога — так же, как в сайдкарах.
fn fixture_name(path: &Path) -> String {
    path.strip_prefix(fixtures_dir())
        .expect("путь внутри каталога фикстур")
        .to_string_lossy()
        .into_owned()
}

/// Категория фикстуры — первый каталог под `test-fixtures/docx`.
fn category(path: &Path) -> String {
    path.strip_prefix(fixtures_dir())
        .ok()
        .and_then(|relative| relative.components().next())
        .map_or_else(
            || "—".to_owned(),
            |part| part.as_os_str().to_string_lossy().into_owned(),
        )
}

/// Число с пробелами между разрядами: «1 234 567» читается легче, чем «1234567».
fn spaced(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.char_indices() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// Строка замера: имя и байты группы.
struct Row {
    name: String,
    group: stats::Group,
}

/// Строка markdown-таблицы с числами замера.
fn print_row(name: &str, group: &stats::Group) {
    let nodes = group.nodes;
    println!(
        "| {} | {} | {} | {} | {} | {} |",
        name,
        spaced(nodes.paragraphs),
        spaced(nodes.runs),
        spaced(nodes.cells),
        spaced(group.sized),
        spaced(group.heap),
    );
}

/// Шапка markdown-таблицы замеров.
fn print_header(title: &str, first_column: &str) {
    println!("\n{title}\n");
    println!("| {first_column} | абзацев | run | ячеек | Σ size_of, Б | heap, Б |");
    println!("|---|---:|---:|---:|---:|---:|");
}

/// Разбивка по частям документа: тело отдельно, служебные части отдельно.
fn print_parts(parts: &stats::Stats) {
    print_header("По частям документа (все фикстуры вместе)", "часть");
    for (name, group) in parts.rows() {
        print_row(name, &group);
    }
}

/// Печатает итоговую строку, разбивку по частям документа и markdown-таблицы по
/// фикстурам и категориям.
///
/// Числа не фиксируются: фикстуры и модель меняются, тест обязан мерить. Проверяются
/// только инварианты — корпус разобран, узлы посчитаны.
#[test]
fn prints_memory_table_for_fixtures() {
    let paths = fixture_files("docx");
    assert!(
        !paths.is_empty(),
        "в test-fixtures/docx нет ни одного .docx"
    );

    let mut fixtures: Vec<Row> = Vec::new();
    let mut broken: Vec<String> = Vec::new();
    let mut by_category: BTreeMap<String, stats::Group> = BTreeMap::new();
    let mut parts = stats::Stats::default();
    for path in &paths {
        let name = fixture_name(path);
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{name} не читается: {e}"));
        match doc_converter_docx::open(bytes) {
            Ok(document) => {
                let measured = stats::Stats::of(&document);
                let group = measured.total();
                parts.merge(&measured);
                by_category.entry(category(path)).or_default().merge(&group);
                fixtures.push(Row { name, group });
            }
            // Битые фикстуры фатальны по замыслу (ADR-0016): в таблицы они не
            // попадают, но счётчик их не теряет.
            Err(_) => broken.push(name),
        }
    }

    let mut total = stats::Group::default();
    for row in &fixtures {
        total.merge(&row.group);
    }

    let nodes = total.nodes;
    println!(
        "Фикстур: {} (разобрано {}, фатальных {}); абзацев {}; run {}; ячеек {}; строк {}; \
         таблиц {}; ссылок {}; полей {}; закладок {}; рисунков {}; Unknown {}; текста, Б {}; \
         Σ size_of, Б {}; heap, Б {}",
        paths.len(),
        fixtures.len(),
        broken.len(),
        spaced(nodes.paragraphs),
        spaced(nodes.runs),
        spaced(nodes.cells),
        spaced(nodes.rows),
        spaced(nodes.tables),
        spaced(nodes.hyperlinks),
        spaced(nodes.fields),
        spaced(nodes.bookmarks),
        spaced(nodes.drawings),
        spaced(nodes.unknown),
        spaced(nodes.text_bytes),
        spaced(total.sized),
        spaced(total.heap),
    );
    if !broken.is_empty() {
        println!("Не разобрались: {}", broken.join(", "));
    }

    print_parts(&parts);

    print_header("По фикстурам", "фикстура");
    for row in &fixtures {
        print_row(&row.name, &row.group);
    }

    let categories: Vec<Row> = by_category
        .iter()
        .map(|(name, group)| Row {
            name: name.clone(),
            group: *group,
        })
        .collect();
    print_header("По категориям", "категория");
    for row in &categories {
        print_row(&row.name, &row.group);
    }

    assert_eq!(
        fixtures.len() + broken.len(),
        paths.len(),
        "каждая фикстура должна быть либо разобрана, либо отнесена к фатальным"
    );
    // Три фикстуры `broken/` падают по замыслу (ADR-0016), поэтому отказ разбора
    // вне этого каталога — находка, а не норма.
    for name in &broken {
        assert!(
            name.starts_with("broken/"),
            "{name}: разбор упал вне `broken/`"
        );
    }
    // Нижняя граница, а не точное число: набор фикстур растёт, и рост не должен
    // ронять замер, а регрессия разбора — должна.
    assert!(
        fixtures.len() >= 94,
        "разобралось {} фикстур — меньше, чем было до слайса",
        fixtures.len()
    );
    assert!(total.sized > 0, "Σ size_of = 0: обходчик не увидел узлов");
    assert!(
        nodes.paragraphs > 0,
        "обходчик не посчитал ни одного абзаца"
    );
    for (name, group) in &by_category {
        assert!(
            group.nodes.paragraphs > 0,
            "в категории {name} ни одного абзаца: обход её не видит"
        );
    }
}
