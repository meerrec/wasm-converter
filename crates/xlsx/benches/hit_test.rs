//! Попадание точки в лист: бюджет ROADMAP §9 — меньше 2 мс.
//!
//! `hit_test` строит раскладку листа на каждый вызов: бюджет закрывает именно
//! этот путь, потому что вызывающий (обработчик клика) приходит один раз, без
//! подготовленной геометрии.
//!
//! Факт (Apple Silicon, release): ~48 мкс на 5000×50 при бюджете < 2 мс.
//!
//!     cargo bench -p doc-converter-xlsx --bench hit_test

#[path = "../tests/common/package.rs"]
mod package;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use doc_converter_xlsx::paint::hit_test;
use doc_converter_xlsx::{open, Viewport};

const ROWS: u32 = 5_000;
const COLS: u32 = 50;

fn bench_hit_test(c: &mut Criterion) {
    let book = open(package::package(ROWS, COLS)).expect("синтетическая книга открывается");
    let sheet = &book.sheets()[0];
    let viewport = Viewport::new(400.0, 800.0, 1200.0, 800.0, 1.0);

    let mut group = c.benchmark_group("hit_test");
    group.bench_function(format!("{ROWS}x{COLS}"), |b| {
        b.iter(|| black_box(hit_test(sheet, viewport, 700.5, 400.25)));
    });
    group.finish();
}

criterion_group!(benches, bench_hit_test);
criterion_main!(benches);
