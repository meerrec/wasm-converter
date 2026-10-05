//! Дифференциальный тест на фикстурах exceljs.
//!
//! `test-fixtures/xlsx/` собирается скриптом `scripts/gen-fixtures.ts`: книги
//! пишет exceljs, он же читает их обратно и складывает значения ячеек в
//! `oracle.json`. Здесь тот же файл разбирается нашим парсером, и два
//! независимых разбора сверяются — по значениям, формулам, гиперссылкам и
//! видимости листов.
//!
//! Проверка идёт в обе стороны: каждая ячейка эталона должна найтись у нас, а
//! каждая наша непустая ячейка — в эталоне. Односторонняя проверка пропускала
//! бы лишние ячейки, которых в файле нет.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use doc_converter_render::display_list::{
    Color as PixelColor, DisplayList, DrawCommand, TextAlign,
};
use doc_converter_xlsx::cellref::{CellRef, Range};
use doc_converter_xlsx::{
    open, paint_sheet, BorderStyle, CellIsOperator, CellValue, Color, ColorScale, ConditionalRule,
    DataBar, Fill, FillPattern, IconSet, PaintOptions, PaneKind, PaneState, RuleKind, Sheet,
    SheetLayout, SheetState, Threshold, ThresholdKind, Viewport, Workbook, XlsxError,
};
use serde_json::Value;

/// Каталог с фикстурами и эталоном.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/xlsx")
}

fn oracle() -> Value {
    let path = fixtures_dir().join("oracle.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} не читается: {e}", path.display()));
    serde_json::from_str(&text).expect("эталон — корректный JSON")
}

/// Имена книг в каталоге фикстур, по возрастанию.
fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("каталог фикстур")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().into_owned();
            name.ends_with(".xlsx").then_some(name)
        })
        .collect();
    names.sort();
    names
}

fn state_of(name: &str) -> SheetState {
    match name {
        "hidden" => SheetState::Hidden,
        "veryHidden" => SheetState::VeryHidden,
        _ => SheetState::Visible,
    }
}

/// Текст ячейки нашего разбора — то, что показывает Excel.
fn text_of<'a>(book: &'a Workbook, value: &'a CellValue) -> Option<&'a str> {
    value.text(book.shared_strings())
}

/// Сравнить одну ячейку эталона с нашей.
fn check_cell(book: &Workbook, file: &str, sheet: &doc_converter_xlsx::Sheet, record: &Value) {
    let fields = record.as_array().expect("ячейка эталона — массив");
    let index = |i: usize| fields.get(i);
    let row = index(0).and_then(Value::as_u64).expect("строка") as u32;
    let col = index(1).and_then(Value::as_u64).expect("столбец") as u32;
    let kind = index(2).and_then(Value::as_str).expect("вид значения");
    let at = CellRef::new(row, col);
    let where_ = format!("{file}: {at}");

    let cell = sheet.cells.cell(at);
    if kind == "-" && cell.is_none() {
        return;
    }
    let Some(cell) = cell else {
        panic!("{where_}: ячейки нет в разборе, а в эталоне есть");
    };

    match kind {
        "-" => assert_eq!(
            cell.value,
            CellValue::Empty,
            "{where_}: в эталоне значения нет"
        ),
        "n" => {
            let expected = index(3).and_then(Value::as_f64).expect("число");
            match cell.value {
                CellValue::Number(actual) => {
                    assert_eq!(actual, expected, "{where_}: число разошлось с эталоном")
                }
                ref other => panic!("{where_}: ожидалось число {expected}, а разобрано {other:?}"),
            }
        }
        "s" => {
            let expected = index(3).and_then(Value::as_str).expect("строка");
            assert_eq!(
                text_of(book, &cell.value),
                Some(expected),
                "{where_}: текст разошёлся с эталоном"
            );
        }
        "b" => {
            let expected = index(3).and_then(Value::as_bool).expect("логическое");
            assert_eq!(
                cell.value,
                CellValue::Bool(expected),
                "{where_}: логическое разошлось с эталоном"
            );
        }
        "e" => {
            let expected = index(3).and_then(Value::as_str).expect("код ошибки");
            assert_eq!(
                text_of(book, &cell.value),
                Some(expected),
                "{where_}: код ошибки разошёлся с эталоном"
            );
        }
        other => panic!("{where_}: неизвестный вид значения `{other}`"),
    }

    if let Some(formula) = index(4).and_then(Value::as_str) {
        assert_eq!(
            cell.formula.as_deref(),
            Some(formula),
            "{where_}: текст формулы разошёлся с эталоном"
        );
    }

    if let Some(link) = index(5).and_then(Value::as_str) {
        let hyperlink = sheet
            .hyperlink_at(at)
            .unwrap_or_else(|| panic!("{where_}: гиперссылки нет в разборе"));
        // exceljs показывает внутренние переходы как `#Лист2!A1`.
        let expected = link.strip_prefix('#').unwrap_or(link);
        assert_eq!(
            hyperlink.target.address(),
            Some(expected),
            "{where_}: цель гиперссылки разошлась с эталоном"
        );
    }
}

