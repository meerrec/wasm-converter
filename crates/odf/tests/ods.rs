//! Сквозные проверки ODS на фикстурах `test-fixtures/odf/ods/`.
//!
//! Фикстуры собирает `scripts/generate_odf_fixtures.ts`, поэтому значения в них
//! известны заранее: тест сверяет не «что-то разобралось», а конкретные ячейки,
//! объединения и геометрию.
#![cfg(feature = "ods")]

use std::io::{Cursor, Write};
use std::path::Path;

use doc_converter_core::Error as CoreError;
use doc_converter_odf::error::OdfError;
use doc_converter_odf::manifest::{ODS_MIME, ODT_MIME};
use doc_converter_odf::open;
use doc_converter_xlsx::{Cell, CellRef, CellValue, Range, SheetState, Workbook};
use zip::write::SimpleFileOptions;

/// Фикстура из общего корпуса ODF.
fn fixture(name: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/odf/ods");
    std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("фикстура {name}: {error}"))
}

/// Ячейка книги: лист, строка и столбец — 0-based.
fn cell(book: &Workbook, sheet: usize, row: u32, col: u32) -> Option<&Cell> {
    book.sheets().get(sheet)?.cells.cell(CellRef::new(row, col))
}

/// Текст ячейки, если он у неё есть.
fn text(book: &Workbook, sheet: usize, row: u32, col: u32) -> Option<&str> {
    cell(book, sheet, row, col)?
        .value
        .text(book.shared_strings())
}

/// Значение ячейки; отсутствующая ячейка — паника, тест ждёт её явно.
fn value(book: &Workbook, sheet: usize, row: u32, col: u32) -> &CellValue {
    &cell(book, sheet, row, col)
        .unwrap_or_else(|| panic!("нет ячейки ({row}, {col})"))
        .value
}

#[test]
fn simple_fixture_has_two_sheets_and_values() {
    let book = open(fixture("simple.ods")).expect("simple.ods разбирается");

    assert_eq!(book.sheets().len(), 2);
    assert_eq!(book.sheets()[0].meta.name, "Data");
    assert_eq!(book.sheets()[1].meta.name, "Summary");
    assert_eq!(book.sheets()[0].meta.part, "content.xml");
    assert_eq!(book.sheets()[0].meta.state, SheetState::Visible);

    // Строка, число, логическое значение и дата.
    assert_eq!(text(&book, 0, 0, 0), Some("Item"));
    assert_eq!(value(&book, 0, 1, 1), &CellValue::Number(12.5));
    assert_eq!(value(&book, 0, 1, 2), &CellValue::Bool(true));
    assert_eq!(value(&book, 0, 2, 2), &CellValue::Bool(false));
    // 15.01.2024 в системе 1900 — серийный номер 45306.
    assert_eq!(value(&book, 0, 1, 3), &CellValue::Number(45_306.0));
    assert_eq!(value(&book, 0, 2, 3), &CellValue::Number(45_342.0));

    // Процент хранится долей, как и в XLSX.
    assert_eq!(value(&book, 0, 3, 2), &CellValue::Number(0.25));

    // Второй лист.
    assert_eq!(text(&book, 1, 1, 0), Some("Total"));
    assert_eq!(value(&book, 1, 1, 1), &CellValue::Number(20.0));
}

#[test]
fn date_style_is_mapped_to_a_number_format() {
    let book = open(fixture("simple.ods")).expect("simple.ods разбирается");
    let cell = cell(&book, 0, 1, 3).expect("ячейка даты");
    let format = book.styles().resolve(cell.style);
    let code = book
        .styles()
        .format_code(format.num_fmt)
        .expect("у даты есть числовой формат");

    // Стиль `ce_date` ссылается на `N_date` из styles.xml: DD.MM.YYYY.
    assert!(code.contains("DD"), "код формата {code}");
    assert!(code.contains("YYYY"), "код формата {code}");
}

#[test]
fn dates_and_percents_render_through_their_formats() {
    let book = open(fixture("simple.ods")).expect("simple.ods разбирается");

    // Дата — серийный номер плюс формат из `styles.xml`: видно 15.01.2024, а не
    // 45306. Это и есть проверка, что значение не потеряно вместе с видом.
    let date = cell(&book, 0, 1, 3).expect("ячейка даты");
    let date_format = book.styles().resolve(date.style).num_fmt;
    let shown =
        doc_converter_xlsx::paint::display_text(&book, 0, CellRef::new(1, 3), date, date_format);
    assert_eq!(shown.as_deref(), Some("15.01.2024"));

    // Процент: 0,25 при формате `0%` — это 25 %.
    let percent = cell(&book, 0, 3, 2).expect("ячейка процента");
    let percent_format = book.styles().resolve(percent.style).num_fmt;
    let shown = doc_converter_xlsx::paint::display_text(
        &book,
        0,
        CellRef::new(3, 2),
        percent,
        percent_format,
    );
    assert_eq!(shown.as_deref(), Some("25%"));
}

