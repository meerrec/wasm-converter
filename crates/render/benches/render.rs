use criterion::{black_box, criterion_group, criterion_main, Criterion};
use doc_converter_render::display_list::*;
use doc_converter_render::sab::RingState;

fn make_dl_rects(n: usize) -> Vec<u8> {
    let mut dl = DisplayList::with_capacity(n);
    for i in 0..n {
        dl.push(DrawCommand::Rect {
            x: (i % 100) as f32,
            y: (i / 100) as f32,
            w: 10.0,
            h: 10.0,
            fill: Color::rgba(0, 0, 0, 255),
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    dl.to_bytes()
}

fn make_dl_text(n: usize) -> Vec<u8> {
    let mut dl = DisplayList::with_capacity(n);
    for i in 0..n {
        let s = dl.intern("Hello, world");
        let font = dl.intern("Calibri");
        dl.push(DrawCommand::Text {
            x: i as f32,
            y: 0.0,
            text: s,
            font,
            size: 12.0,
            color: Color::BLACK,
            align: TextAlign::Left,
            baseline: TextBaseline::Alphabetic,
            bold: false,
            italic: false,
        });
    }
    dl.to_bytes()
}

fn bench_dl_encode(c: &mut Criterion) {
    let mut dl = DisplayList::with_capacity(1000);
    for i in 0..1000 {
        dl.push(DrawCommand::Rect {
            x: i as f32,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            fill: Color::BLACK,
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        });
    }
    c.bench_function("dl_encode_1k_rects", |b| {
        let mut out = Vec::with_capacity(64 * 1024);
        b.iter(|| {
            dl.to_bytes_into(black_box(&mut out));
            black_box(out.len())
        })
    });
}

fn bench_dl_decode(c: &mut Criterion) {
    let bytes = make_dl_rects(1000);
    c.bench_function("dl_decode_1k_rects", |b| {
        b.iter(|| {
            let r = DisplayList::from_bytes(black_box(&bytes)).unwrap();
            black_box(r.iter().count())
        })
    });
}

fn bench_dl_decode_text(c: &mut Criterion) {
    let bytes = make_dl_text(1000);
    c.bench_function("dl_decode_1k_text", |b| {
        b.iter(|| {
            let r = DisplayList::from_bytes(black_box(&bytes)).unwrap();
            black_box(r.iter().count())
        })
    });
}

fn bench_sab_state_cycle(c: &mut Criterion) {
    c.bench_function("sab_state_cycle_1k", |b| {
        b.iter(|| {
            let mut st = RingState::default();
            for _ in 0..1000 {
                st.commit_write(64);
                st.commit_read();
            }
            black_box(st.seq_writer)
        })
    });
}

criterion_group!(
    benches,
    bench_dl_encode,
    bench_dl_decode,
    bench_dl_decode_text,
    bench_sab_state_cycle
);
criterion_main!(benches);
