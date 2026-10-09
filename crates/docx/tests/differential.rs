//! Гейт дифференциальной проверки парсера против mammoth.
//!
//! Оракул `test-fixtures/docx/mammoth-oracle.json` строит `scripts/diff-mammoth.ts`
//! (`pnpm diff:mammoth`): по каждой фикстуре он запоминает нормализованные абзацы
//! mammoth.js и классифицирует расхождение (`match`, `expected`, `non-goal`,
//! `bug`, `unclassified`). Здесь те же данные превращаются в проверку: пока
//! парсер и mammoth совпадают на всём, что объявлено `match`, гейт зелёный; любое
//! новое расхождение — падение.
//!
//! Гейт проверяет четыре вещи:
//!
//! - множества фикстур сходятся в обе стороны: каждый `.docx` каталога описан в
//!   оракуле, и наоборот;
//! - там, где объявлен `match`, наши абзацы обязаны совпасть с mammoth; классы
//!   `bug` и `unclassified` не проходят никогда — расхождение обязано получить
//!   решение, а не остаться «ну почти»;
//! - `expected`/`non-goal` несут непустую причину, несопоставимые записи —
//!   непустой `skip_reason`;
//! - метрика ROADMAP §9: кандидатов ≥ 50, `non-goal` ≤ 8, доля
//!   `match / (кандидаты − non-goal)` ≥ 0.95.
//!
//! Наши абзацы считаются **здесь заново, а не читаются из оракула**: в оракул они
//! намеренно не пишутся (`model.counts` — только диагностика), иначе сверка
//! выродилась бы в сравнение оракула с самим собой и регрессию парсера не поймала
//! бы.
//!
//! Нормализация повторена из `crates/docx/examples/dump_model.rs` дословно — он
//! источник истины. Расхождение с ним обнаружится само: оракул построен на выводе
//! этого примера, и любая правка нормализации здесь повалит классификацию `match`
//! на фикстурах, которые трогает правка. Третий вариант нормализации заводить не
//! нужно: правится пример, за ним перегенерируется оракул.
//!
//! Тест не запускает ни Node, ни cargo и не ходит в сеть: он читает оракул и
//! фикстуры, и больше ничего.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use doc_converter_docx::{BlockItem, Document, Inline, RunContent};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Пороги гейта
// ---------------------------------------------------------------------------

/// Минимум кандидатов: ROADMAP, DoD Спринта 8 — «50 из них — дифференциальный
/// тест vs mammoth ≥ 95 %»; там же §9, строка «Differential vs mammoth | ≥ 95 %».
const MIN_CANDIDATES: usize = 50;

/// Потолок `non-goal` — ≈10 % кандидатов (`MAX_NON_GOAL` в
/// `scripts/diff-mammoth.ts`). Без потолка знаменатель `кандидаты − non-goal`
/// раздувается переклассификацией: чем больше расхождений объявлено «не нашей
/// целью», тем выше доля.
const MAX_NON_GOAL: usize = 8;

/// Порог доли совпадений — ROADMAP §9, он же в заголовке оракула (`threshold`).
const THRESHOLD: f64 = 0.95;

/// Оракул рядом с фикстурами — единственный `.json` в каталоге, у которого нет
/// пары `.docx`.
const ORACLE: &str = "mammoth-oracle.json";

/// Переменная окружения, подменяющая путь к оракулу.
///
/// Нужна ровно одному потребителю — проверке самого гейта: тест, который нельзя
/// заставить упасть, ничего не проверяет. Рабочие прогоны и CI её не ставят.
const ORACLE_ENV: &str = "DOCX_MAMMOTH_ORACLE";

/// Что делать человеку, у которого гейт покраснел.
const REGENERATE: &str = "Оракул перегенерируется командой `pnpm diff:mammoth`, \
     сверка без записи — `npx tsx scripts/diff-mammoth.ts --check`.";

// ---------------------------------------------------------------------------
// Оракул
// ---------------------------------------------------------------------------

