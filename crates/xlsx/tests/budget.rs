//! Бюджеты из ROADMAP §9 на синтетической книге.
//!
//! Проверки разделены по цене: то, что гоняется всегда, стоит миллисекунды;
//! миллион ячеек помечен `#[ignore]` — в отладочной сборке он идёт десятками
//! секунд, а бюджет ставился для релизной.
//!
//!     cargo test -p doc-converter-xlsx --release --test budget -- --ignored --nocapture

#[path = "common/package.rs"]
mod package;

use doc_converter_xlsx::{open, Cell};

/// Миллион ячеек должен открываться быстрее двух секунд (ROADMAP §9).
#[test]
#[ignore = "тяжёлый: запускать в релизной сборке с --ignored"]
fn million_cells_open_within_two_seconds() {
    let bytes = package::package(100_000, 10);
    let size = bytes.len();

    let started = std::time::Instant::now();
    let book = open(bytes).unwrap();
    let elapsed = started.elapsed();

    let cells = book.sheets()[0].cells.cell_count();
    assert_eq!(cells, 1_000_000);
    println!(
        "1M ячеек: {:?}, модель ≈ {} МБ, пакет {} МБ",
        elapsed,
        cells * std::mem::size_of::<Cell>() / (1024 * 1024),
        size / (1024 * 1024),
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "миллион ячеек разобран за {elapsed:?}, бюджет — 2 с"
    );
}

/// Размер ячейки — это и есть память модели: миллион ячеек даёт ровно
/// `cell_count × size_of::<Cell>()` плюс таблицы строк.
///
/// Проверка сторожит структуру: лишнее поле, раздувающее `Cell`, обязано
/// упереться в этот потолок и попасть в ревью, а не в прод.
#[test]
fn cell_stays_small() {
    assert_eq!(
        std::mem::size_of::<Cell>(),
        48,
        "размер ячейки изменился — пересчитайте бюджет памяти в ROADMAP"
    );
}

/// Пустые строки не занимают памяти: миллион пустых строк в файле — это две
/// записи в модели, а не миллион.
#[test]
fn empty_rows_cost_nothing() {
    let mut sheet = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for row in [1_u32, 1_000_000] {
        sheet.push_str(&format!(
            r#"<row r="{row}"><c r="A{row}"><v>{row}</v></c></row>"#
        ));
    }
    sheet.push_str("</sheetData></worksheet>");

    let bytes = package::package_with_sheet(&sheet);
    let book = open(bytes).unwrap();
    let cells = &book.sheets()[0].cells;

    assert_eq!(cells.cell_count(), 2);
    assert_eq!(cells.row_count(), 2);
    assert_eq!(cells.last_row(), Some(999_999));
    // Строки между ними не существуют, но и не мешают.
    assert!(cells.cells_of_row(500_000).is_empty());
}
