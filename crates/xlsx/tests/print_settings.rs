//! Настройки печати книги: часть листа и заголовки из `definedNames`.
//!
//! Печатаемые заголовки — единственная настройка, которая лежит не в листе:
//! `workbook.xml` хранит их ссылкой на имя листа, поэтому проверяется, что
//! [`open`] доносит их до нужного листа, а не до первого.

use std::io::{Cursor, Write};

use doc_converter_xlsx::{open, Orientation, PrintSettings, Span};

/// Два листа («Первый» и «Второй») с заданным содержимым и `definedNames`.
fn package(first: &str, second: &str, defined_names: &str) -> Vec<u8> {
    let sheet = |body: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#
        )
    };
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Первый" sheetId="1" r:id="rId1"/>
    <sheet name="Второй" sheetId="2" r:id="rId2"/>
  </sheets>
  {defined_names}
</workbook>"#
    );
    let workbook_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="xl/workbook.xml"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"/>
</Relationships>"#;
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml"
            ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"#;

    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("[Content_Types].xml", content_types.to_owned()),
            ("_rels/.rels", root_rels.to_owned()),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels.to_owned()),
            ("xl/worksheets/sheet1.xml", sheet(first)),
            ("xl/worksheets/sheet2.xml", sheet(second)),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn sheet_print_settings_come_from_the_part() {
    let book = open(package(
        r#"<sheetPr><pageSetUpPr fitToPage="1"/></sheetPr>
           <pageSetup orientation="landscape" fitToWidth="2" fitToHeight="1"/>"#,
        r#"<printOptions gridLines="1" headings="1"/>"#,
        "",
    ))
    .unwrap();

    let first = &book.sheets()[0].print;
    assert!(first.fit_to_page);
    assert_eq!(first.orientation, Orientation::Landscape);
    assert_eq!(first.fit_to_width, 2);
    assert_eq!(first.fit_to_height, 1);

    let second = &book.sheets()[1].print;
    assert!(!second.fit_to_page);
    assert!(second.grid_lines);
    assert!(second.headings);
}

#[test]
fn print_titles_land_on_the_named_sheet() {
    let book = open(package(
        "<sheetData/>",
        "<sheetData/>",
        r#"<definedNames>
             <definedName name="_xlnm.Print_Titles" localSheetId="1">'Второй'!$1:$2,'Второй'!$A:$B</definedName>
           </definedNames>"#,
    ))
    .unwrap();

    assert_eq!(book.sheets()[0].print.repeat_header_rows, None);
    assert_eq!(book.sheets()[0].print.repeat_first_columns, None);

    let second = &book.sheets()[1].print;
    assert_eq!(second.repeat_header_rows, Some(Span { first: 0, last: 1 }));
    assert_eq!(
        second.repeat_first_columns,
        Some(Span { first: 0, last: 1 })
    );
}

#[test]
fn book_without_defined_names_keeps_default_print_settings() {
    let book = open(package("<sheetData/>", "<sheetData/>", "")).unwrap();

    for sheet in book.sheets() {
        assert_eq!(sheet.print, PrintSettings::default());
        assert_eq!(sheet.print.repeat_header_rows, None);
        assert_eq!(sheet.print.repeat_first_columns, None);
    }
}

#[test]
fn print_titles_for_an_unknown_sheet_are_ignored() {
    let book = open(package(
        "<sheetData/>",
        "<sheetData/>",
        r#"<definedNames>
             <definedName name="_xlnm.Print_Titles">'Удалённый'!$1:$1</definedName>
           </definedNames>"#,
    ))
    .unwrap();

    for sheet in book.sheets() {
        assert_eq!(sheet.print, PrintSettings::default());
    }
}
