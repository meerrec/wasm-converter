//! Сквозной `open()` на минимальном, но валидном пакете.
//!
//! Фикстур из Excel на этом шаге ещё нет, поэтому пакет собирается в памяти —
//! ровно те части, которые нужны каталогу листов и их содержимому.

use std::io::{Cursor, Write};

use doc_converter_xlsx::cellref::CellRef;
use doc_converter_xlsx::{
    open, CellValue, Color, HyperlinkTarget, PaneKind, PaneState, SheetState, XlsxError,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml"
            ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="xl/workbook.xml"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <workbookPr date1904="1"/>
  <sheets>
    <sheet name="Данные" sheetId="1" r:id="rId1"/>
    <sheet name="Скрытый" sheetId="2" state="veryHidden" r:id="rId2"/>
    <sheet name="Диаграмма" sheetId="3" r:id="rId3"/>
  </sheets>
</workbook>"#;

const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                Target="/xl/worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet"
                Target="chartsheets/sheet1.xml"/>
</Relationships>"#;

const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="1F497D"/></a:dk2>
      <a:lt2><a:srgbClr val="EEECE1"/></a:lt2>
      <a:accent1><a:srgbClr val="4F81BD"/></a:accent1>
      <a:accent2><a:srgbClr val="C0504D"/></a:accent2>
      <a:accent3><a:srgbClr val="9BBB59"/></a:accent3>
      <a:accent4><a:srgbClr val="8064A2"/></a:accent4>
      <a:accent5><a:srgbClr val="4BACC6"/></a:accent5>
      <a:accent6><a:srgbClr val="F79646"/></a:accent6>
      <a:hlink><a:srgbClr val="0000FF"/></a:hlink>
      <a:folHlink><a:srgbClr val="800080"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office">
      <a:majorFont><a:latin typeface="Cambria"/></a:majorFont>
      <a:minorFont><a:latin typeface="Calibri"/></a:minorFont>
    </a:fontScheme>
  </a:themeElements>
</a:theme>"#;

const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2">
  <si><t>Привет</t></si>
  <si><t xml:space="preserve"> мир </t></si>
</sst>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="0.00%"/></numFmts>
  <fonts count="2">
    <font><sz val="11"/><name val="Calibri"/></font>
    <font><b/><sz val="11"/><name val="Arial"/></font>
  </fonts>
  <fills count="2">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill>
  </fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="2">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="164" fontId="1" fillId="1" borderId="0" xfId="0"
        applyNumberFormat="1" applyFont="1" applyFill="1"/>
  </cellXfs>
</styleSheet>"#;

const SHEET1: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <dimension ref="A1:C9"/>
  <sheetViews><sheetView tabSelected="1" showGridLines="0" zoomScale="85" workbookViewId="0">
    <pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/>
  </sheetView></sheetViews>
  <sheetFormatPr defaultRowHeight="15" defaultColWidth="9"/>
  <cols><col min="2" max="2" width="20" customWidth="1"/></cols>
  <sheetData>
    <row r="1">
      <c r="A1" t="s"><v>0</v></c>
      <c r="B1"><v>42</v></c>
    </row>
    <row r="2" ht="30" customHeight="1"><c r="A2" s="1"><v>2.5</v></c></row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells>
  <hyperlinks>
    <hyperlink ref="A1" r:id="rId1"/>
    <hyperlink ref="A2" location="Скрытый!A1" display="скрытый лист"/>
  </hyperlinks>
</worksheet>"#;

const SHEET1_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
                Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/>
</Relationships>"#;

const SHEET2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>скрытый лист</t></is></c></row>
  </sheetData>
</worksheet>"#;

/// Собрать OOXML-пакет из частей.
fn package(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        for (name, body) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

fn package_with_workbook() -> Vec<u8> {
    package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", SHEET1),
        ("xl/worksheets/sheet2.xml", SHEET2),
        ("xl/worksheets/_rels/sheet1.xml.rels", SHEET1_RELS),
    ])
}