fn check_sheet(book: &Workbook, file: &str, expected: &Value) {
    let name = expected["name"].as_str().expect("имя листа");
    let sheet = book
        .sheet(name)
        .unwrap_or_else(|| panic!("{file}: лист `{name}` не найден"));
    assert_eq!(
        sheet.meta.state,
        state_of(expected["state"].as_str().unwrap_or("visible")),
        "{file}: видимость листа `{name}`"
    );

    let records = expected["cells"].as_array().expect("ячейки листа");
    let mut expected_cells = HashSet::new();
    for record in records {
        let fields = record.as_array().expect("ячейка эталона");
        let row = fields[0].as_u64().expect("строка") as u32;
        let col = fields[1].as_u64().expect("столбец") as u32;
        expected_cells.insert((row, col));
        check_cell(book, file, sheet, record);
    }

    // Обратная проверка: непустых ячеек, которых нет в эталоне, быть не должно.
    for (row, cells) in sheet.cells.rows() {
        for cell in cells {
            if cell.value == CellValue::Empty {
                continue;
            }
            assert!(
                expected_cells.contains(&(row, cell.col)),
                "{file}: лист `{name}`, ячейка {}:{} есть в разборе, но её нет в эталоне",
                row + 1,
                cell.col + 1
            );
        }
    }
}

#[test]
fn every_fixture_parses_like_the_oracle() {
    let oracle = oracle();
    let files = oracle["files"].as_object().expect("файлы эталона");
    let names = fixture_names();

    // Эталон и каталог обязаны совпадать: иначе тест молча проверял бы не всё.
    assert_eq!(
        names.len(),
        files.len(),
        "фикстур {names_len}, а в эталоне {files_len}; перегенерируйте: node scripts/gen-fixtures.ts",
        names_len = names.len(),
        files_len = files.len()
    );

    for name in &names {
        let path = fixtures_dir().join(name);
        let bytes = std::fs::read(&path).expect("фикстура читается");
        let book = open(bytes).unwrap_or_else(|e| panic!("{name}: {e}\n{e:?}"));

        let expected = files
            .get(name)
            .unwrap_or_else(|| panic!("{name}: нет в эталоне"));
        let sheets = expected["sheets"].as_array().expect("листы эталона");
        assert_eq!(
            book.sheet_count(),
            sheets.len(),
            "{name}: число листов разошлось с эталоном"
        );
        for sheet in sheets {
            check_sheet(&book, name, sheet);
        }
    }
}

/// Тема есть во всех фикстурах, и `theme="1"` — это `dk1` (цвет текста), а не
/// `lt1`: если индексировать палитру в порядке элементов `<a:clrScheme>`, цвет
/// текста по умолчанию стал бы белым — текст исчез бы.
#[test]
fn every_fixture_resolves_the_default_text_color_to_black() {
    for name in fixture_names() {
        let book = open(std::fs::read(fixtures_dir().join(&name)).unwrap()).unwrap();
        assert_eq!(
            book.theme().color(1),
            Some(Color::Rgb(0xFF00_0000)),
            "{name}: theme=\"1\" должен разрешаться в чёрный"
        );
    }
}

