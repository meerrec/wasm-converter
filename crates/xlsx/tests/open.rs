//! Сквозной `open()` на минимальном, но валидном пакете.
//!
//! Фикстур из Excel на этом шаге ещё нет, поэтому пакет собирается в памяти —
//! ровно те части, которые нужны каталогу листов и их содержимому.

use std::io::{Cursor, Write};

use doc_converter_xlsx::cellref::CellRef;
use doc_converter_xlsx::{open, CellValue, Color, SheetState, XlsxError};

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
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="s"><v>0</v></c>
      <c r="B1"><v>42</v></c>
    </row>
    <row r="2"><c r="A2" s="1"><v>2.5</v></c></row>
  </sheetData>
</worksheet>"#;

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

    let first = wb.sheet("Данные").unwrap();
    assert_eq!(first.meta.part, "xl/worksheets/sheet1.xml");
    assert_eq!(first.meta.state, SheetState::Visible);
    assert_eq!(first.data.cell_count(), 3);
    assert_eq!(
        first.data.cell(CellRef::new(0, 1)).unwrap().value,
        CellValue::Number(42.0)
    );
    assert_eq!(first.data.cell(CellRef::new(1, 0)).unwrap().style, 1);

    // Индекс общей строки разрешается через таблицу книги.
    let shared = first.data.cell(CellRef::new(0, 0)).unwrap();
    assert_eq!(shared.value, CellValue::SharedString(0));
    assert_eq!(shared.value.text(wb.shared_strings()), Some("Привет"));

    let second = wb.sheet("Скрытый").unwrap();
    assert_eq!(second.meta.state, SheetState::VeryHidden);
    assert_eq!(
        second
            .data
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
    let format = styles.resolve(first.data.cell(CellRef::new(1, 0)).unwrap().style);
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
        .data
        .cell(CellRef::new(0, 0))
        .unwrap();
    assert_eq!(shared.value.text(wb.shared_strings()), None);
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
