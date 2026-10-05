//! Изображения листа: якоря и ссылки на media на фикстурах.
//!
//! Фикстуры собраны `scripts/gen-fixtures.ts`: в PNG-книге оба вида якорей,
//! в JPEG-книге их границы попадают в середину ячеек, в книге «поверх данных»
//! картинки лежат на данных и у правого края листа.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use doc_converter_xlsx::{open, EditAs, ImageAnchor, Workbook};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/xlsx")
}

fn open_fixture(name: &str) -> Workbook {
    let path = fixtures_dir().join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    open(bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Сравнить пиксели с допуском: EMU делится на 9525 с дробным остатком.
fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 0.01
}

/// PNG-книга: one-cell якорь с явным `ext` и two-cell диапазон E3:H8.
#[test]
fn png_book_keeps_both_anchor_kinds() {
    let book = open_fixture("images-png.xlsx");
    let sheet = &book.sheets()[0];
    assert_eq!(sheet.meta.name, "PNG");
    assert_eq!(sheet.images.len(), 2);

    let one_cell = &sheet.images[0];
    assert_eq!(one_cell.name.as_deref(), Some("Picture 1"));
    assert_eq!(one_cell.media.as_deref(), Some("xl/media/image1.png"));
    assert_eq!(one_cell.edit_as, Some(EditAs::OneCell));
    match one_cell.anchor {
        ImageAnchor::OneCell { from, ext } => {
            assert_eq!((from.col, from.row), (1, 1));
            assert!(
                close(from.col_off, 0.0) && close(from.row_off, 0.0),
                "{from:?}"
            );
            // 914400×685800 EMU — это 96×72 пикселя.
            assert!(close(ext.cx, 96.0), "{ext:?}");
            assert!(close(ext.cy, 72.0), "{ext:?}");
        }
        other @ ImageAnchor::TwoCell { .. } => panic!("ожидался one-cell якорь: {other:?}"),
    }

    let two_cell = &sheet.images[1];
    assert_eq!(two_cell.name.as_deref(), Some("Picture 2"));
    assert_eq!(two_cell.media.as_deref(), Some("xl/media/image1.png"));
    match two_cell.anchor {
        ImageAnchor::TwoCell { from, to } => {
            assert_eq!((from.col, from.row), (4, 2));
            assert!(
                close(from.col_off, 0.0) && close(from.row_off, 0.0),
                "{from:?}"
            );
            assert_eq!((to.col, to.row), (8, 8));
        }
        other @ ImageAnchor::OneCell { .. } => panic!("ожидался two-cell якорь: {other:?}"),
    }
}

/// JPEG-книга: границы якорей — середина ячеек, то есть смещения не нулевые.
#[test]
fn jpeg_book_keeps_offsets_inside_cells() {
    let book = open_fixture("images-jpeg.xlsx");
    let sheet = &book.sheets()[0];
    assert_eq!(sheet.images.len(), 2);

    let one_cell = &sheet.images[0];
    assert_eq!(one_cell.media.as_deref(), Some("xl/media/image1.jpeg"));
    match one_cell.anchor {
        ImageAnchor::OneCell { from, ext } => {
            assert_eq!((from.col, from.row), (2, 3));
            // 320000 и 90000 EMU — 33,6 и 9,45 пикселя.
            assert!(close(from.col_off, 33.5958), "{from:?}");
            assert!(close(from.row_off, 9.4488), "{from:?}");
            assert!(close(ext.cx, 120.0) && close(ext.cy, 80.0), "{ext:?}");
        }
        other @ ImageAnchor::TwoCell { .. } => panic!("ожидался one-cell якорь: {other:?}"),
    }

    let two_cell = &sheet.images[1];
    match two_cell.anchor {
        ImageAnchor::TwoCell { from, to } => {
            assert_eq!((from.col, from.row), (0, 0));
            assert_eq!((to.col, to.row), (3, 4));
            assert!(close(to.col_off, 33.5958), "{to:?}");
            assert!(close(to.row_off, 0.0), "{to:?}");
        }
        other @ ImageAnchor::OneCell { .. } => panic!("ожидался two-cell якорь: {other:?}"),
    }
}

/// Книга «поверх данных»: порядок наложения сохраняется, а якорь у правого
/// края листа не обрезается разбором.
#[test]
fn over_data_book_keeps_order_and_edge_anchor() {
    let book = open_fixture("images-over-data.xlsx");
    let sheet = &book.sheets()[0];
    assert_eq!(sheet.images.len(), 3);

    let media: Vec<_> = sheet
        .images
        .iter()
        .map(|image| image.media.as_deref())
        .collect();
    assert_eq!(
        media,
        [
            Some("xl/media/image1.png"),
            Some("xl/media/image2.jpeg"),
            Some("xl/media/image1.png"),
        ]
    );

    // Две картинки делят одну media-часть: ссылка на неё не уникальна.
    assert_eq!(sheet.images[2].name.as_deref(), Some("Picture 3"));
    match sheet.images[2].anchor {
        ImageAnchor::OneCell { from, ext } => {
            assert_eq!((from.col, from.row), (16_381, 4));
            assert!(close(ext.cx, 80.0) && close(ext.cy, 60.0), "{ext:?}");
        }
        other @ ImageAnchor::TwoCell { .. } => panic!("ожидался one-cell якорь: {other:?}"),
    }
}

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
  <sheets><sheet name="Лист1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                Target="worksheets/sheet1.xml"/>
</Relationships>"#;

const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
  <drawing r:id="rId1"/>
</worksheet>"#;

const SHEET_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing"
                Target="../drawings/drawing1.xml"/>
</Relationships>"#;

/// Три картинки: целая ссылка, ссылка без связи и ссылка на отсутствующую часть.
const DRAWING: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
          xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff>
              <xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:ext cx="9525" cy="9525"/>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="1" name="Picture 1"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill></xdr:pic>
  </xdr:oneCellAnchor>
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff>
              <xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:ext cx="9525" cy="9525"/>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Picture 2"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip r:embed="rId2"/></xdr:blipFill></xdr:pic>
  </xdr:oneCellAnchor>
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>2</xdr:col><xdr:colOff>0</xdr:colOff>
              <xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:ext cx="9525" cy="9525"/>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="3" name="Picture 3"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip r:embed="rId3"/></xdr:blipFill></xdr:pic>
  </xdr:oneCellAnchor>
</xdr:wsDr>"#;

const DRAWING_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
                Target="../media/image1.png"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
                Target="../media/missing.png"/>
</Relationships>"#;

fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        for (name, body) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

/// Части пакета с чертежом; `drawing` добавляет сам чертёж и его связи.
fn package_entries(drawing: bool) -> Vec<(&'static str, &'static [u8])> {
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", WORKBOOK.as_bytes()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
        ("xl/worksheets/sheet1.xml", SHEET.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_RELS.as_bytes()),
        ("xl/media/image1.png", b"\x89PNG\r\n\x1a\n"),
    ];
    if drawing {
        entries.push(("xl/drawings/drawing1.xml", DRAWING.as_bytes()));
        entries.push((
            "xl/drawings/_rels/drawing1.xml.rels",
            DRAWING_RELS.as_bytes(),
        ));
    }
    entries
}

