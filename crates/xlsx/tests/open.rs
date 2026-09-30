//! Сквозной `open()` на минимальном, но валидном пакете.
//!
//! Фикстур из Excel на этом шаге ещё нет, поэтому пакет собирается в памяти —
//! ровно те части, которые нужны каталогу листов.

use std::io::{Cursor, Write};

use doc_converter_xlsx::{open, SheetState, XlsxError};

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
    ])
}

#[test]
fn opens_sheets_strings_and_date_system() {
    let mut entries = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
    ];
    entries.push(("xl/sharedStrings.xml", SHARED_STRINGS));

    let wb = open(package(&entries)).unwrap();

    // Диаграмма в каталоге листов не занимает места.
    assert_eq!(wb.sheet_count(), 2);
    assert_eq!(wb.sheets()[0].name, "Данные");
    assert_eq!(wb.sheets()[0].part, "xl/worksheets/sheet1.xml");
    assert_eq!(wb.sheets()[0].state, SheetState::Visible);
    assert_eq!(wb.sheets()[1].part, "xl/worksheets/sheet2.xml");
    assert_eq!(wb.sheets()[1].state, SheetState::VeryHidden);
    assert_eq!(wb.sheet("Диаграмма"), None);

    assert!(wb.date1904());
    assert_eq!(wb.shared_strings().len(), 2);
    assert_eq!(wb.shared_strings().get(0), Some("Привет"));
    // Пробелы по краям — часть строки, а не форматирование.
    assert_eq!(wb.shared_strings().get(1), Some(" мир "));

    // Стили и содержимое листов — следующие шаги Фазы 3.
    assert!(wb.styles().is_empty());
}

#[test]
fn workbook_without_shared_strings_opens() {
    let wb = open(package_with_workbook()).unwrap();

    assert_eq!(wb.sheet_count(), 2);
    assert!(wb.shared_strings().is_empty());
    assert_eq!(wb.shared_strings().get(0), None);
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