#[test]
fn opens_sheets_strings_and_date_system() {
    let entries = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", SHEET1),
        ("xl/worksheets/sheet2.xml", SHEET2),
        ("xl/sharedStrings.xml", SHARED_STRINGS),
        ("xl/styles.xml", STYLES),
    ];

    let wb = open(package(&entries)).unwrap();

    // Диаграмма в каталоге листов не занимает места.
    assert_eq!(wb.sheet_count(), 2);
    assert!(wb.sheet("Диаграмма").is_none());
    assert!(wb.date1904());
    // В этом пакете темы нет — её цвета остаются неразрешёнными.
    assert!(wb.theme().color(1).is_none());

    let first = wb.sheet("Данные").unwrap();
    assert_eq!(first.meta.part, "xl/worksheets/sheet1.xml");
    assert_eq!(first.meta.state, SheetState::Visible);
    assert_eq!(first.cells.cell_count(), 3);
    assert_eq!(
        first.cells.cell(CellRef::new(0, 1)).unwrap().value,
        CellValue::Number(42.0)
    );
    assert_eq!(first.cells.cell(CellRef::new(1, 0)).unwrap().style, 1);

    // Индекс общей строки разрешается через таблицу книги.
    let shared = first.cells.cell(CellRef::new(0, 0)).unwrap();
    assert_eq!(shared.value, CellValue::SharedString(0));
    assert_eq!(shared.value.text(wb.shared_strings()), Some("Привет"));

    let second = wb.sheet("Скрытый").unwrap();
    assert_eq!(second.meta.state, SheetState::VeryHidden);
    assert_eq!(
        second
            .cells
            .cell(CellRef::new(0, 0))
            .unwrap()
            .value
            .text(wb.shared_strings()),
        Some("скрытый лист")
    );

    assert_eq!(wb.shared_strings().len(), 2);
    // Пробелы по краям — часть строки, а не форматирование.
    assert_eq!(wb.shared_strings().get(1), Some(" мир "));

    // Индекс формата из ячейки доводит до шрифта, заливки и кода числа.
    let styles = wb.styles();
    assert_eq!(styles.len(), 2);
    let format = styles.resolve(first.cells.cell(CellRef::new(1, 0)).unwrap().style);
    let font = styles.font(format.font).unwrap();
    assert_eq!(font.name, "Arial");
    assert!(font.bold);
    assert_eq!(
        styles.fill(format.fill).unwrap().foreground,
        Color::Rgb(0xFFFF_FF00)
    );
    assert_eq!(styles.number_format(format.num_fmt), Some("0.00%"));
}

#[test]
fn workbook_without_shared_strings_opens() {
    let wb = open(package_with_workbook()).unwrap();

    assert_eq!(wb.sheet_count(), 2);
    assert!(wb.shared_strings().is_empty());
    // Части стилей в пакете нет — это не ошибка.
    assert!(wb.styles().is_empty());
    // Ссылка на строку без таблицы текста не даёт, но лист открывается.
    let shared = wb
        .sheet("Данные")
        .unwrap()
        .cells
        .cell(CellRef::new(0, 0))
        .unwrap();
    assert_eq!(shared.value.text(wb.shared_strings()), None);
}

#[test]
fn theme_is_found_through_workbook_rels() {
    // Связь на тему дописывается в конец карты связей — так же, как её
    // размещает Excel.
    let rels = WORKBOOK_RELS.replace(
        "</Relationships>",
        concat!(
            r#"<Relationship Id="rId4" "#,
            r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" "#,
            r#"Target="theme/theme1.xml"/></Relationships>"#,
        ),
    );
    let bytes = package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", SHEET1),
        ("xl/worksheets/sheet2.xml", SHEET2),
        ("xl/theme/theme1.xml", THEME),
    ]);

    let wb = open(bytes).unwrap();
    let theme = wb.theme();

    // Часть нашлась по связи книги (`Target="theme/theme1.xml"`), а индексы
    // пришли в порядке SpreadsheetML: 0 — `lt1`, 1 — `dk1`.
    assert_eq!(theme.color(0), Some(Color::Rgb(0xFFFF_FFFF)));
    assert_eq!(theme.color(1), Some(Color::Rgb(0xFF00_0000)));
    assert_eq!(theme.color(4), Some(Color::Rgb(0xFF4F_81BD)));
    assert_eq!(theme.color(11), Some(Color::Rgb(0xFF80_0080)));
    assert_eq!(theme.major_font(), Some("Cambria"));
    assert_eq!(theme.minor_font(), Some("Calibri"));
}

#[test]
fn missing_sheet_part_is_an_error() {
    let bytes = package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
    ]);

    let err = open(bytes).unwrap_err();

    assert!(matches!(err, XlsxError::Core(_)));
    assert!(err.to_string().contains("xl/worksheets/sheet1.xml"));
}

#[test]
fn missing_workbook_part_is_an_error() {
    let bytes = package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
    ]);

    let err = open(bytes).unwrap_err();

    assert!(matches!(err, XlsxError::Core(_)));
    assert!(err.to_string().contains("xl/_rels/workbook.xml.rels"));
}