/// Полный пакет с чертежом или без него.
fn package_with_drawing(drawing: bool) -> Vec<u8> {
    package(&package_entries(drawing))
}

/// Битая ссылка на media: отсутствующая связь и отсутствующая часть не роняют
/// открытие, но media у таких картинок не разрешается, а id не назначается.
#[test]
fn broken_media_links_keep_the_anchor_without_media() {
    let book = open(package_with_drawing(true)).unwrap();
    let images = &book.sheets()[0].images;

    assert_eq!(images.len(), 3);
    assert_eq!(images[0].media.as_deref(), Some("xl/media/image1.png"));
    // rId2 в связях чертежа не объявлена вовсе.
    assert_eq!(images[1].media, None);
    // rId3 ссылается на часть, которой нет в пакете.
    assert_eq!(images[2].media, None);
    assert_eq!(images[2].name.as_deref(), Some("Picture 3"));
    assert!(matches!(images[2].anchor, ImageAnchor::OneCell { .. }));

    assert_eq!(images[0].image_id, Some(0));
    assert_eq!(images[1].image_id, None);
    assert_eq!(images[2].image_id, None);
    assert_eq!(book.images().len(), 1);
    assert_eq!(book.images()[0].bytes, b"\x89PNG\r\n\x1a\n");
    assert_eq!(book.images()[0].mime, "image/png");
}

