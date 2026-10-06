//! Sink-путь экспорта: байты PDF уходят в приёмник вызывающего
//! (`export_xlsx_sheet_to`), а не возвращаются готовым `Vec<u8>`.
//!
//! Сравнение путей — постраничное: побайтовое невозможно, printpdf берёт имена
//! шрифтов и `/ID` из глобального счётчика случайности (`RAND_SEED`), и каждый
//! следующий экспорт в процессе даёт другие байты (см. `pagination.rs`). Потоки
//! содержимого страниц от счётчика не зависят — сверяем их, замаскировав
//! случайные имена.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Фикстура на 13 страниц: и пагинация, и потоки содержимого нетривиальны.
const FIXTURE: &str = "scale-ten-pages.xlsx";

/// Сколько страниц обязана давать эта книга при настройках по умолчанию
/// (эталон — `pagination.rs`).
const PAGES: usize = 13;

/// Путь к книге в `test-fixtures/xlsx` (как в `pagination.rs`).
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из общего набора фикстур.
fn open() -> doc_converter_xlsx::Workbook {
    let path = fixture(FIXTURE);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{FIXTURE}: {err}"))
}

/// Экспорт листа 0 старым путём — готовым `Vec<u8>`.
fn export_vec() -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&open(), 0)
        .unwrap_or_else(|err| panic!("{FIXTURE} не экспортировалась: {err}"))
}

/// Экспорт листа 0 через sink.
fn export_sink<W: Write>(out: &mut W) {
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet_to(&open(), 0, out)
        .unwrap_or_else(|err| panic!("{FIXTURE} не экспортировалась через sink: {err}"));
}

/// Заменить случайные имена шрифтов заглушкой (копия из `pagination.rs`).
///
/// `random_character_string_32` собирает имя из цифр числа, переведённых в
/// `A`–`J`, — ровно 32 символа. В данных фикстур таких длинных цепочек `A`–`J`
/// нет, поэтому замена не задевает содержимое ячеек.
fn mask_random_font_names(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if !(b'A'..=b'J').contains(&bytes[index]) {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (b'A'..=b'J').contains(&bytes[index]) {
            index += 1;
        }
        if index - start >= 32 {
            out.extend_from_slice(b"RANDOM");
        } else {
            out.extend_from_slice(&bytes[start..index]);
        }
    }
    out
}

/// Потоки содержимого страниц по порядку страниц, со снятыми случайными
/// именами.
fn page_contents(bytes: &[u8]) -> Vec<Vec<u8>> {
    let doc = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    doc.get_pages()
        .values()
        .map(|id| {
            let content = doc.get_page_content(*id).expect("поток содержимого");
            mask_random_font_names(&content)
        })
        .collect()
}

/// Сравнить страницы, не вываливая в сообщение килобайты содержимого.
fn assert_same_pages(expected: &[Vec<u8>], actual: &[Vec<u8>]) {
    assert_eq!(actual.len(), expected.len(), "число страниц разошлось");
    for (index, (expected_page, actual_page)) in expected.iter().zip(actual).enumerate() {
        if expected_page == actual_page {
            continue;
        }
        let first_diff = expected_page
            .iter()
            .zip(actual_page)
            .position(|(expected_byte, actual_byte)| expected_byte != actual_byte);
        panic!(
            "страница {}: потоки содержимого разошлись ({} и {} байт, первое расхождение: {first_diff:?})",
            index + 1,
            expected_page.len(),
            actual_page.len(),
        );
    }
}

#[test]
fn sink_yields_same_pages_as_vec_path() {
    let mut bytes = Vec::new();
    export_sink(&mut bytes);
    let pages = page_contents(&bytes);
    assert_eq!(pages.len(), PAGES, "{FIXTURE}: число страниц изменилось");
    assert_same_pages(&page_contents(&export_vec()), &pages);
}

/// Приёмник, принимающий не больше 64 байт за вызов `write`: так ведёт себя
/// транспорт с чанками фиксированного размера, а докрутить остаток обязан
/// вызывающий (`write_all`).
struct DripWriter {
    bytes: Vec<u8>,
}

impl Write for DripWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let take = buf.len().min(64);
        self.bytes.extend_from_slice(&buf[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn sink_survives_partial_writes() {
    let mut writer = DripWriter { bytes: Vec::new() };
    export_sink(&mut writer);
    assert_same_pages(&page_contents(&export_vec()), &page_contents(&writer.bytes));
}

/// Приёмник, который сразу отказывает.
struct BrokenWriter;

impl Write for BrokenWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "sink закрыт"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn sink_write_error_becomes_export_error() {
    let err = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet_to(&open(), 0, &mut BrokenWriter)
        .expect_err("запись в закрытый sink обязана провалиться");
    assert!(
        matches!(err, doc_converter_core::Error::Export(_)),
        "ошибка sink не стала Export: {err:?}"
    );
}
