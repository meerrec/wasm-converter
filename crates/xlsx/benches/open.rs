//! Открытие книги: разбор ZIP, `sharedStrings`, стилей и листа.
//!
//! Бюджет из ROADMAP §9 — миллион ячеек меньше чем за две секунды нативно.
//! Замер идёт на пакете, собранном в памяти: так в измерение не попадает
//! чтение с диска, а объём задаётся точно.

#[path = "../tests/common/package.rs"]
mod package;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn open_sheet(c: &mut Criterion) {
    let mut group = c.benchmark_group("open");
    for (rows, cols) in [(1_000_u32, 10_u32), (10_000, 10), (100_000, 10)] {
        let bytes = package::package(rows, cols);
        let cells = u64::from(rows) * u64::from(cols);
        group.throughput(Throughput::Elements(cells));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{cells}cells")),
            &bytes,
            |b, bytes| {
                b.iter_batched(
                    || bytes.clone(),
                    |data| doc_converter_xlsx::open(data).unwrap(),
                    criterion::BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(benches, open_sheet);
criterion_main!(benches);
