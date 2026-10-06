//! Сквозные проверки: лист XLSX → PDF.
//!
//! Фикстуры — те же книги из `test-fixtures/xlsx`, что и у `crates/xlsx`
//! (собраны `scripts/gen-fixtures.ts`). Проверки структурные: файл парсится
//! `lopdf`, страница одна, текст извлекается по `ToUnicode` — то же, что
//! делает `pdftotext` в приёмке спринта.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Экспортировать лист книги настройками по умолчанию.
fn export(name: &str, sheet: usize) -> Vec<u8> {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
    )
    .unwrap_or_else(|e| panic!("{name} не открылась: {e}"));
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, sheet)
        .unwrap_or_else(|e| panic!("{name} не экспортировалась: {e}"))
}

/// Есть ли последовательность байт в буфере.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Текст первой страницы, восстановленный картой `ToUnicode`.
///
/// `lopdf::extract_text` принимает в CMap только `CIDSystemInfo`, `CMapName` и
/// `CMapType` (падает на `/CMapVersion` и `/WMode`, которые printpdf пишет
/// всегда), а `pdftotext` из приёмки спринта лишние ключи игнорирует. Здесь
/// повторяется то, что делает `pdftotext`: коды из `Tj` переводятся картой
/// `ToUnicode` обратно в символы.
fn decoded_text(bytes: &[u8]) -> String {
    let doc = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    let cmap = tounicode_map(&doc);
    let page = *doc.get_pages().values().next().expect("страница есть");
    let content = doc.get_page_content(page).expect("поток содержимого");
    decode_content(&content, &cmap)
}

/// Пары «код в потоке → символ Unicode» из `beginbfchar`-секции CMap.
///
/// В PDF ровно одна карта: шрифт в документе один.
fn tounicode_map(doc: &lopdf::Document) -> HashMap<u16, char> {
    for object in doc.objects.values() {
        let Ok(stream) = object.as_stream() else {
            continue;
        };
        let Ok(content) = stream.get_plain_content() else {
            continue;
        };
        let text = String::from_utf8_lossy(&content);
        if text.contains("beginbfchar") {
            return parse_bfchar(&text);
        }
    }
    panic!("в PDF нет ToUnicode CMap");
}

/// Разобрать `beginbfchar`-секцию: строки вида `<0001> <0061>`.
fn parse_bfchar(cmap: &str) -> HashMap<u16, char> {
    let mut map = HashMap::new();
    let mut inside = false;
    for line in cmap.lines() {
        let line = line.trim();
        if line.ends_with("beginbfchar") {
            inside = true;
        } else if line == "endbfchar" {
            inside = false;
        } else if inside {
            let Some((source, target)) = line.split_once(' ') else {
                continue;
            };
            let code = u16::from_str_radix(source.trim_matches(['<', '>']), 16);
            let value = u16::from_str_radix(target.trim_matches(['<', '>']), 16);
            if let (Ok(code), Ok(value)) = (code, value) {
                if let Some(ch) = char::from_u32(u32::from(value)) {
                    map.insert(code, ch);
                }
            }
        }
    }
    map
}

/// Раскодировать hex-строки перед `Tj` (по одной на нарисованную строку текста).
fn decode_content(content: &[u8], cmap: &HashMap<u16, char>) -> String {
    let text = String::from_utf8_lossy(content);
    let mut out = String::new();
    for chunk in text.split("Tj") {
        let Some(open) = chunk.rfind('<') else {
            continue;
        };
        let Some(close) = chunk[open..].find('>') else {
            continue;
        };
        let hex = &chunk[open + 1..open + close];
        for pair in hex.as_bytes().chunks(4) {
            let Ok(code) = u16::from_str_radix(&String::from_utf8_lossy(pair), 16) else {
                continue;
            };
            if let Some(ch) = cmap.get(&code) {
                out.push(*ch);
            }
        }
        out.push(' ');
    }
    out
}

#[test]
fn sheet_becomes_single_page_pdf() {
    let bytes = export("content-mixed-types.xlsx", 0);

    assert!(
        bytes.starts_with(b"%PDF-"),
        "нет сигнатуры PDF: {:?}",
        &bytes[..bytes.len().min(8)]
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    assert_eq!(doc.get_pages().len(), 1, "страница должна быть одна");

    // Шрифт встроен подмножеством: Type0/Identity-H требует FontFile2.
    assert!(
        contains(&bytes, b"FontFile2"),
        "в PDF нет встроенного подмножества шрифта"
    );

    let page = *doc.get_pages().values().next().expect("страница есть");
    let content = doc.get_page_content(page).expect("поток содержимого");
    let content = String::from_utf8_lossy(&content);
    assert!(content.contains("BT"), "в потоке нет текстовых секций");
    assert!(
        content.contains("Tj") || content.contains("TJ"),
        "в потоке нет операторов вывода текста"
    );
}

#[test]
fn cyrillic_text_round_trips_through_tounicode() {
    let bytes = export("values-unicode.xlsx", 0);
    assert!(
        contains(&bytes, b"ToUnicode"),
        "в PDF нет карты ToUnicode — кириллицу не извлечь"
    );

    let text = decoded_text(&bytes);
    assert!(
        text.contains("Тире — и дефис - разные"),
        "кириллица потерялась: {text:?}"
    );
    assert!(text.contains("эмодзи"), "кириллица потерялась: {text:?}");
}

#[test]
fn cell_text_reaches_the_page() {
    let bytes = export("values-strings.xlsx", 0);
    let text = decoded_text(&bytes);

    assert!(text.contains("Привет"), "текст ячейки потерялся: {text:?}");
    assert!(
        text.contains("Hello, world"),
        "текст ячейки потерялся: {text:?}"
    );
}

#[test]
fn long_sheet_is_split_into_pages() {
    // 600 строк по 15 px и A4 с полями по умолчанию: на страницу их влезает
    // 64, поэтому страниц должно быть не меньше десяти.
    let bytes = export("scale-ten-pages.xlsx", 0);

    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages().len();
    assert!(pages >= 10, "лист не разбит на страницы: страниц {pages}");

    // Текст первой и последней страниц не пуст: содержимое не потерялось при
    // разбивке.
    for (index, page) in doc.get_pages().into_iter() {
        let content = doc.get_page_content(page).expect("поток содержимого");
        let content = String::from_utf8_lossy(&content);
        assert!(
            content.contains("Tj"),
            "страница {index} пуста: при разбивке потерялся текст"
        );
    }
}

#[test]
fn short_sheet_stays_one_page() {
    let bytes = export("content-single-cell.xlsx", 0);

    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    assert_eq!(doc.get_pages().len(), 1, "короткий лист — одна страница");
}

#[test]
fn empty_sheet_is_still_a_page() {
    let bytes = export("content-empty-sheet.xlsx", 0);

    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    assert_eq!(doc.get_pages().len(), 1, "пустой лист — всё равно страница");
}

#[test]
fn out_of_range_sheet_is_an_error() {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture("content-single-cell.xlsx")).expect("фикстура читается"),
    )
    .expect("книга открывается");
    let result = PdfExporter::new(PdfOptions::default()).export_xlsx_sheet(&book, 7);

    assert!(result.is_err(), "несуществующий лист должен быть ошибкой");
}
