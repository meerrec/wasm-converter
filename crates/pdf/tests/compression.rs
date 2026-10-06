//! Регресс-тесты сжатия форка printpdf (ADR-0010, эпик G).
//!
//! Форк несёт три правки в `vendor/printpdf/src/serialize.rs`: сжатие потока
//! содержимого страницы, сжатие шрифтового потока и вызов `Document::compress()`
//! при `optimize`. Откат любой из трёх роняет эти проверки — в этом и смысл
//! файла. Снаружи сжатие включает `PdfOptions::compress` (в printpdf он же
//! `PdfSaveOptions::optimize`).

use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};
use lopdf::{Document, Object, Stream};

/// Книга с 2000 ячейками: поток содержимого достаточно большой, чтобы
/// `FlateDecode` окупился (lopdf сжимает только при экономии ≥ 19 байт).
const FIXTURE: &str = "content-dense.xlsx";

/// Путь к книге в `test-fixtures/xlsx` (как в `export.rs`).
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Экспортировать первый лист книги с указанным сжатием.
fn export(compress: bool) -> Vec<u8> {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture(FIXTURE)).unwrap_or_else(|err| panic!("{FIXTURE}: {err}")),
    )
    .unwrap_or_else(|err| panic!("{FIXTURE} не открылась: {err}"));
    let options = PdfOptions {
        compress,
        ..PdfOptions::default()
    };
    PdfExporter::new(options)
        .export_xlsx_sheet(&book, 0)
        .unwrap_or_else(|err| panic!("{FIXTURE} не экспортировалась: {err}"))
}

/// Первая страница документа.
fn first_page(doc: &Document) -> lopdf::ObjectId {
    *doc.get_pages().values().next().expect("в PDF нет страниц")
}

/// Поток содержимого первой страницы — в том виде, в каком его записал printpdf.
fn content_stream(doc: &Document) -> &Stream {
    let page = first_page(doc);
    let contents = doc
        .get_object(page)
        .expect("страница есть")
        .as_dict()
        .expect("страница — словарь")
        .get(b"Contents")
        .expect("у страницы нет /Contents");
    let id = contents
        .as_reference()
        .expect("/Contents страницы не ссылка на поток");
    doc.get_object(id)
        .expect("объект содержимого есть")
        .as_stream()
        .expect("/Contents не поток")
}

/// Ссылка на шрифтовой поток `FontFile2` шрифта: `DescendantFonts[0]` →
/// `FontDescriptor` → `FontFile2` (та же цепочка, что в `export.rs`).
fn font_file_id(doc: &Document, font: &lopdf::Dictionary) -> Option<lopdf::ObjectId> {
    let descriptor = if let Ok(descendants) = font.get(b"DescendantFonts") {
        let first = descendants.as_array().ok()?.first()?;
        let descendant = match first {
            Object::Reference(id) => doc.get_object(*id).ok()?,
            other => other,
        };
        descendant
            .as_dict()
            .ok()?
            .get(b"FontDescriptor")
            .ok()?
            .as_reference()
            .ok()?
    } else {
        font.get(b"FontDescriptor").ok()?.as_reference().ok()?
    };
    doc.get_object(descriptor)
        .ok()?
        .as_dict()
        .ok()?
        .get(b"FontFile2")
        .ok()?
        .as_reference()
        .ok()
}

/// Шрифтовой поток первой страницы.
fn font_stream(doc: &Document) -> &Stream {
    let page = first_page(doc);
    for (_, font) in doc.get_page_fonts(page).expect("шрифты страницы") {
        if let Some(id) = font_file_id(doc, font) {
            return doc
                .get_object(id)
                .expect("объект FontFile2 есть")
                .as_stream()
                .expect("FontFile2 не поток");
        }
    }
    panic!("в PDF нет встроенного шрифта");
}

/// Есть ли на потоке фильтр `FlateDecode` (имя или элемент массива).
fn is_flate(stream: &Stream) -> bool {
    stream.filters().is_ok_and(|filters| {
        filters
            .iter()
            .any(|filter| *filter == b"FlateDecode".as_slice())
    })
}

#[test]
fn compress_puts_flate_on_page_and_font_streams() {
    let bytes = export(true);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");

    assert!(
        is_flate(content_stream(&doc)),
        "поток содержимого страницы без /FlateDecode — правка (a) форка не применилась"
    );
    assert!(
        is_flate(font_stream(&doc)),
        "шрифтовой поток без /FlateDecode — правка (b) форка не применилась"
    );
}

#[test]
fn length1_keeps_the_uncompressed_font_size() {
    let bytes = export(true);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let font = font_stream(&doc);

    // ISO 32000: /Length1 — размер распакованной программы; сжимается только
    // сам поток, и lopdf при сжатии перезаписывает лишь /Length.
    let length1 = font
        .dict
        .get(b"Length1")
        .and_then(Object::as_i64)
        .expect("у шрифтового потока нет /Length1");
    let plain = font.get_plain_content().expect("шрифт распаковывается");
    assert_eq!(
        plain.len() as i64,
        length1,
        "/Length1 не равен распакованному размеру программы шрифта"
    );

    let length = font
        .dict
        .get(b"Length")
        .and_then(Object::as_i64)
        .expect("у шрифтового потока нет /Length");
    assert_eq!(
        length,
        font.content.len() as i64,
        "/Length не пересчитан после сжатия"
    );
}

#[test]
fn without_compress_streams_stay_unfiltered() {
    let bytes = export(false);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");

    assert!(
        !is_flate(content_stream(&doc)),
        "при compress: false поток содержимого сжался"
    );
    assert!(
        !is_flate(font_stream(&doc)),
        "при compress: false шрифтовой поток сжался"
    );
}

#[test]
fn compressed_pdf_is_much_smaller() {
    let raw = export(false);
    let packed = export(true);
    eprintln!(
        "{FIXTURE}: без сжатия {} байт, со сжатием {} байт",
        raw.len(),
        packed.len()
    );

    assert!(
        packed.len() < raw.len(),
        "сжатый PDF не меньше несжатого: {} против {}",
        packed.len(),
        raw.len()
    );
    // План G4: минимум в 1,5 раза. Без вызова doc.compress() (правка (c))
    // размеры совпадают, и порог не набирается.
    assert!(
        packed.len() * 3 < raw.len() * 2,
        "сжатие меньше 1,5×: {} против {}",
        packed.len(),
        raw.len()
    );
}