/// Фикстуры с раскладкой проверяются отдельно: exceljs не отдаёт ни ширины
/// столбцов, ни закрепления в том же виде, что и файл, поэтому эталона на них
/// нет — зато есть точные ожидания.
#[test]
fn layout_fixtures_keep_their_geometry() {
    let cases = [
        ("layout-column-widths.xlsx", 0, 4.0),
        ("layout-column-widths.xlsx", 3, 30.0),
        ("layout-hidden-columns.xlsx", 1, 0.0),
        ("layout-hidden-columns.xlsx", 0, 8.43),
    ];
    for (file, col, width) in cases {
        let book = open(std::fs::read(fixtures_dir().join(file)).unwrap()).unwrap();
        let sheet = &book.sheets()[0];
        assert_eq!(sheet.dims.col_width(col), width, "{file}: столбец {col}");
    }

    let rows =
        open(std::fs::read(fixtures_dir().join("layout-row-heights.xlsx")).unwrap()).unwrap();
    let sheet = &rows.sheets()[0];
    assert_eq!(sheet.dims.row_height(0), 10.0);
    assert_eq!(sheet.dims.row_height(4), 80.0);
    // Строка без своей высоты берёт общую.
    assert_eq!(sheet.dims.row_height(20), 15.0);

    let hidden =
        open(std::fs::read(fixtures_dir().join("layout-hidden-rows.xlsx")).unwrap()).unwrap();
    assert_eq!(hidden.sheets()[0].dims.row_height(1), 0.0);

    let merged = open(std::fs::read(fixtures_dir().join("layout-merged.xlsx")).unwrap()).unwrap();
    let merges = &merged.sheets()[0].merges;
    assert_eq!(merges.len(), 3);
    assert!(merges.anchored_at(CellRef::new(0, 0)).is_some());
    // C1:D2 накрывает D2, но не C3.
    assert!(merges.covering(CellRef::new(1, 3)).is_some());
    assert!(merges.covering(CellRef::new(2, 3)).is_none());

    for (file, cols, rows, active) in [
        ("layout-frozen-rows.xlsx", 0, 1, PaneKind::BottomLeft),
        ("layout-frozen-columns.xlsx", 1, 0, PaneKind::TopRight),
        ("layout-frozen-both.xlsx", 2, 2, PaneKind::BottomRight),
    ] {
        let book = open(std::fs::read(fixtures_dir().join(file)).unwrap()).unwrap();
        let pane = book.sheets()[0]
            .view
            .pane
            .unwrap_or_else(|| panic!("{file}: закрепления нет"));
        assert_eq!(pane.cols, cols, "{file}");
        assert_eq!(pane.rows, rows, "{file}");
        assert_eq!(pane.state, PaneState::Frozen, "{file}");
        assert_eq!(pane.active, active, "{file}");
    }

    let plain = open(std::fs::read(fixtures_dir().join("layout-no-grid.xlsx")).unwrap()).unwrap();
    assert!(!plain.sheets()[0].view.show_grid_lines);
    assert_eq!(plain.sheets()[0].view.zoom, 85);
}

/// Ссылки фикстуры видны в кадре как ссылки: exceljs объявляет `<hyperlink>`,
/// но встроенный стиль «Hyperlink» к ячейкам не прикладывает, поэтому
/// подчёркивание и цвет `hlink` достраивает painter.
#[test]
fn layout_links_are_painted_as_links() {
    let book = open(std::fs::read(fixtures_dir().join("layout-links.xlsx")).unwrap()).unwrap();
    let sheet = book.sheet("Ссылки").expect("лист фикстуры");

    let mut dl = DisplayList::new();
    paint_sheet(
        &book,
        sheet,
        Viewport::default(),
        &PaintOptions {
            show_grid: false,
            show_headers: false,
            ..PaintOptions::default()
        },
        &mut dl,
    );

    let texts: Vec<(String, bool, PixelColor)> = (0..dl.len())
        .filter_map(|i| dl.cmd(i))
        .filter_map(|cmd| match cmd {
            DrawCommand::Text {
                text,
                underline,
                color,
                ..
            } => Some((dl.string(*text).to_owned(), *underline, *color)),
            _ => None,
        })
        .collect();

    // A1–A4 — ссылки: внешние и переход на второй лист; B1 — обычный текст.
    for link in ["Example", "С параметрами", "Почта", "На второй лист"]
    {
        let (_, underline, color) = texts
            .iter()
            .find(|(text, ..)| text == link)
            .unwrap_or_else(|| panic!("{link}: текста нет в кадре"));
        assert!(*underline, "{link}: ссылка не подчёркнута");
        // hlink темы фикстуры — 0000FF; в DisplayList каналы уже RRGGBBAA.
        assert_eq!(*color, PixelColor(0x0000_FFFF), "{link}: цвет ссылки");
    }

    let (_, underline, color) = texts
        .iter()
        .find(|(text, ..)| text == "рядом")
        .expect("B1: текста нет в кадре");
    assert!(!*underline, "ячейка без ссылки не подчёркнута");
    assert_eq!(*color, PixelColor::BLACK, "ячейка без ссылки — чёрная");
}

/// Границы листа: `<dimension>` из файла и фактические не обязаны совпадать,
/// поэтому проверяется именно фактический размах.
#[test]
fn extreme_cells_are_reachable() {
    let book = open(std::fs::read(fixtures_dir().join("edge-max-cell.xlsx")).unwrap()).unwrap();
    let sheet = &book.sheets()[0];
    let corner = sheet
        .cells
        .cell(CellRef::new(1_048_575, 16_383))
        .expect("ячейка XFD1048576");
    assert_eq!(
        corner.value.text(book.shared_strings()),
        Some("угол"),
        "правый нижний угол листа"
    );
    assert_eq!(
        sheet.cells.used_range().unwrap().last,
        CellRef::new(1_048_575, 16_383)
    );
}