#[test]
fn merge_geometry_and_formulas_are_kept() {
    let book = open(fixture("simple.ods")).expect("simple.ods разбирается");
    let sheet = &book.sheets()[0];

    // A4:B4 — объединение в четвёртой строке (0-based 3).
    let merge = sheet
        .merges
        .covering(CellRef::new(3, 1))
        .expect("B4 покрыта объединением");
    assert_eq!(
        merge,
        Range {
            first: CellRef::new(3, 0),
            last: CellRef::new(3, 1),
        }
    );
    assert_eq!(text(&book, 0, 3, 0), Some("Merged A4:B4"));

    // Ширины: первый столбец 4 см, второй 3 см; высота строки 0,8 см.
    // 4 см при 96 dpi и ширине цифры 7,4 px — около 20,4 «символов» Excel.
    let width = sheet.dims.col_width(0);
    assert!((width - 20.43).abs() < 0.2, "ширина столбца {width}");
    let height = sheet.dims.row_height(0);
    // 0,8 см = 22,68 pt.
    assert!((height - 22.68).abs() < 0.1, "высота строки {height}");

    // Формулы не роняют разбор и сохраняются текстом без `of:=`.
    let formula = cell(&book, 0, 1, 4)
        .expect("ячейка с формулой")
        .formula
        .as_deref();
    assert_eq!(formula, Some("SUM([.B2:.B3])"));
    assert_eq!(value(&book, 0, 1, 4), &CellValue::Number(20.0));
}

#[test]
fn repeated_rows_and_columns_are_expanded() {
    let book = open(fixture("repeated.ods")).expect("repeated.ods разбирается");
    assert_eq!(book.sheets().len(), 1);
    let sheet = &book.sheets()[0];
    assert_eq!(sheet.meta.name, "Repeated");

    // Заголовок (4) + четыре повторённые строки (2 ячейки) + три строки
    // объединения (3 + 2 + 1); покрытые ячейки значений не создают.
    assert_eq!(sheet.cells.cell_count(), 18);

    // `table:number-rows-repeated="4"`: строки 1–4 повторяют содержимое.
    for row in 1..=4 {
        assert_eq!(text(&book, 0, row, 0), Some("same"), "строка {row}");
        assert_eq!(value(&book, 0, row, 1), &CellValue::Number(1.0));
        assert!(
            cell(&book, 0, row, 2).is_none(),
            "пустой повтор не создаёт ячеек"
        );
    }

    // Объединение 2×2 начинается в A6 (0-based row 5).
    let merge = sheet
        .merges
        .covering(CellRef::new(5, 0))
        .expect("объединение 2×2");
    assert_eq!(
        merge,
        Range {
            first: CellRef::new(5, 0),
            last: CellRef::new(6, 1),
        }
    );
    assert_eq!(sheet.merges.len(), 1);

    // Покрытые ячейки съедены объединением и не стали отдельными значениями.
    assert!(cell(&book, 0, 5, 1).is_none());
    assert!(cell(&book, 0, 6, 0).is_none());
    assert!(cell(&book, 0, 6, 1).is_none());
    assert_eq!(text(&book, 0, 7, 3), Some("D8"));

    // Ширины: три столбца по 3 см и четвёртый 6 см.
    assert!((sheet.dims.col_width(0) - 15.32).abs() < 0.2);
    assert!((sheet.dims.col_width(2) - 15.32).abs() < 0.2);
    assert!((sheet.dims.col_width(3) - 30.65).abs() < 0.2);
}

/// Минимальный валидный каталог ODF с указанным media-type корня.
fn manifest(media_type: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
  <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="{media_type}"/>
  <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#
    )
}

/// `content.xml` с одной таблицей и одной числовой ячейкой.
const CONTENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
  <office:body><office:spreadsheet>
    <table:table table:name="Only">
      <table:table-row>
        <table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell>
      </table:table-row>
    </table:table>
  </office:spreadsheet></office:body>
</office:document-content>"#;

