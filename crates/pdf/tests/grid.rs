//! Печатная сетка: `PageConfig::print_grid_lines` (задача I1 спринта 7).
//!
//! Сетка — линии по рёбрам ячеек области печати, те же, что у canvas-пути
//! (`xlsx::paint::draw_grid`): цвет `D9D9D9`, толщина в один пиксель раскладки,
//! координаты по `SheetLayout`. Источник решения при этом другой: canvas
//! смотрит на экранную настройку `view.show_grid_lines`, PDF — на печатную
//! `print_grid_lines`, как Excel.
//!
//! Проверки структурные: поток содержимого разбирается `lopdf`, из него
//! выбираются штрихи цвета сетки. Эталон координат не записан числами, а
//! считается из `SheetLayout` той же книги: golden-тест, снятый с отрисовки,
//! ловит изменение, но не дефект.

use std::path::{Path, PathBuf};

use doc_converter_pdf::{PageConfig, PdfExporter, PdfOptions};
use doc_converter_xlsx::layout::PX_PER_POINT;
use doc_converter_xlsx::{SheetLayout, Workbook};
use lopdf::content::{Content, Operation};
use lopdf::Object;

/// Точность сравнения координат — доли точки.
const EPS: f32 = 1e-3;

/// Цвет сетки `dl_color::GRID` из `xlsx::paint`, в долях единицы: `0xD9D9D9`.
const GRID_CHANNEL: f32 = 217.0 / 255.0;

/// Толщина линии сетки в точках: 1 px раскладки при 96 dpi.
const GRID_WIDTH_PT: f32 = 0.75;

/// Точки на миллиметр; у `PageConfig` поля в миллиметрах.
const PT_PER_MM: f32 = 72.0 / 25.4;

/// Высота A4 книжной ориентации в миллиметрах.
const A4_H_MM: f32 = 297.0;

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из общего набора фикстур.
fn open(name: &str) -> Workbook {
    doc_converter_xlsx::open(std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}")))
        .unwrap_or_else(|e| panic!("{name} не открылась: {e}"))
}

/// Экспортировать лист с включённой или выключенной печатной сеткой.
fn export(name: &str, sheet: usize, print_grid_lines: bool) -> Vec<u8> {
    let book = open(name);
    let page = PageConfig {
        print_grid_lines,
        ..PageConfig::default()
    };
    PdfExporter::new(PdfOptions {
        page,
        ..PdfOptions::default()
    })
    .export_xlsx_sheet(&book, sheet)
    .unwrap_or_else(|e| panic!("{name} не экспортировалась: {e}"))
}

/// Операции всех страниц: `get_page_content` распаковывает FlateDecode.
fn all_ops(bytes: &[u8]) -> Vec<Operation> {
    let doc = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    let mut ops = Vec::new();
    for &page_id in doc.get_pages().values() {
        let bytes = doc.get_page_content(page_id).expect("поток содержимого");
        let content = Content::decode(&bytes).expect("операции разбираются");
        ops.extend(content.operations);
    }
    ops
}

/// Число из операнда.
fn number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("координата не число: {other:?}"),
    }
}

/// Отрезок, как он записан в потоке: концы в точках PDF и состояние кисти.
#[derive(Debug, Clone, Copy)]
struct Stroke {
    from: (f32, f32),
    to: (f32, f32),
    rgb: Option<(f32, f32, f32)>,
    width: f32,
}

/// Штрихи потока: пути из двух точек, замкнутые оператором `S`.
///
/// Состояние кисти (`RG`, `w`) переносится на следующие за ним пути: printpdf
/// не повторяет цвет перед каждой линией.
fn strokes(ops: &[Operation]) -> Vec<Stroke> {
    let mut out = Vec::new();
    let mut rgb = None;
    let mut width = 1.0;
    let mut path: Vec<(f32, f32)> = Vec::new();
    for op in ops {
        match op.operator.as_str() {
            "RG" => {
                rgb = Some((
                    number(&op.operands[0]),
                    number(&op.operands[1]),
                    number(&op.operands[2]),
                ));
            }
            "w" => width = number(&op.operands[0]),
            "m" => {
                path.clear();
                path.push((number(&op.operands[0]), number(&op.operands[1])));
            }
            "l" => path.push((number(&op.operands[0]), number(&op.operands[1]))),
            "S" => {
                if let [from, to] = path[..] {
                    out.push(Stroke {
                        from,
                        to,
                        rgb,
                        width,
                    });
                }
                path.clear();
            }
            // Заливки и отсечения закрывают путь без штриха.
            "f" | "f*" | "B" | "B*" | "n" => path.clear(),
            _ => {}
        }
    }
    out
}

/// Есть ли у штриха цвет сетки.
fn is_grid(stroke: &Stroke) -> bool {
    stroke.rgb.is_some_and(|(r, g, b)| {
        (r - GRID_CHANNEL).abs() < EPS
            && (g - GRID_CHANNEL).abs() < EPS
            && (b - GRID_CHANNEL).abs() < EPS
    })
}

/// Штрихи цвета сетки.
fn grid_strokes(ops: &[Operation]) -> Vec<Stroke> {
    strokes(ops).into_iter().filter(is_grid).collect()
}

/// Горизонтальные и вертикальные штрихи по отдельности.
fn split(strokes: &[Stroke]) -> (Vec<Stroke>, Vec<Stroke>) {
    strokes
        .iter()
        .copied()
        .partition(|stroke| (stroke.from.1 - stroke.to.1).abs() < EPS)
}