/// Запись оракула по одной фикстуре; поля — контракт `buildOracle` в
/// `scripts/diff-mammoth.ts`.
struct Record {
    key: String,
    comparable: bool,
    /// Причина несопоставимости; у сопоставимых `None`.
    skip_reason: Option<String>,
    /// `None` — классификации нет (у несопоставимых записей).
    classification: Option<String>,
    /// Причина расхождения: непустая у `expected`/`non-goal`, пустая у `match`.
    reason: String,
    /// Нормализованные абзацы mammoth; `None` — в записи `null`.
    mammoth: Option<Vec<String>>,
    /// `model.counts` — счётчики нашего парсера на момент генерации. Только
    /// диагностика: в решении гейта не участвует (см. модульную документацию).
    model_counts: Option<String>,
}

/// Заголовок оракула и записи по фикстурам.
struct Oracle {
    path: PathBuf,
    generator: String,
    mammoth: String,
    threshold: f64,
    min_comparable: usize,
    candidates: usize,
    records: Vec<Record>,
}

/// Обязательное поле оракула: отсутствие — не «ноль», а сломанный контракт.
fn required<'a>(value: &'a Value, field: &str, where_: &str) -> &'a Value {
    value
        .get(field)
        .unwrap_or_else(|| panic!("{where_}: в оракуле нет поля `{field}`"))
}

fn required_str(value: &Value, field: &str, where_: &str) -> String {
    required(value, field, where_)
        .as_str()
        .unwrap_or_else(|| panic!("{where_}: `{field}` — не строка"))
        .to_owned()
}

fn required_usize(value: &Value, field: &str, where_: &str) -> usize {
    let number = required(value, field, where_)
        .as_u64()
        .unwrap_or_else(|| panic!("{where_}: `{field}` — не целое"));
    usize::try_from(number).unwrap_or_else(|_| panic!("{where_}: `{field}` не влезает в usize"))
}

fn required_f64(value: &Value, field: &str, where_: &str) -> f64 {
    required(value, field, where_)
        .as_f64()
        .unwrap_or_else(|| panic!("{where_}: `{field}` — не число"))
}

/// Абзацы mammoth из записи: `null` — записи нет, список обязан быть списком
/// строк.
fn mammoth_paragraphs(record: &Value, key: &str) -> Option<Vec<String>> {
    let mammoth = required(record, "mammoth", key);
    if mammoth.is_null() {
        return None;
    }
    let paragraphs = required(mammoth, "paragraphs", key)
        .as_array()
        .unwrap_or_else(|| panic!("{key}: `mammoth.paragraphs` — не список"));
    Some(
        paragraphs
            .iter()
            .map(|paragraph| {
                paragraph
                    .as_str()
                    .unwrap_or_else(|| panic!("{key}: абзац mammoth — не строка"))
                    .to_owned()
            })
            .collect(),
    )
}

/// Прочитать оракул целиком. Сломанная структура — паника: чинить её в тесте
/// нечем, оракул машинный.
fn read_oracle(path: &Path) -> Oracle {
    let text =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("{} не читается: {e}", path.display()));
    let root: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} — некорректный JSON: {e}", path.display()));

    let fixtures = required(&root, "fixtures", "оракул")
        .as_object()
        .unwrap_or_else(|| panic!("оракул: `fixtures` — не объект"));
    let mut records = Vec::with_capacity(fixtures.len());
    for (key, record) in fixtures {
        records.push(Record {
            key: key.clone(),
            comparable: required(record, "comparable", key)
                .as_bool()
                .unwrap_or_else(|| panic!("{key}: `comparable` — не bool")),
            skip_reason: required(record, "skip_reason", key)
                .as_str()
                .map(str::to_owned),
            classification: required(record, "classification", key)
                .as_str()
                .map(str::to_owned),
            reason: required_str(record, "reason", key),
            mammoth: mammoth_paragraphs(record, key),
            model_counts: record
                .get("model")
                .and_then(|model| model.get("counts"))
                .and_then(|counts| serde_json::to_string(counts).ok()),
        });
    }
    records.sort_by(|left, right| left.key.cmp(&right.key));

    Oracle {
        path: path.to_path_buf(),
        generator: required_str(&root, "generator", "оракул"),
        mammoth: required_str(&root, "mammoth", "оракул"),
        threshold: required_f64(&root, "threshold", "оракул"),
        min_comparable: required_usize(&root, "min_comparable", "оракул"),
        candidates: required_usize(&root, "candidates", "оракул"),
        records,
    }
}