/// Собрать ZIP: `mimetype` первой записью и без сжатия — как требует ODF.
fn package(parts: &[(&str, &[u8])], mimetype: Option<&str>) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        if let Some(mime) = mimetype {
            writer
                .start_file(
                    "mimetype",
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(mime.as_bytes()).unwrap();
        }
        for (name, data) in parts {
            writer
                .start_file(
                    *name,
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    buf
}

#[test]
fn minimal_package_opens() {
    let manifest = manifest(ODS_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        Some(ODS_MIME),
    );

    let book = open(bytes).expect("собранный пакет разбирается");
    assert_eq!(book.sheets().len(), 1);
    assert_eq!(book.sheets()[0].meta.name, "Only");
    assert_eq!(value(&book, 0, 0, 0), &CellValue::Number(1.0));
}

#[test]
fn garbage_is_a_core_error() {
    let error = open(b"not a zip".to_vec()).expect_err("не ZIP");
    assert!(matches!(error, OdfError::Core(_)), "{error:?}");
}

#[test]
fn missing_mimetype_is_malformed() {
    let manifest = manifest(ODS_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        None,
    );

    let error = open(bytes).expect_err("нет mimetype");
    assert!(
        matches!(&error, OdfError::Malformed { part, .. } if part == "mimetype"),
        "{error:?}"
    );
}

#[test]
fn wrong_mimetype_is_unsupported() {
    let manifest = manifest(ODT_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        Some(ODT_MIME),
    );

    let error = open(bytes).expect_err("ODT в этом срезе не поддержан");
    assert!(
        matches!(&error, OdfError::Unsupported(message) if message.contains("odt")),
        "{error:?}"
    );
}

#[test]
fn missing_manifest_is_malformed() {
    let bytes = package(&[("content.xml", CONTENT.as_bytes())], Some(ODS_MIME));

    let error = open(bytes).expect_err("нет каталога");
    assert!(
        matches!(&error, OdfError::Malformed { part, .. } if part == "META-INF/manifest.xml"),
        "{error:?}"
    );
}

#[test]
fn missing_content_is_a_missing_part() {
    let manifest = manifest(ODS_MIME);
    let bytes = package(
        &[("META-INF/manifest.xml", manifest.as_bytes())],
        Some(ODS_MIME),
    );

    let error = open(bytes).expect_err("нет content.xml");
    assert!(
        matches!(
            &error,
            OdfError::Core(CoreError::MissingPart(part)) if part == "content.xml"
        ),
        "{error:?}"
    );
}

#[test]
fn conflicting_root_media_type_is_malformed() {
    // mimetype говорит ODS, каталог — ODT: пакет противоречит сам себе.
    let manifest = manifest(ODT_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        Some(ODS_MIME),
    );

    let error = open(bytes).expect_err("противоречие");
    assert!(matches!(error, OdfError::Malformed { .. }), "{error:?}");
}

/// `content.xml` с одной таблицей и телом строк, вставленным как есть.
fn content_with(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
  <office:body><office:spreadsheet>
    <table:table table:name="S">{body}</table:table>
  </office:spreadsheet></office:body>
</office:document-content>"#
    )
}

/// Пакет ODS из готового `content.xml`.
fn ods_package(content: &str) -> Vec<u8> {
    let manifest = manifest(ODS_MIME);
    package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", content.as_bytes()),
        ],
        Some(ODS_MIME),
    )
}

#[test]
fn giant_repeated_row_with_content_is_a_limit_error() {
    let content = content_with(
        r#"<table:table-row table:number-rows-repeated="2000000">
             <table:table-cell office:value-type="float" office:value="1"/>
           </table:table-row>"#,
    );

    let error = open(ods_package(&content)).expect_err("повтор больше лимита Excel");
    assert!(
        matches!(&error, OdfError::Limit(message) if message.contains("1048576")),
        "{error:?}"
    );
}

#[test]
fn giant_repeated_empty_row_is_skipped() {
    // Хвост листа LibreOffice: миллион пустых строк не должен ни разворачиваться,
    // ни считаться ошибкой.
    let content = content_with(
        r#"<table:table-row>
             <table:table-cell office:value-type="float" office:value="1"/>
           </table:table-row>
           <table:table-row table:number-rows-repeated="1048576"/>"#,
    );

    let book = open(ods_package(&content)).expect("пустой повтор пропускается");
    assert_eq!(book.sheets()[0].cells.cell_count(), 1);
    assert_eq!(value(&book, 0, 0, 0), &CellValue::Number(1.0));
}

#[test]
fn giant_repeated_column_with_content_is_a_limit_error() {
    let content = content_with(
        r#"<table:table-row>
             <table:table-cell table:number-columns-repeated="20000"
                 office:value-type="float" office:value="1"/>
           </table:table-row>"#,
    );

    let error = open(ods_package(&content)).expect_err("повтор больше лимита Excel");
    assert!(
        matches!(&error, OdfError::Limit(message) if message.contains("16384")),
        "{error:?}"
    );
}

#[test]
fn mimetype_must_be_the_first_part() {
    // mimetype записан вторым: спецификация ODF требует его первым, иначе
    // определить формат по первым байтам нельзя.
    let manifest = manifest(ODS_MIME);
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options =
            || SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("content.xml", options()).unwrap();
        writer.write_all(CONTENT.as_bytes()).unwrap();
        writer
            .start_file("META-INF/manifest.xml", options())
            .unwrap();
        writer.write_all(manifest.as_bytes()).unwrap();
        writer.start_file("mimetype", options()).unwrap();
        writer.write_all(ODS_MIME.as_bytes()).unwrap();
        writer.finish().unwrap();
    }

    let error = open(buf).expect_err("mimetype не первый");
    assert!(
        matches!(&error, OdfError::Malformed { part, .. } if part == "mimetype"),
        "{error:?}"
    );
}
