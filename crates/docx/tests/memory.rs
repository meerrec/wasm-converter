//! Бюджет памяти модели (ROADMAP §9): размер узлов в байтах.
//!
//! `Paragraph` — самый массовый узел документа, поэтому у него бюджет жёсткий: ≤ 256 байт.
//! Остальные типы печатаются информационно: по ним бюджет ещё не закрыт, и слайс, который
//! возьмётся за следующий, должен видеть точку отсчёта.
//!
//! Запуск с числами: `cargo test -p doc-converter-docx --test memory -- --nocapture`.

use std::mem::size_of;

use doc_converter_docx::{
    Cell, CellBorders, Inline, Paragraph, RawPPr, RawRPr, Run, SectionProperties,
};

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