// ---------------------------------------------------------------------------
// Наша сторона
// ---------------------------------------------------------------------------

/// Каталог с фикстурами — тот же, что у остальных тестов крейта.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Оракул по умолчанию; `DOCX_MAMMOTH_ORACLE` подменяет путь (см. `ORACLE_ENV`).
fn oracle_path() -> PathBuf {
    std::env::var_os(ORACLE_ENV).map_or_else(|| fixtures_dir().join(ORACLE), PathBuf::from)
}

/// Все `.docx` под каталогом фикстур: ключ — путь относительно каталога без
/// расширения (`basic/hello`), как в оракуле. Образец — `tests/fixtures.rs`.
fn discover(dir: &Path, prefix: &str, out: &mut BTreeMap<String, PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("{} не читается: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("запись каталога").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            discover(&path, &join(prefix, &name), out);
        } else if path.extension().is_some_and(|ext| ext == "docx") {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            out.insert(join(prefix, &stem), path);
        }
    }
}

/// Склеить ключ фикстуры: `prefix/name`, либо просто `name` в корне.
fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Абзацы фикстуры в порядке документа — нормализация из
/// `crates/docx/examples/dump_model.rs`, повторённая дословно (см. модульную
/// документацию).
fn paragraphs(document: &Document) -> Vec<String> {
    let mut out = Vec::new();
    walk_blocks(&document.body.items, &mut out);
    out
}

/// Блоки в порядке документа; таблица — рекурсивно: строки по порядку, ячейки по
/// порядку, их `items` — в тот же плоский список.
fn walk_blocks(items: &[BlockItem], out: &mut Vec<String>) {
    for item in items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                let mut text = String::new();
                text_of_inlines(&paragraph.runs, &mut text);
                let text = text.trim();
                if !text.is_empty() {
                    out.push(text.to_owned());
                }
            }
            BlockItem::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        walk_blocks(&cell.items, out);
                    }
                }
            }
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
}

/// Текст inline-содержимого: прогоны как есть, `w:tab` → `"\t"`, разрывы,
/// символы, рисунки и неизвестное — ничего; ссылка и результат поля —
/// рекурсивно. Сноски, концевые сноски, комментарии и колонтитулы сюда не
/// попадают: mammoth выносит их отдельно.
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

/// Разобрать и нормализовать фикстуру; ошибка — текстом, а не паникой:
/// расхождение обязано попасть в общий список, а не оборвать прогон на первой
/// фикстуре.
fn load(path: &Path) -> Result<Vec<String>, String> {
    let bytes = fs::read(path).map_err(|e| format!("{} не читается: {e}", path.display()))?;
    let document: Document = doc_converter_docx::open(bytes).map_err(|e| format!("{e:?}"))?;
    Ok(paragraphs(&document))
}

// ---------------------------------------------------------------------------
// Сверка
// ---------------------------------------------------------------------------

/// Первое расхождение списков: индекс и оба значения (`None` — абзаца нет).
/// Списки разной длины, где короткий — префикс длинного, тоже расходятся.
fn first_difference<'a>(
    ours: &'a [String],
    theirs: &'a [String],
) -> Option<(usize, Option<&'a str>, Option<&'a str>)> {
    let common = ours.len().min(theirs.len());
    for index in 0..common {
        if ours[index] != theirs[index] {
            return Some((
                index,
                Some(ours[index].as_str()),
                Some(theirs[index].as_str()),
            ));
        }
    }
    if ours.len() != theirs.len() {
        return Some((
            common,
            ours.get(common).map(String::as_str),
            theirs.get(common).map(String::as_str),
        ));
    }
    None
}