#[test]
fn not_a_zip_is_an_error() {
    let err = open(b"not a zip at all".to_vec()).unwrap_err();

    assert!(matches!(err, XlsxError::Core(_)));
}

#[test]
fn reads_view_geometry_merges_and_hyperlinks() {
    let wb = open(package_with_workbook()).unwrap();
    let sheet = wb.sheet("Данные").unwrap();

    // Вид листа.
    assert!(!sheet.view.show_grid_lines);
    assert!(sheet.view.show_headers);
    assert_eq!(sheet.view.zoom, 85);
    let pane = sheet.view.pane.unwrap();
    assert_eq!(pane.rows, 1);
    assert_eq!(pane.cols, 0);
    assert_eq!(pane.state, PaneState::Frozen);
    assert_eq!(pane.active, PaneKind::BottomLeft);
    assert_eq!(pane.top_left, Some(CellRef::new(1, 0)));

    // Объявленная геометрия: `<dimension>` врёт, фактические границы — нет.
    assert_eq!(sheet.dims.col_width(1), 20.0);
    assert_eq!(sheet.dims.col_width(0), 9.0);
    assert_eq!(sheet.dims.row_height(1), 30.0);
    assert_eq!(sheet.cells.used_range().unwrap().last, CellRef::new(1, 1));
    assert_eq!(
        sheet.dims.declared.unwrap().last,
        CellRef::new(8, 2),
        "объявленный диапазон сохраняется как есть"
    );

    // Объединение и гиперссылки.
    let merged = sheet.merged_range(CellRef::new(0, 1)).unwrap();
    assert_eq!(merged.first, CellRef::new(0, 0));
    assert_eq!(sheet.merged_range(CellRef::new(1, 1)), None);

    let external = sheet.hyperlink_at(CellRef::new(0, 0)).unwrap();
    assert_eq!(
        external.target,
        HyperlinkTarget::External("https://example.com/?a=1&b=2".into())
    );

    let internal = sheet.hyperlink_at(CellRef::new(1, 0)).unwrap();
    assert_eq!(
        internal.target,
        HyperlinkTarget::Internal("Скрытый!A1".into())
    );
    assert_eq!(internal.display.as_deref(), Some("скрытый лист"));
    assert_eq!(sheet.hyperlink_at(CellRef::new(1, 1)), None);

    // У второго листа связей нет вовсе — это не ошибка.
    assert!(wb.sheet("Скрытый").unwrap().hyperlinks.is_empty());
}

/// Связи листа с примечаниями: гиперссылка из той же части остаётся рабочей.
const SHEET1_RELS_WITH_COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
                Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments"
                Target="../comments1.xml"/>
</Relationships>"#;

const COMMENTS1: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <authors><author>Ирек</author></authors>
  <commentList>
    <comment ref="B1" authorId="0">
      <text><r><t>Первая</t></r><r><t xml:space="preserve"> строка</t></r></text>
    </comment>
  </commentList>
</comments>"#;

/// Комментарии приходят из части, найденной по связи листа, а не по имени.
#[test]
fn reads_comments_through_sheet_rels() {
    let entries = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", SHEET1),
        ("xl/worksheets/sheet2.xml", SHEET2),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            SHEET1_RELS_WITH_COMMENTS,
        ),
        ("xl/comments1.xml", COMMENTS1),
    ];

    let wb = open(package(&entries)).unwrap();
    let sheet = wb.sheet("Данные").unwrap();

    let comment = sheet.comment_at(CellRef::new(0, 1)).unwrap();
    assert_eq!(comment.author.as_deref(), Some("Ирек"));
    assert_eq!(comment.text, "Первая строка");
    assert_eq!(sheet.comments.len(), 1);
    assert_eq!(sheet.comment_at(CellRef::new(0, 0)), None);
    // Гиперссылка из той же части связей продолжает работать.
    assert!(sheet.hyperlink_at(CellRef::new(0, 0)).is_some());

    // У второго листа связи на примечания нет — это не ошибка.
    assert!(wb.sheet("Скрытый").unwrap().comments.is_empty());
}

/// Лист без части примечаний открывается, примечаний у него нет.
#[test]
fn sheet_without_comments_opens() {
    let wb = open(package_with_workbook()).unwrap();

    for sheet in wb.sheets() {
        assert!(sheet.comments.is_empty());
        assert!(sheet.comment_at(CellRef::new(0, 0)).is_none());
    }
}
