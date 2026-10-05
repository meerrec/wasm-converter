//! Цена условного форматирования на кадре в 10 тысяч ячеек.
//!
//! Бюджет из ROADMAP §9 — меньше 5 мс на 10k ячеек. Меряется весь
//! `paint_sheet`: разрешение стиля и построение команд идут вместе с
//! рисованием листа, а не отдельной фазой. Базовая линия `none` — тот же лист
//! без правил: разница с ней и есть вклад условного форматирования.
//!
//!     cargo bench -p doc-converter-xlsx --bench conditional

#[path = "../tests/common/package.rs"]
mod package;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use doc_converter_render::display_list::DisplayList;
use doc_converter_xlsx::{open, paint_sheet, PaintOptions, Viewport};

const ROWS: u32 = 1_000;
const COLS: u32 = 10;

/// Лист `ROWS × COLS` с правилом на весь диапазон; `None` — без правил.
fn sheet_xml(kind: Option<&str>) -> String {
    let mut xml = String::with_capacity(ROWS as usize * COLS as usize * 16);
    xml.push_str(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for row in 0..ROWS {
        xml.push_str(&format!(r#"<row r="{}">"#, row + 1));
        for col in 0..COLS {
            xml.push_str(&format!(
                r#"<c r="{}{}"><v>{}</v></c>"#,
                package::column(col),
                row + 1,
                col
            ));
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData>");
    if let Some(rule) = kind {
        xml.push_str(&format!(
            r#"<conditionalFormatting sqref="A1:{}{}">{rule}</conditionalFormatting>"#,
            package::column(COLS - 1),
            ROWS,
        ));
    }
    xml.push_str("</worksheet>");
    xml
}

fn rule(kind: &str) -> &'static str {
    match kind {
        "color_scale" => {
            r#"<cfRule type="colorScale" priority="1"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FFF8696B"/><color rgb="FF63BE7B"/></colorScale></cfRule>"#
        }
        "data_bar" => {
            r#"<cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar></cfRule>"#
        }
        "icon_set" => {
            r#"<cfRule type="iconSet" priority="1"><iconSet iconSet="3TrafficLights1"><cfvo type="percent" val="0"/><cfvo type="percent" val="33"/><cfvo type="percent" val="67"/></iconSet></cfRule>"#
        }
        other => panic!("неизвестное правило: {other}"),
    }
}

fn conditional_formatting(c: &mut Criterion) {
    let mut group = c.benchmark_group("conditional_formatting");
    group.throughput(Throughput::Elements(u64::from(ROWS) * u64::from(COLS)));
    for kind in ["none", "color_scale", "data_bar", "icon_set"] {
        let xml = sheet_xml((kind != "none").then(|| rule(kind)));
        let book = open(package::package_with_sheet(&xml)).unwrap();
        let sheet = &book.sheets()[0];
        // Окно покрывает весь лист: иначе рисовались бы одни видимые ячейки.
        let viewport = Viewport {
            scroll_x: 0.0,
            scroll_y: 0.0,
            width: 800.0,
            height: 21_000.0,
            scale: 1.0,
        };
        let options = PaintOptions {
            show_grid: false,
            show_headers: false,
            ..PaintOptions::default()
        };
        group.bench_with_input(BenchmarkId::from_parameter(kind), &(), |b, ()| {
            let mut dl = DisplayList::new();
            b.iter(|| {
                paint_sheet(black_box(&book), sheet, viewport, &options, &mut dl);
                dl.len()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, conditional_formatting);
criterion_main!(benches);
