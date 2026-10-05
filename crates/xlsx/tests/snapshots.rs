//! Snapshot-тесты `DisplayList` на фикстурах exceljs.
//!
//! Кадр сериализуется в компактный текстовый дамп команд: регресс в раскладке,
//! стилях или тексте виден обычным дифом снэпшота `insta`, а не пиксельным
//! сравнением. Фикстуры — те же книги из `test-fixtures/xlsx`, что и в
//! дифференциальном тесте; здесь проверяется не разбор значений, а то, как лист
//! превращается в кадр.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use doc_converter_render::display_list::{DisplayList, DrawCommand};
use doc_converter_xlsx::{open, paint_sheet, PaintOptions};

/// Подмножество фикстур по одному представителю на класс сценария:
/// условное форматирование, контент, края, форматы, картинки, раскладка,
/// стили, значения.
const FIXTURES: &[&str] = &[
    "cf-cell-is",
    "cf-data-bar",
    "cf-icon-set",
    "content-dense",
    "content-rich-text",
    "content-table",
    "edge-merged-large",
    "edge-zero-width-column",
    "formats-currency-ruble",
    "formats-date-ru",
    "formats-percent",
    "images-over-data",
    "images-png",
    "layout-frozen-both",
    "layout-links",
    "layout-merged",
    "styles-border-styles",
    "styles-pattern-fills",
    "values-long-text",
    "values-unicode",
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/xlsx")
}

/// Дамп кадра: по строке на команду, строки пула разрешены в текст.
///
/// Длинные строки обрезаются: полный текст уже проверяется дифференциальным
/// тестом, а снэпшоту важна форма кадра, а не каждый символ.
fn dump(dl: &DisplayList) -> String {
    let mut out = String::with_capacity(4096);
    for i in 0..dl.len() {
        let cmd = dl.cmd(i).expect("команда в пределах списка");
        match cmd {
            DrawCommand::Clear => out.push_str("Clear\n"),
            DrawCommand::Rect {
                x,
                y,
                w,
                h,
                fill,
                stroke,
                stroke_w,
                radius,
            } => {
                let _ = writeln!(
                    out,
                    "Rect x={x} y={y} w={w} h={h} fill={fill:?} stroke={stroke:?} sw={stroke_w} r={radius:?}"
                );
            }
            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                stroke_w,
                style,
            } => {
                let _ = writeln!(
                    out,
                    "Line x1={x1} y1={y1} x2={x2} y2={y2} stroke={stroke:?} sw={stroke_w} style={style:?}"
                );
            }
            DrawCommand::Text {
                x,
                y,
                text,
                font,
                size,
                color,
                align,
                baseline,
                bold,
                italic,
                underline,
            } => {
                let text = dl.string(*text);
                let font = dl.string(*font);
                let _ = writeln!(
                    out,
                    "Text x={x} y={y} text={text:?} font={font:?} size={size} color={color:?} align={align:?} baseline={baseline:?} bold={bold} italic={italic} underline={underline}"
                );
            }
            DrawCommand::Image {
                x,
                y,
                w,
                h,
                bitmap_id,
            } => {
                let _ = writeln!(out, "Image x={x} y={y} w={w} h={h} id={bitmap_id}");
            }
            DrawCommand::Chart { x, y, w, h, .. } => {
                let _ = writeln!(out, "Chart x={x} y={y} w={w} h={h}");
            }
            DrawCommand::PushClip { x, y, w, h } => {
                let _ = writeln!(out, "PushClip x={x} y={y} w={w} h={h}");
            }
            DrawCommand::PopClip => out.push_str("PopClip\n"),
            DrawCommand::PushTransform { a, b, c, d, e, f } => {
                let _ = writeln!(out, "PushTransform a={a} b={b} c={c} d={d} e={e} f={f}");
            }
            DrawCommand::PopTransform => out.push_str("PopTransform\n"),
        }
    }
    out
}

/// Первый лист фикстуры в кадр: вьюпорт и оформление — по умолчанию.
fn render_first_sheet(name: &str) -> String {
    let path = fixtures_dir().join(format!("{name}.xlsx"));
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("{} не читается: {e}", path.display()));
    let book = open(bytes).expect("фикстура открывается");
    let sheet = book.sheets().first().expect("у фикстуры есть первый лист");
    let mut dl = DisplayList::new();
    paint_sheet(
        &book,
        sheet,
        Default::default(),
        &PaintOptions::default(),
        &mut dl,
    );
    dump(&dl)
}

#[test]
fn display_list_snapshots() {
    for name in FIXTURES {
        let dump = render_first_sheet(name);
        insta::with_settings!({ snapshot_suffix => *name }, {
            insta::assert_snapshot!(dump);
        });
    }
}