/// Отсортированные координаты: сравнение множеств, а не порядка отрисовки.
fn sorted(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    values
}

#[track_caller]
fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < EPS,
        "ожидалось {expected}, получено {actual}"
    );
}

/// Число и координаты линий сетки — рёбра `SheetLayout` области печати.
///
/// Эталон считается из раскладки, а не берётся записанным: у листа без границ
/// и заливок штрихи в потоке — только сетка, и они обязаны лечь ровно на
/// границы столбцов и строк, включая замыкающие рёбра последней строки и
/// последнего столбца.
#[test]
fn grid_lines_follow_layout_edges() {
    let name = "content-mixed-types.xlsx";
    let book = open(name);
    let sheet = book.sheets().first().expect("лист есть");
    let layout = SheetLayout::new(sheet);
    let used = sheet.cells.used_range().expect("лист не пуст");
    let bytes = export(name, 0, true);
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается");
    assert_eq!(
        doc.get_pages().len(),
        1,
        "лист должен уместиться на страницу"
    );

    let (horizontal, vertical) = split(&grid_strokes(&all_ops(&bytes)));
    let rows = used.last.row - used.first.row + 2;
    let cols = used.last.col - used.first.col + 2;
    assert_eq!(horizontal.len(), rows as usize, "горизонтальных линий");
    assert_eq!(vertical.len(), cols as usize, "вертикальных линий");

    // Геометрия страницы по умолчанию: поля 20/15/20/15 мм, масштаб 100%.
    let page = PageConfig::default();
    let margins = page.margins;
    let origin_x = margins.left_mm * PT_PER_MM;
    let origin_y = margins.top_mm * PT_PER_MM;
    let height = A4_H_MM * PT_PER_MM;
    let px_per_pt = PX_PER_POINT / page.scale;
    let x_of = |col: u32| origin_x + layout.column_x(col) / px_per_pt;
    // В PDF начало координат в левом нижнем углу, у раскладки — в левом верхнем.
    let y_of = |row: u32| height - (origin_y + layout.row_y(row) / px_per_pt);

    // Каждая линия идёт от края до края области печати…
    let left = x_of(used.first.col);
    let right = x_of(used.last.col + 1);
    for stroke in &horizontal {
        close(stroke.from.1, stroke.to.1);
        close(stroke.from.0.min(stroke.to.0), left);
        close(stroke.from.0.max(stroke.to.0), right);
    }

    let top = y_of(used.first.row);
    let bottom = y_of(used.last.row + 1);
    for stroke in &vertical {
        close(stroke.from.0, stroke.to.0);
        close(stroke.from.1.min(stroke.to.1), bottom);
        close(stroke.from.1.max(stroke.to.1), top);
    }

    // …а их координаты — рёбра строк и столбцов раскладки. Сравниваются
    // множества, а не порядок отрисовки: он ничем не зафиксирован.
    let rows_expected = sorted((used.first.row..=used.last.row + 1).map(y_of).collect());
    let cols_expected = sorted((used.first.col..=used.last.col + 1).map(x_of).collect());
    for (actual, expected) in sorted(horizontal.iter().map(|s| s.from.1).collect())
        .iter()
        .zip(&rows_expected)
    {
        close(*actual, *expected);
    }
    for (actual, expected) in sorted(vertical.iter().map(|s| s.from.0).collect())
        .iter()
        .zip(&cols_expected)
    {
        close(*actual, *expected);
    }
}

/// При `print_grid_lines: false` сетки нет, а рамки ячеек остаются.
///
/// Сетка и рамка — разные сущности: настройка печати гасит только сетку.
#[test]
fn disabled_grid_keeps_cell_borders() {
    let name = "content-styles-only.xlsx";
    let with = all_ops(&export(name, 0, true));
    let without = all_ops(&export(name, 0, false));

    assert!(
        !grid_strokes(&with).is_empty(),
        "с включённой сеткой линий нет"
    );
    assert!(
        grid_strokes(&without).is_empty(),
        "с выключенной сеткой линии остались"
    );

    let borders = |ops: &[Operation]| strokes(ops).iter().filter(|s| !is_grid(s)).count();
    assert!(borders(&without) > 0, "рамки ячеек пропали");
    assert_eq!(
        borders(&with),
        borders(&without),
        "сетка изменила число рамок: рамки рисуются независимо"
    );
}

/// Цвет сетки — `D9D9D9`, толщина — 1 px раскладки (0.75 pt): как у canvas.
#[test]
fn grid_strokes_use_excel_color_and_hairline_width() {
    let grid = grid_strokes(&all_ops(&export("content-mixed-types.xlsx", 0, true)));
    assert!(!grid.is_empty(), "линий сетки нет");
    for stroke in &grid {
        let (r, g, b) = stroke.rgb.expect("у штриха сетки задан цвет");
        close(r, GRID_CHANNEL);
        close(g, GRID_CHANNEL);
        close(b, GRID_CHANNEL);
        close(stroke.width, GRID_WIDTH_PT);
    }
}

/// Лист без данных: экспорт не падает, сетки нет — рисовать нечего.
#[test]
fn empty_sheet_exports_without_grid_lines() {
    let bytes = export("content-empty-sheet.xlsx", 0, true);
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается");
    assert!(!doc.get_pages().is_empty(), "страница есть");
    assert!(
        grid_strokes(&all_ops(&bytes)).is_empty(),
        "на пустом листе сетка не рисуется"
    );
}
