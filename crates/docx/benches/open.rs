//! Открытие DOCX: ZIP-пакет, `word/document.xml`, стили и нумерация.
//!
//! Критерий ROADMAP §9 — `openDocx` на 50 МБ (native) быстрее 1,5 с — до сих
//! пор был декларацией; здесь он измеряется на фикстуре профиля
//! (`profile_50mib.docx`: 100 000 абзацев, 500 таблиц, 50 изображений).
//!
//! В замер входит только разбор: байты читаются с диска один раз, до
//! `b.iter` и до медианы. Меряется `parse_docx(&bytes, ZipLimits::default())` —
//! ровно та работа, что внутри `open`, но без передачи владения `Vec`: `open`
//! копировал бы 54 МиБ в каждом прогоне, и копия попала бы в число.
//!
//! Большие фикстуры в git не лежат (десятки МиБ), их пишет генератор:
//!
//!     npx tsx scripts/generate_docx_fixtures.ts --large
//!
//!     cargo bench -p doc-converter-docx --bench open

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use doc_converter_core::ZipLimits;
use doc_converter_docx::{parse_docx, Document};

/// Фикстура профиля ROADMAP §9.
const PROFILE: &str = "profile_50mib.docx";

/// Остальные большие фикстуры генератора (11–15 МиБ) — у каждой своя доминанта:
/// абзацы, таблицы, изображения, нумерация, смесь.
const LARGE: [&str; 5] = [
    "many_paragraphs.docx",
    "wide_table.docx",
    "many_images.docx",
    "numbered_list.docx",
    "mixed.docx",
];

/// Прогонов ручного замера медианы — как в `crates/pdf/benches/pdf.rs`.
/// Разбор 50 МиБ идёт секундами; сотня прогонов растянула бы бенч на минуты.
const MEDIAN_RUNS: usize = 5;

/// Каталог больших фикстур (`scripts/generate_docx_fixtures.ts --large`).
fn large_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/docx-large")
}

/// Байты фикстуры. Отсутствие файла — падение, а не пропуск кейса: молча
/// пропущенный замер выглядит как зелёный прогон, а число бюджета никто не
/// увидит.
fn read_large(name: &str) -> Vec<u8> {
    let path = large_dir().join(name);
    match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => panic!(
            "{}: {err} — большие фикстуры пишет \
             `npx tsx scripts/generate_docx_fixtures.ts --large`",
            path.display()
        ),
    }
}

/// Размер входа в МиБ — для `Throughput` в отчёте criterion и для печати.
fn mib(bytes: &[u8]) -> f64 {
    bytes.len() as f64 / (1024.0 * 1024.0)
}

/// Один разбор фикстуры теми же лимитами, что и у `open`.
///
/// Ошибка разбора роняет бенч, а не превращается в замер `Err`: фикстура,
/// вышедшая за `ZipLimits::default()` (64 МиБ на часть, ADR-0015), —
/// сломанный вход, и медиана по нему была бы мусором.
fn parse(bytes: &[u8]) -> Document {
    parse_docx(bytes, ZipLimits::default())
        .expect("фикстура разбирается в пределах ZipLimits::default()")
}

/// Медиана `MEDIAN_RUNS` разборов в целых миллисекундах.
///
/// Это ручной замер, а не выборка criterion: он идёт в том же процессе и сразу
/// после criterion-прогона группы (кэши те же), но `N` в печатаемой строке —
/// медиана отдельных разборов без доверительного интервала, тогда как отчёт
/// criterion показывает оценку среднего по своим сэмплам с интервалом.
fn median_ms(bytes: &[u8]) -> u64 {
    let mut times: Vec<Duration> = (0..MEDIAN_RUNS)
        .map(|_| {
            let started = Instant::now();
            black_box(parse(bytes));
            started.elapsed()
        })
        .collect();
    times.sort_unstable();
    (times[MEDIAN_RUNS / 2].as_secs_f64() * 1000.0).round() as u64
}

/// Профиль ROADMAP §9 (50 МиБ, 100 000 абзацев) и печать медианы для CI.
fn bench_open_docx_50mib(c: &mut Criterion) {
    let bytes = read_large(PROFILE);
    let input_mib = mib(&bytes);

    let mut group = c.benchmark_group("bench_open_docx_50mib");
    // МиБ/с в отчёте: вход — байты файла, а не число абзацев.
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    // Разбор 50 МиБ идёт секундами; сто сэмплов criterion растянули бы прогон
    // на минуты, десяти хватает для порядка числа (как в pdf-бенче на 500 страниц).
    group.sample_size(10);
    group.bench_function("profile_50mib", |b| {
        b.iter(|| black_box(parse(black_box(&bytes))));
    });

    let median = median_ms(&bytes);
    group.finish();

    // Строка — контракт с шагом CI: он берёт её последней в логе (`tail -1`),
    // поэтому группы объявлены так, чтобы эта печаталась после «больших».
    //
    // «50 МиБ» — часть формата, но для грепа это лишние цифры: шаблон
    // `медиана разбора [0-9]+` поймает «50», а не медиану. Разбирать строку
    // нужно целиком: `grep -oE 'медиана разбора 50 МиБ: [0-9]+' | grep -oE '[0-9]+' | tail -1`.
    println!("медиана разбора 50 МиБ: {median} мс (вход {input_mib:.1} МиБ, бюджет 1500 мс)");
}

/// Те же большие фикстуры — видно, что разбор упирается не только в абзацы.
fn bench_open_docx_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("bench_open_docx_large");
    group.sample_size(10);
    for name in LARGE {
        let bytes = read_large(name);
        let input_mib = mib(&bytes);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &bytes, |b, input| {
            b.iter(|| black_box(parse(black_box(input))));
        });
        // Строка намеренно без «разбора»: шаблон CI — про профиль ROADMAP,
        // и лишнее совпадение `медиана разбора` его бы сбило.
        let median = median_ms(&bytes);
        println!("{name}: {input_mib:.1} МиБ, медиана {median} мс");
    }
    group.finish();
}

// «Большие» первыми: строка профиля ROADMAP должна остаться в логе последней.
criterion_group!(benches, bench_open_docx_large, bench_open_docx_50mib);
criterion_main!(benches);