#[test]
fn corrupt_packages_fail_loudly() {
    // Обрезанный архив и не архив вовсе — единственные фикстуры, которые
    // обязаны падать: остальное чинится в разборе.
    let dir = fixtures_dir();
    let truncated = std::fs::read(dir.join("edge-many-rows.xlsx")).unwrap();
    let half = &truncated[..truncated.len() / 2];
    assert!(matches!(open(half.to_vec()), Err(XlsxError::Core(_))));

    assert!(matches!(
        open(b"not a zip at all".to_vec()),
        Err(XlsxError::Core(_))
    ));
}

/// Открыть книгу фикстуры условного форматирования.
fn open_fixture(name: &str) -> Workbook {
    let path = fixtures_dir().join(name);
    open(std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Диапазоны `sqref` так, как их записывает Excel: через пробел.
fn ranges(value: &str) -> Vec<Range> {
    value
        .split_whitespace()
        .map(|token| Range::parse_ref(token).unwrap())
        .collect()
}

fn cell_is(operator: CellIsOperator, formulas: &[&str]) -> RuleKind {
    RuleKind::CellIs {
        operator,
        formulas: formulas.iter().map(|text| (*text).to_owned()).collect(),
    }
}

fn expression(formulas: &[&str]) -> RuleKind {
    RuleKind::Expression {
        formulas: formulas.iter().map(|text| (*text).to_owned()).collect(),
    }
}

fn threshold(kind: ThresholdKind, value: Option<f64>) -> Threshold {
    Threshold {
        kind,
        value,
        gte: true,
    }
}

fn assert_rule(rule: &ConditionalRule, priority: u32, dxf_id: Option<u32>, kind: RuleKind) {
    assert_eq!(rule.priority, priority, "приоритет правила: {rule:?}");
    assert_eq!(rule.dxf_id, dxf_id, "ссылка на dxf: {rule:?}");
    assert_eq!(rule.kind, kind, "содержимое правила");
    assert!(!rule.stop_if_true, "stopIfTrue не ожидался: {rule:?}");
}

fn color_scale(sheet: &Sheet, block: usize) -> &ColorScale {
    match &sheet.conditional_formatting[block].rules[0].kind {
        RuleKind::ColorScale(scale) => scale,
        other => panic!("блок {block}: ожидалась шкала, а разобрано {other:?}"),
    }
}

fn data_bar(sheet: &Sheet, block: usize) -> &DataBar {
    match &sheet.conditional_formatting[block].rules[0].kind {
        RuleKind::DataBar(bar) => bar,
        other => panic!("блок {block}: ожидалась гистограмма, а разобрано {other:?}"),
    }
}

fn icon_set(sheet: &Sheet, block: usize) -> &IconSet {
    match &sheet.conditional_formatting[block].rules[0].kind {
        RuleKind::IconSet(set) => set,
        other => panic!("блок {block}: ожидался набор значков, а разобрано {other:?}"),
    }
}

#[test]
fn cf_cell_is_rules_reach_the_model() {
    let book = open_fixture("cf-cell-is.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 2);
    let first = &sheet.conditional_formatting[0];
    assert_eq!(first.ranges, ranges("A1:A12"));
    assert_eq!(first.rules.len(), 3);
    assert_rule(
        &first.rules[0],
        1,
        Some(0),
        cell_is(CellIsOperator::GreaterThan, &["30"]),
    );
    assert_rule(
        &first.rules[1],
        2,
        Some(1),
        cell_is(CellIsOperator::LessThanOrEqual, &["20"]),
    );
    assert_rule(
        &first.rules[2],
        3,
        Some(2),
        cell_is(CellIsOperator::Between, &["25", "45"]),
    );

    let second = &sheet.conditional_formatting[1];
    assert_eq!(second.ranges, ranges("C1:C12"));
    assert_eq!(second.rules.len(), 4);
    assert_rule(
        &second.rules[0],
        4,
        Some(3),
        cell_is(CellIsOperator::Equal, &["0.5"]),
    );
    assert_rule(
        &second.rules[1],
        5,
        Some(4),
        cell_is(CellIsOperator::NotEqual, &["0.25"]),
    );
    assert_rule(
        &second.rules[2],
        6,
        Some(5),
        cell_is(CellIsOperator::GreaterThanOrEqual, &["0.75"]),
    );
    assert_rule(
        &second.rules[3],
        7,
        Some(6),
        cell_is(CellIsOperator::NotBetween, &["0.3", "0.7"]),
    );

    // Дифференциальные форматы, на которые ссылаются правила.
    let styles = book.styles();
    assert_eq!(styles.dxfs().count(), 7);
    assert_eq!(
        styles.dxf(0).unwrap().fill.as_ref().unwrap(),
        &Fill {
            pattern: FillPattern::Solid,
            foreground: Color::Rgb(0xFFFF_C7CE),
            ..Fill::default()
        }
    );
    let font = styles.dxf(3).unwrap().font.as_ref().unwrap();
    assert!(font.bold);
    assert_eq!(font.color, Color::Rgb(0xFF9C_0006));
    let border = styles.dxf(5).unwrap().border.as_ref().unwrap();
    assert_eq!(border.top.style, BorderStyle::Thin);
    assert_eq!(border.top.color, Color::Rgb(0xFF00_0000));
}

#[test]
fn cf_expression_rules_reach_the_model() {
    let book = open_fixture("cf-expression.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 2);
    let first = &sheet.conditional_formatting[0];
    assert_eq!(first.ranges, ranges("B2:B13"));
    assert_rule(&first.rules[0], 1, Some(0), expression(&["MOD($B2,2)=0"]));

    let second = &sheet.conditional_formatting[1];
    assert_eq!(second.ranges, ranges("A2:B13"));
    assert_rule(&second.rules[0], 2, Some(1), expression(&["$B2=0"]));
    // `&gt;` в файле — это `>` в модели.
    assert_rule(&second.rules[1], 3, Some(2), expression(&["LEN($A2)>10"]));
}

#[test]
fn cf_color_scale_fixture_reaches_the_model() {
    let book = open_fixture("cf-color-scale.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 3);
    assert_eq!(
        color_scale(sheet, 0).thresholds,
        vec![
            threshold(ThresholdKind::Min, None),
            threshold(ThresholdKind::Max, None),
        ]
    );
    assert_eq!(
        color_scale(sheet, 0).colors,
        vec![Color::Rgb(0xFFF8_696B), Color::Rgb(0xFF63_BE7B)]
    );

    assert_eq!(
        color_scale(sheet, 1).thresholds,
        vec![
            threshold(ThresholdKind::Min, None),
            threshold(ThresholdKind::Percentile, Some(50.0)),
            threshold(ThresholdKind::Max, None),
        ]
    );
    assert_eq!(color_scale(sheet, 1).colors.len(), 3);

    assert_eq!(
        color_scale(sheet, 2).thresholds,
        vec![
            threshold(ThresholdKind::Number, Some(0.1)),
            threshold(ThresholdKind::Percent, Some(50.0)),
            threshold(ThresholdKind::Number, Some(1.0)),
        ]
    );
    assert_eq!(
        color_scale(sheet, 2).colors,
        vec![
            Color::Rgb(0xFFFF_0000),
            Color::Rgb(0xFFFF_FFFF),
            Color::Rgb(0xFF00_00FF),
        ]
    );

    // Визуальные правила не ссылаются на `dxf`.
    for block in &sheet.conditional_formatting {
        assert_eq!(block.rules[0].dxf_id, None);
    }
    assert_eq!(book.styles().dxfs().count(), 0);
}

#[test]
fn cf_data_bar_fixture_reaches_the_model() {
    let book = open_fixture("cf-data-bar.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 2);
    assert_eq!(
        data_bar(sheet, 0).thresholds,
        vec![
            threshold(ThresholdKind::Min, None),
            threshold(ThresholdKind::Max, None),
        ]
    );
    assert_eq!(data_bar(sheet, 0).color, Color::Rgb(0xFF63_8EC6));
    assert!(data_bar(sheet, 0).show_value);

    // Отрицательные значения: границы заданы числами.
    assert_eq!(
        data_bar(sheet, 1).thresholds,
        vec![
            threshold(ThresholdKind::Number, Some(-50.0)),
            threshold(ThresholdKind::Number, Some(70.0)),
        ]
    );
    assert_eq!(data_bar(sheet, 1).color, Color::Rgb(0xFF63_BE7B));
    assert_eq!(book.styles().dxfs().count(), 0);
}

#[test]
fn cf_icon_set_fixture_reaches_the_model() {
    let book = open_fixture("cf-icon-set.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 3);

    assert_eq!(icon_set(sheet, 0).icon_set, "3TrafficLights1");
    assert!(!icon_set(sheet, 0).reverse);
    assert!(icon_set(sheet, 0).show_value);
    assert_eq!(
        icon_set(sheet, 0).thresholds,
        vec![
            threshold(ThresholdKind::Percent, Some(0.0)),
            threshold(ThresholdKind::Percent, Some(33.0)),
            threshold(ThresholdKind::Percent, Some(67.0)),
        ]
    );

    assert_eq!(icon_set(sheet, 1).icon_set, "4Arrows");
    assert!(icon_set(sheet, 1).reverse);
    assert_eq!(icon_set(sheet, 1).thresholds.len(), 4);

    assert_eq!(icon_set(sheet, 2).icon_set, "5Quarters");
    assert!(!icon_set(sheet, 2).show_value);
    assert_eq!(icon_set(sheet, 2).thresholds.len(), 5);
}

#[test]
fn cf_multi_range_fixture_reaches_the_model() {
    let book = open_fixture("cf-multi-range.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 2);
    let first = &sheet.conditional_formatting[0];
    assert_eq!(first.ranges, ranges("A1:A10 C1:C10 E1:E10"));
    assert_rule(
        &first.rules[0],
        1,
        Some(0),
        cell_is(CellIsOperator::GreaterThan, &["55"]),
    );

    let second = &sheet.conditional_formatting[1];
    assert_eq!(second.ranges, ranges("B1:B10 D1:D10"));
    assert_rule(
        &second.rules[0],
        2,
        Some(1),
        expression(&["MOD(ROW(),2)=0"]),
    );
    // У правила-шкалы нет ни `dxfId`, ни содержимого: в файле только `type`.
    assert_eq!(second.rules[1].priority, 3);
    assert_eq!(second.rules[1].dxf_id, None);
    assert!(matches!(second.rules[1].kind, RuleKind::ColorScale(_)));
}

#[test]
fn cf_priorities_fixture_keeps_file_order_and_stop_if_true() {
    let book = open_fixture("cf-priorities.xlsx");
    let sheet = &book.sheets()[0];

    assert_eq!(sheet.conditional_formatting.len(), 3);
    let first = &sheet.conditional_formatting[0];
    // Порядок в файле (9, 5, 7) не совпадает с порядком применения, и модель
    // его не переставляет: сортировка — забота потребителя.
    let priorities: Vec<u32> = first.rules.iter().map(|rule| rule.priority).collect();
    assert_eq!(priorities, [9, 5, 7]);
    assert!(!first.rules[0].stop_if_true);
    assert!(first.rules[1].stop_if_true);
    assert!(!first.rules[2].stop_if_true);

    assert_rule(
        &sheet.conditional_formatting[1].rules[0],
        12,
        Some(3),
        expression(&["MOD($A1,2)=0"]),
    );
    assert_rule(
        &sheet.conditional_formatting[2].rules[0],
        3,
        Some(4),
        cell_is(CellIsOperator::LessThan, &["15"]),
    );
}

// --- Визуальные правила: что попадает в кадр --------------------------------

/// Кадр первого листа фикстуры без заголовков и сетки.
///
/// Без сетки и заголовков прямоугольники ячеек совпадают с раскладкой, и
/// заливку шкалы ничто не перекрывает.
fn painted_fixture(name: &str) -> (Workbook, DisplayList) {
    let book = open_fixture(name);
    let sheet = &book.sheets()[0];
    let mut dl = DisplayList::new();
    paint_sheet(
        &book,
        sheet,
        Viewport::default(),
        &PaintOptions {
            show_grid: false,
            show_headers: false,
            ..PaintOptions::default()
        },
        &mut dl,
    );
    (book, dl)
}

/// Прямоугольники кадра: `(x, y, ширина, высота, цвет)`.
fn rects(dl: &DisplayList) -> Vec<(f32, f32, f32, f32, PixelColor)> {
    (0..dl.len())
        .filter_map(|i| dl.cmd(i))
        .filter_map(|cmd| match cmd {
            DrawCommand::Rect {
                x, y, w, h, fill, ..
            } => Some((*x, *y, *w, *h, *fill)),
            _ => None,
        })
        .collect()
}

/// Прямоугольники заданного цвета, в порядке отрисовки.
fn rects_with(dl: &DisplayList, color: PixelColor) -> Vec<(f32, f32, f32, f32)> {
    rects(dl)
        .into_iter()
        .filter(|(.., fill)| *fill == color)
        .map(|(x, y, w, h, _)| (x, y, w, h))
        .collect()
}

/// Заливка ячейки (row, col) — прямоугольник точно по её границам.
fn cell_fill(
    dl: &DisplayList,
    layout: &SheetLayout,
    row: u32,
    col: u32,
) -> Option<(f32, f32, f32, f32, PixelColor)> {
    let x = layout.column_x(col);
    let y = layout.row_y(row);
    let w = layout.column_width(col);
    rects(dl).into_iter().find(|(rx, ry, rw, ..)| {
        (rx - x).abs() < 0.01 && (ry - y).abs() < 0.01 && (rw - w).abs() < 0.01
    })
}

/// Текстовые команды кадра: `(текст, x, y, выравнивание, цвет)`.
fn texts(dl: &DisplayList) -> Vec<(String, f32, f32, TextAlign, PixelColor)> {
    (0..dl.len())
        .filter_map(|i| dl.cmd(i))
        .filter_map(|cmd| match cmd {
            DrawCommand::Text {
                text,
                x,
                y,
                align,
                color,
                ..
            } => Some((dl.string(*text).to_owned(), *x, *y, *align, *color)),
            _ => None,
        })
        .collect()
}

/// Номер команды с текстом ячейки — чтобы проверить порядок слоёв.
fn text_index(dl: &DisplayList, want: &str) -> usize {
    (0..dl.len())
        .find(|&i| {
            matches!(dl.cmd(i), Some(DrawCommand::Text { text, .. }) if dl.string(*text) == want)
        })
        .unwrap_or_else(|| panic!("в кадре нет текста {want}"))
}

/// Номер команды — первого прямоугольника с заданной заливкой.
fn rect_index(dl: &DisplayList, color: PixelColor) -> usize {
    (0..dl.len())
        .find(|&i| matches!(dl.cmd(i), Some(DrawCommand::Rect { fill, .. }) if *fill == color))
        .unwrap_or_else(|| panic!("в кадре нет прямоугольника цвета {color:?}"))
}

/// Значок ячейки (row, col): текст и цвет. Значок ищется по координате —
/// строка задаёт вертикаль, столбец горизонталь.
fn icon_at(dl: &DisplayList, layout: &SheetLayout, row: u32, col: u32) -> (String, PixelColor) {
    let y = layout.row_y(row) + layout.row_height(row) / 2.0;
    let (first, last) = (layout.column_x(col), layout.column_x(col + 1));
    texts(dl)
        .into_iter()
        .find(|(text, tx, ty, ..)| {
            matches!(text.as_str(), "●" | "▼" | "▲" | "★")
                && (ty - y).abs() < 0.5
                && *tx >= first
                && *tx < last
        })
        .map(|(text, _, _, _, color)| (text, color))
        .unwrap_or_else(|| panic!("нет значка в ячейке ({row}, {col})"))
}

#[test]
fn cf_color_scale_fixture_paints_the_cell_background() {
    let (book, dl) = painted_fixture("cf-color-scale.xlsx");
    let layout = SheetLayout::new(&book.sheets()[0]);

    // A1:A12 — два цвета, min → max. Крайние значения получают цвета порогов
    // точно, середина — интерполяцию между ними.
    let (.., first) = cell_fill(&dl, &layout, 0, 0).expect("A1: заливка шкалы");
    assert_eq!(first, PixelColor(0xF869_6BFF), "A1: цвет минимума");
    let (.., last) = cell_fill(&dl, &layout, 11, 0).expect("A12: заливка шкалы");
    assert_eq!(last, PixelColor(0x63BE_7BFF), "A12: цвет максимума");
    let (.., middle) = cell_fill(&dl, &layout, 5, 0).expect("A6: заливка шкалы");
    // 35 в шкале 10…65 — 0.4545 пути от F8696B к 63BE7B.
    assert_eq!(middle, PixelColor(0xB490_72FF), "A6: интерполяция");

    // B1:B12 — три цвета с процентилем: между min и max стоит жёлтый порог.
    // Процентиль 50 от 12 значений (40…95 по возрастанию) — это 67.5, а не 70:
    // `PERCENTILE.INC` берёт ранг 0.5 * (12 - 1) = 5.5, то есть 65 плюс
    // половина шага до 70. B6 = 70 выше порога, поэтому его цвет — интерполяция
    // жёлтого к зелёному на 1/11 пути: (70 - 67.5) / (95 - 67.5).
    let (.., middle) = cell_fill(&dl, &layout, 5, 1).expect("B6: заливка шкалы");
    assert_eq!(
        middle,
        PixelColor(0xF1E7_83FF),
        "B6: цвет выше порога-процентиля"
    );

    // C1:C12 — числовые пороги: ниже первого — красный, на верхнем — синий.
    let (.., low) = cell_fill(&dl, &layout, 0, 2).expect("C1: заливка шкалы");
    assert_eq!(low, PixelColor(0xFF00_00FF), "C1: цвет ниже первого порога");
    let (.., high) = cell_fill(&dl, &layout, 11, 2).expect("C12: заливка шкалы");
    assert_eq!(high, PixelColor(0x0000_FFFF), "C12: цвет верхнего порога");
}

#[test]
fn cf_data_bar_fixture_draws_bars_under_the_text() {
    let (book, dl) = painted_fixture("cf-data-bar.xlsx");
    let layout = SheetLayout::new(&book.sheets()[0]);
    let color = PixelColor(0x638E_C6FF);
    let inner = layout.column_width(0) - 2.0;

    // A1:A12 — все значения положительные: шкала от минимума к максимуму,
    // минимум получает полосу нулевой длины, максимум — всю ширину с полями.
    let bars = rects_with(&dl, color);
    assert_eq!(bars.len(), 11, "минимум полосы не получает");
    let (x, y, w, h) = *bars.last().expect("полоса A12");
    assert!(
        (x - 1.0).abs() < 0.01 && (w - inner).abs() < 0.01,
        "A12: полоса во всю ячейку с полями: x={x}, w={w}"
    );
    // Высота полосы — доля строки, по центру ячейки.
    assert!(h < layout.row_height(11), "полоса ниже строки: {h}");
    assert!(y > layout.row_y(11), "полоса не прижата к верху");

    // Полоса рисуется под текстом ячейки: её команда идёт раньше.
    assert!(
        rect_index(&dl, color) < text_index(&dl, "65"),
        "полоса A12 перекрывает текст"
    );

    // B1:B12 — есть отрицательные: полоса отсчитывается от нулевой оси.
    let color = PixelColor(0x63BE_7BFF);
    let bars = rects_with(&dl, color);
    // Ноль полосы не получает — она у него нулевой длины.
    assert_eq!(bars.len(), 11, "полосу получает каждая ячейка, кроме нуля");
    let column = layout.column_x(1);
    let axis = column + 1.0 + 50.0 / 120.0 * (layout.column_width(1) - 2.0);
    let (x, y, w, h) = bars[0];
    // Полоса по центру строки — тем же полем, что и положительные в столбце A.
    let centered = layout.row_y(0) + (layout.row_height(0) - h) / 2.0;
    assert!(
        (y - centered).abs() < 0.01 && x > column && x < axis,
        "B1: отрицательная полоса слева от оси, по центру строки: x={x}, y={y}"
    );
    assert!(h > 0.0 && w > 0.0, "B1: полоса видна");
    let (x2, _, w2, _) = *bars.last().expect("полоса B12");
    // Максимум шкалы: полоса начинается на оси и доходит до правого поля.
    let right = column + layout.column_width(1) - 1.0;
    assert!(
        (x2 - axis).abs() < 0.01 && (x2 + w2 - right).abs() < 0.01 && w2 > w,
        "B12: положительная полоса от оси до края: x2={x2}, axis={axis}, w2={w2}, w={w}"
    );
}

#[test]
fn cf_icon_set_fixture_shows_icons_by_threshold() {
    let (book, dl) = painted_fixture("cf-icon-set.xlsx");
    let layout = SheetLayout::new(&book.sheets()[0]);

    // A1:A12 — светофор: значок один (круг), уровень несёт цвет.
    assert_eq!(
        icon_at(&dl, &layout, 0, 0),
        ("●".into(), PixelColor(0xF869_6BFF))
    );
    assert_eq!(
        icon_at(&dl, &layout, 11, 0),
        ("●".into(), PixelColor(0x63BE_7BFF))
    );

    // B1:B12 — четыре стрелки с `reverse`: верхнему значению достаётся значок
    // нижнего уровня, нижнему — верхнего.
    assert_eq!(
        icon_at(&dl, &layout, 0, 1),
        ("▼".into(), PixelColor(0xF869_6BFF))
    );
    assert_eq!(
        icon_at(&dl, &layout, 11, 1),
        ("▲".into(), PixelColor(0x63BE_7BFF))
    );

    // C1:C12 — `showValue="0"`: значения не рисуются, значки стоят по центру
    // ячейки.
    let texts = texts(&dl);
    assert!(
        !texts.iter().any(|(text, ..)| text == "1"),
        "значение скрыто значком"
    );
    let centered = texts
        .iter()
        .filter(|(text, tx, _, align, _)| {
            text == "●"
                && *align == TextAlign::Center
                && *tx > layout.column_x(2)
                && *tx < layout.column_x(3)
        })
        .count();
    assert_eq!(centered, 12, "значки столбца C стоят по центру");
}
