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

use doc_converter_render::display_list::{Color as PixelColor, DisplayList, DrawCommand};
use doc_converter_xlsx::cellref::{CellRef, Range};
use doc_converter_xlsx::{
    open, paint_sheet, BorderStyle, CellIsOperator, CellValue, Color, ColorScale, ConditionalRule,
    DataBar, Fill, FillPattern, IconSet, PaintOptions, PaneKind, PaneState, RuleKind, Sheet,
    SheetState, Threshold, ThresholdKind, Viewport, Workbook, XlsxError,
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