/// Отсутствующий чертёж — не ошибка: лист остаётся без картинок.
#[test]
fn missing_drawing_part_opens_without_images() {
    let book = open(package_with_drawing(false)).unwrap();

    assert_eq!(book.sheets().len(), 1);
    assert!(book.sheets()[0].images.is_empty());
    assert!(book.images().is_empty());
}

/// Media, на которую никто не ссылается, в реестр не попадает: реестр — не
/// свалка частей пакета, а то, что нужно нарисовать.
#[test]
fn unreferenced_media_is_not_stored() {
    let mut entries = package_entries(true);
    entries.push(("xl/media/orphan.png", b"orphan"));
    let book = open(package(&entries)).unwrap();

    assert_eq!(book.images().len(), 1);
    assert_eq!(book.images()[0].media, "xl/media/image1.png");
}

/// PNG-книга: обе картинки ссылаются на одну часть — запись в реестре одна,
/// id у картинок совпадают, байты настоящие.
#[test]
fn png_book_registry_keeps_shared_media_bytes() {
    let book = open_fixture("images-png.xlsx");
    assert_eq!(book.images().len(), 1);

    let stored = &book.images()[0];
    assert_eq!(stored.id, 0);
    assert_eq!(stored.media, "xl/media/image1.png");
    assert_eq!(stored.mime, "image/png");
    assert_eq!(&stored.bytes[..8], b"\x89PNG\r\n\x1a\n");

    let ids: Vec<_> = book.sheets()[0]
        .images
        .iter()
        .map(|image| image.image_id)
        .collect();
    assert_eq!(ids, [Some(0), Some(0)]);
}

/// JPEG-книга: MIME отличается от PNG, байты непусты.
#[test]
fn jpeg_book_registry_keeps_jpeg_bytes() {
    let book = open_fixture("images-jpeg.xlsx");
    assert_eq!(book.images().len(), 1);

    let stored = &book.images()[0];
    assert_eq!(stored.id, 0);
    assert_eq!(stored.media, "xl/media/image1.jpeg");
    assert_eq!(stored.mime, "image/jpeg");
    assert_eq!(&stored.bytes[..2], &[0xFF, 0xD8]);
    assert_eq!(book.sheets()[0].images[0].image_id, Some(0));
}

/// Книга «поверх данных»: две части, три картинки; id идут по порядку первого
/// упоминания, а повторы делят запись.
#[test]
fn over_data_book_registry_dedups_media_and_numbers_ids() {
    let book = open_fixture("images-over-data.xlsx");
    let registry = book.images();
    assert_eq!(registry.len(), 2);
    assert_eq!(registry[0].media, "xl/media/image1.png");
    assert_eq!(registry[0].mime, "image/png");
    assert_eq!(registry[1].media, "xl/media/image2.jpeg");
    assert_eq!(registry[1].mime, "image/jpeg");
    assert!(registry.iter().all(|image| !image.bytes.is_empty()));

    let ids: Vec<_> = book.sheets()[0]
        .images
        .iter()
        .map(|image| image.image_id)
        .collect();
    assert_eq!(ids, [Some(0), Some(1), Some(0)]);
}

/// Id стабильны: повторное открытие той же книги даёт тот же реестр.
#[test]
fn image_ids_are_stable_across_opens() {
    let first = open_fixture("images-over-data.xlsx");
    let second = open_fixture("images-over-data.xlsx");

    assert_eq!(first.images(), second.images());
    let ids = |book: &Workbook| -> Vec<_> {
        book.sheets()[0]
            .images
            .iter()
            .map(|image| image.image_id)
            .collect()
    };
    assert_eq!(ids(&first), ids(&second));
}

/// Книга без картинок даёт пустой реестр, а не ошибку.
#[test]
fn book_without_images_has_empty_registry() {
    let book = open_fixture("content-single-cell.xlsx");

    assert!(book.images().is_empty());
    assert!(book.image(0).is_none());
}