/// Значение для сообщения о падении: `None` — абзаца нет, длинное укорачивается.
fn show(text: Option<&str>) -> String {
    match text {
        None => "<абзаца нет>".to_owned(),
        Some(text) => format!("`{}`", truncate(text)),
    }
}

/// Начало длинного текста для сообщения о падении: абзацы бывают на килобайты,
/// и падение должно оставаться читаемым.
fn truncate(text: &str) -> String {
    const LIMIT: usize = 120;
    if text.chars().count() <= LIMIT {
        return text.to_owned();
    }
    let head: String = text.chars().take(LIMIT).collect();
    format!("{head}…")
}

/// Перечисление ключей для сообщения о падении и сводки: длинный список
/// обрезается, чтобы падение оставалось читаемым.
fn keys(list: &[String]) -> String {
    const LIMIT: usize = 20;
    if list.is_empty() {
        return "нет".to_owned();
    }
    if list.len() <= LIMIT {
        return list.join(", ");
    }
    format!("{}, … ещё {}", list[..LIMIT].join(", "), list.len() - LIMIT)
}

/// Подробности расхождения абзацев одной фикстуры.
fn difference_report(
    key: &str,
    ours: &[String],
    theirs: &[String],
    difference: Option<(usize, Option<&str>, Option<&str>)>,
    model_counts: Option<&str>,
) -> String {
    let Some((index, ours_text, theirs_text)) = difference else {
        return format!("{key}: списки абзацев совпали");
    };
    let mut report = format!(
        "{key}: абзацев у нас {}, у mammoth {}; первый несовпавший индекс {index}\n    \
         наш:     {}\n    mammoth: {}",
        ours.len(),
        theirs.len(),
        show(ours_text),
        show(theirs_text)
    );
    if let Some(counts) = model_counts {
        report.push_str(&format!("\n    счётчики оракула (диагностика): {counts}"));
    }
    report
}

/// Число сопоставимых записей по классам.
#[derive(Default)]
struct Classes {
    matches: usize,
    expected: usize,
    non_goal: usize,
    bug: usize,
    unclassified: usize,
}

/// Гейт: сверка с оракулом в обе стороны, честность классификации и метрика
/// ROADMAP §9.
#[test]
fn mammoth_differential_matches_the_oracle() {
    let oracle = read_oracle(&oracle_path());
    let mut fixtures = BTreeMap::new();
    discover(&fixtures_dir(), "", &mut fixtures);
    assert!(
        !fixtures.is_empty(),
        "в test-fixtures/docx нет ни одного .docx"
    );

    let mut failures: Vec<String> = Vec::new();

    // --- множества фикстур: в обе стороны ---------------------------------
    let on_disk: BTreeSet<&str> = fixtures.keys().map(String::as_str).collect();
    let in_oracle: BTreeSet<&str> = oracle.records.iter().map(|r| r.key.as_str()).collect();
    let only_disk: Vec<String> = on_disk
        .difference(&in_oracle)
        .map(|k| (*k).to_owned())
        .collect();
    let only_oracle: Vec<String> = in_oracle
        .difference(&on_disk)
        .map(|k| (*k).to_owned())
        .collect();
    if !only_disk.is_empty() || !only_oracle.is_empty() {
        failures.push(format!(
            "наборы фикстур разошлись: на диске {}, в оракуле {}; только на диске ({}): {}; \
             только в оракуле ({}): {} — оракул описывает ровно фикстуры каталога, \
             перегенерировать",
            on_disk.len(),
            in_oracle.len(),
            only_disk.len(),
            keys(&only_disk),
            only_oracle.len(),
            keys(&only_oracle),
        ));
    }

    // --- сверка абзацев и честность классификации -------------------------
    let mut classes = Classes::default();
    let mut mismatched: Vec<String> = Vec::new();
    let mut stale: Vec<String> = Vec::new();

    for record in &oracle.records {
        // Записи без фикстуры на диске уже в расхождении множеств: сверять нечего.
        let Some(path) = fixtures.get(&record.key) else {
            continue;
        };

        if !record.comparable {
            if record.skip_reason.as_deref().is_none_or(str::is_empty) {
                failures.push(format!(
                    "{}: `comparable: false` без `skip_reason` — несопоставимость обязана быть \
                     объяснена",
                    record.key
                ));
            }
            if record.classification.is_some() {
                failures.push(format!(
                    "{}: `comparable: false`, но есть `classification` — у несопоставимых записей \
                     классификации нет",
                    record.key
                ));
            }
            if record.mammoth.is_some() {
                failures.push(format!(
                    "{}: `comparable: false`, но `mammoth` не `null` — сверять нечего, а данные есть",
                    record.key
                ));
            }
            continue;
        }

        if record.skip_reason.is_some() {
            failures.push(format!(
                "{}: `comparable: true` с непустым `skip_reason` `{}` — контракт оракула нарушен",
                record.key,
                record.skip_reason.as_deref().unwrap_or_default()
            ));
        }

        let ours = match load(path) {
            Ok(ours) => ours,
            Err(error) => {
                failures.push(format!(
                    "{}: сопоставимую фикстуру не удалось разобрать ({error}) — это регрессия \
                     парсера, а не расхождение с mammoth",
                    record.key
                ));
                continue;
            }
        };
        let Some(theirs) = &record.mammoth else {
            failures.push(format!(
                "{}: `comparable: true`, но `mammoth` — `null`: сверять не с чем",
                record.key
            ));
            continue;
        };

        let difference = first_difference(&ours, theirs);
        if difference.is_some() {
            mismatched.push(record.key.clone());
        }
        let report = difference_report(
            &record.key,
            &ours,
            theirs,
            difference,
            record.model_counts.as_deref(),
        );

        match record.classification.as_deref() {
            Some("match") => {
                classes.matches += 1;
                if difference.is_some() {
                    failures.push(format!(
                        "{report}\n    классификация `match` устарела: списки обязаны совпадать, а \
                         разошлись — это либо регрессия парсера, либо оракул надо перегенерировать"
                    ));
                }
                if !record.reason.is_empty() {
                    failures.push(format!(
                        "{}: `match` с непустым `reason` — у совпадения причины нет",
                        record.key
                    ));
                }
            }
            Some(kind @ ("expected" | "non-goal")) => {
                if kind == "expected" {
                    classes.expected += 1;
                } else {
                    classes.non_goal += 1;
                }
                if record.reason.trim().is_empty() {
                    failures.push(format!(
                        "{}: `{kind}` без `reason` — расхождение обязано называть сторону (наша \
                         модель или mammoth) и решение",
                        record.key
                    ));
                } else if difference.is_none() {
                    // Не падение: расхождение вправе схлопнуться. Но класс устарел, а
                    // `non-goal` вдобавок уменьшает знаменатель гейта — сигнал, что
                    // запись пора переклассифицировать.
                    stale.push(format!("{} ({kind})", record.key));
                }
            }
            Some(kind @ ("bug" | "unclassified")) => {
                if kind == "bug" {
                    classes.bug += 1;
                } else {
                    classes.unclassified += 1;
                }
                failures.push(format!(
                    "{report}\n    классификация `{kind}`: расхождение обязано получить решение — \
                     починить парсер или описать причину в `CLASSIFICATION` \
                     (`scripts/diff-mammoth.ts`), и перегенерировать оракул"
                ));
            }
            Some(other) => {
                failures.push(format!(
                    "{}: неизвестная классификация `{other}` — классы перечислены в \
                     `scripts/diff-mammoth.ts`",
                    record.key
                ));
            }
            None => {
                failures.push(format!(
                    "{}: у сопоставимой записи нет классификации",
                    record.key
                ));
            }
        }
    }

    // --- метрика ----------------------------------------------------------
    let comparable = oracle.records.iter().filter(|r| r.comparable).count();
    if oracle.candidates < MIN_CANDIDATES {
        failures.push(format!(
            "кандидатов {}, порог {MIN_CANDIDATES} (ROADMAP, DoD Спринта 8: «50 из них — \
             дифференциальный тест vs mammoth ≥ 95 %»)",
            oracle.candidates
        ));
    }
    if comparable < oracle.min_comparable {
        failures.push(format!(
            "сопоставимых {comparable} из {} кандидатов, порог оракула `min_comparable` = {} — \
             mammoth не прочитал часть кандидатов (причина в `skip_reason` этих записей), сверка \
             сузилась",
            oracle.candidates, oracle.min_comparable
        ));
    }
    if classes.non_goal > MAX_NON_GOAL {
        failures.push(format!(
            "`non-goal` = {}, потолок {MAX_NON_GOAL} (≈10 % кандидатов) — знаменатель гейта \
             раздут классификацией",
            classes.non_goal
        ));
    }
    let denominator = oracle.candidates.saturating_sub(classes.non_goal);
    let ratio = if denominator == 0 {
        0.0
    } else {
        classes.matches as f64 / denominator as f64
    };
    if ratio < THRESHOLD {
        failures.push(format!(
            "доля `match` / (кандидаты − non-goal) = {}/{denominator} = {ratio:.4}, порог \
             {THRESHOLD} (ROADMAP §9); классы: match {}, expected {}, non-goal {}, bug {}, \
             unclassified {}; несовпавшие фикстуры ({}): {}",
            classes.matches,
            classes.matches,
            classes.expected,
            classes.non_goal,
            classes.bug,
            classes.unclassified,
            mismatched.len(),
            keys(&mismatched),
        ));
    }
    if oracle.threshold != THRESHOLD {
        failures.push(format!(
            "порог оракула {} не совпадает с порогом гейта {THRESHOLD} — оракул сгенерирован под \
             другую планку",
            oracle.threshold
        ));
    }

    // --- сводка -----------------------------------------------------------
    println!(
        "Дифференциал против mammoth {} (оракул {}, генератор `{}`):",
        oracle.mammoth,
        oracle.path.display(),
        oracle.generator
    );
    println!(
        "  фикстур: {} в оракуле, {} на диске; кандидатов: {}, сопоставимых: {} (порог \
         min_comparable {})",
        oracle.records.len(),
        fixtures.len(),
        oracle.candidates,
        comparable,
        oracle.min_comparable
    );
    println!(
        "  match {}, expected {}, non-goal {}, bug {}, unclassified {}",
        classes.matches, classes.expected, classes.non_goal, classes.bug, classes.unclassified
    );
    println!(
        "  доля match/(кандидаты − non-goal): {}/{denominator} = {ratio:.4}, порог {THRESHOLD}",
        classes.matches
    );
    println!(
        "  несовпавшие фикстуры ({}): {}",
        mismatched.len(),
        keys(&mismatched)
    );
    if !stale.is_empty() {
        println!(
            "  классификация устарела, списки совпали ({}): {}",
            stale.len(),
            keys(&stale)
        );
    }
    if failures.is_empty() {
        println!("  вердикт: гейт пройден");
    } else {
        println!("  вердикт: ГЕЙТ НЕ ПРОЙДЕН, нарушений: {}", failures.len());
    }

    assert!(
        failures.is_empty(),
        "дифференциальная проверка против mammoth не пройдена ({} нарушени(й)):\n{}\n\n{REGENERATE}",
        failures.len(),
        failures.join("\n\n")
    );
}
