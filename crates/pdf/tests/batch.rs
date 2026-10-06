//! Batch-экспорт: все листы книги одним PDF.
//!
//! Решение H1 — один документ, а не файл на лист (так печатает книгу Excel):
//! листы идут подряд своими страницами, закладка на лист остаётся навигацией
//! по книге. Проверки структурные — файл разбирается `lopdf`, текст
//! восстанавливается картами `ToUnicode`, как в `tests/export.rs`.
//!
//! Числа страниц листов не зашиты в тесты: эталон — одиночный экспорт
//! (`export_xlsx_sheet`), который к тому же и регресс-барьер: batch-путь не
//! должен его менять.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_xlsx::Workbook;
use lopdf::{Dictionary, Document, Object, ObjectId};

/// Путь к книге в общем наборе фикстур.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из фикстур.
fn open_book(name: &str) -> Workbook {
    let path = fixture(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name}: {err}"))
}

/// PDF разбирается `lopdf`.
fn parse(bytes: &[u8]) -> Document {
    Document::load_mem(bytes).expect("PDF разбирается lopdf")
}

/// Экспорт всей книги одним документом — проверяемый путь.
fn export_book(book: &Workbook) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_book(book)
        .unwrap_or_else(|err| panic!("экспорт книги не удался: {err}"))
}

/// Одиночный экспорт листа — прежний путь и эталон для сравнения.
fn export_sheet(book: &Workbook, sheet: usize) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(book, sheet)
        .unwrap_or_else(|err| panic!("экспорт листа {sheet} не удался: {err}"))
}

/// Число страниц документа.
fn page_count(doc: &Document) -> usize {
    doc.get_pages().len()
}

/// Словарь `/Outlines` из каталога; `None` — дерева закладок в PDF нет.
fn outlines(doc: &Document) -> Option<Dictionary> {
    let catalog = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .and_then(|id| doc.get_dictionary(id))
        .expect("каталог документа");
    let outlines = catalog.get(b"Outlines").ok()?;
    let (_, outlines) = doc.dereference(outlines).expect("разыменование /Outlines");
    Some(outlines.as_dict().expect("/Outlines — словарь").clone())
}

/// Пункты outline по порядку: заголовок и номер страницы (с единицы), на
/// которую ведёт `/Dest`.
fn outline_items(doc: &Document) -> Vec<(String, usize)> {
    let Some(root) = outlines(doc) else {
        return Vec::new();
    };
    let pages = doc.get_pages();
    let mut items = Vec::new();
    let mut next = root
        .get(b"First")
        .ok()
        .and_then(|object| object.as_reference().ok());
    while let Some(id) = next {
        let item = doc.get_dictionary(id).expect("пункт outline — словарь");
        let title = decode_title(item.get(b"Title").expect("/Title"));
        let dest = item
            .get(b"Dest")
            .expect("/Dest")
            .as_array()
            .expect("/Dest — массив");
        let target = dest[0].as_reference().expect("/Dest ссылается на страницу");
        let number = pages
            .iter()
            .find(|(_, page)| **page == target)
            .unwrap_or_else(|| panic!("страница {target:?} не найдена"))
            .0;
        items.push((title, *number as usize));
        next = item
            .get(b"Next")
            .ok()
            .and_then(|object| object.as_reference().ok());
    }
    items
}

/// Заголовок закладки: printpdf пишет его в UTF-16BE с BOM.
fn decode_title(object: &Object) -> String {
    let bytes = object.as_str().expect("заголовок закладки — строка");
    assert_eq!(&bytes[..2], &[0xFE, 0xFF], "заголовок в UTF-16BE с BOM");
    let units: Vec<u16> = bytes[2..]
        .chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).expect("заголовок — UTF-16")
}

/// Текст страницы (с единицы), восстановленный картами `ToUnicode` её шрифтов.
///
/// `lopdf::extract_text` на PDF printpdf падает (см. `tests/export.rs`), поэтому
/// здесь повторяется то, что делает `pdftotext`: hex-строки перед `Tj`
/// переводятся картой шрифта обратно в символы.
fn page_text(doc: &Document, number: u32) -> String {
    let page = *doc
        .get_pages()
        .get(&number)
        .unwrap_or_else(|| panic!("страницы {number} нет"));
    let cmaps = page_cmaps(doc, page);
    let content = doc.get_page_content(page).expect("поток содержимого");
    let content = String::from_utf8_lossy(&content).into_owned();
    // Операторы `Tf`/`Tj` идут в потоке по порядку; в hex-строке ни `T`, ни
    // `j` встретиться не могут (это не hex-цифры), поэтому слияние позиций
    // токенов даёт настоящую последовательность прогонов.
    let mut events: Vec<(usize, &str)> = Vec::new();
    events.extend(content.match_indices("Tf"));
    events.extend(content.match_indices("Tj"));
    events.sort_by_key(|(at, _)| *at);

    let mut font = String::new();
    let mut out = String::new();
    for (at, token) in events {
        if token == "Tf" {
            // Перед оператором стоит `/F2 11` — имя ресурса и кегль.
            if let Some(slash) = content[..at].rfind('/') {
                font = content[slash + 1..at]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
            }
            continue;
        }
        // Перед `Tj` стоит hex-строка `<…>`.
        let Some(open) = content[..at].rfind('<') else {
            continue;
        };
        let Some(close) = content[open..at].find('>') else {
            continue;
        };
        for pair in content.as_bytes()[open + 1..open + close].chunks(4) {
            let Ok(code) = u16::from_str_radix(&String::from_utf8_lossy(pair), 16) else {
                continue;
            };
            if let Some(ch) = cmaps.get(&font).and_then(|map| map.get(&code)) {
                out.push(*ch);
            }
        }
        out.push(' ');
    }
    out
}

/// Карта `ToUnicode` каждого шрифта страницы: имя ресурса → «код → символ».
fn page_cmaps(doc: &Document, page: ObjectId) -> HashMap<String, HashMap<u16, char>> {
    let mut cmaps = HashMap::new();
    for (name, font) in doc.get_page_fonts(page).expect("шрифты страницы") {
        let Ok(to_unicode) = font.get(b"ToUnicode") else {
            continue;
        };
        let Ok(id) = to_unicode.as_reference() else {
            continue;
        };
        let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
            continue;
        };
        let Ok(content) = stream.get_plain_content() else {
            continue;
        };
        cmaps.insert(
            String::from_utf8_lossy(&name).into_owned(),
            parse_bfchar(&String::from_utf8_lossy(&content)),
        );
    }
    cmaps
}

/// Пары «код → символ» из `beginbfchar`-секции CMap.
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

/// Числа страниц листов по отдельным экспортам: эталон для книги.
fn per_sheet_pages(book: &Workbook) -> Vec<usize> {
    (0..book.sheets().len())
        .map(|index| page_count(&parse(&export_sheet(book, index))))
        .collect()
}

/// Книга с тремя листами — целиком в одном PDF.
///
/// DoD H1: ни один лист не теряется, имена листов попадают в закладки.
#[test]
fn book_export_keeps_every_sheet() {
    let book = open_book("sheets-three.xlsx");
    let doc = parse(&export_book(&book));

    assert_eq!(page_count(&doc), 3, "три листа по странице — три страницы");
    for (index, number) in [1_u32, 2, 3].into_iter().enumerate() {
        let text = page_text(&doc, number);
        assert!(
            text.contains(&format!("значение {}", index + 1)),
            "на странице {number} нет содержимого листа {}: {text:?}",
            index + 1
        );
    }

    let items = outline_items(&doc);
    assert_eq!(
        items,
        vec![
            ("Лист1".to_owned(), 1),
            ("Лист2".to_owned(), 2),
            ("Лист3".to_owned(), 3),
        ],
        "закладка на каждый лист, на его первую страницу"
    );
}

/// Число страниц книги — сумма страниц её листов, а не страница на лист:
/// листы разной высоты и занимают не по одной странице.
#[test]
fn book_pages_are_the_sum_of_sheet_pages() {
    let book = open_book("sheets-different-shapes.xlsx");
    let expected = per_sheet_pages(&book);
    assert!(
        expected.iter().any(|pages| *pages > 1),
        "фикстура должна давать лист длиннее страницы, иначе сумма не проверяется"
    );

    let doc = parse(&export_book(&book));
    assert_eq!(page_count(&doc), expected.iter().sum::<usize>());
}

/// Закладки ведут на первую страницу каждого листа: у листа в несколько
/// страниц — на его начало, а не на предыдущий лист.
#[test]
fn bookmarks_point_at_each_sheet_start() {
    let book = open_book("sheets-different-shapes.xlsx");
    let pages = per_sheet_pages(&book);
    let doc = parse(&export_book(&book));

    let mut expected = Vec::new();
    let mut start = 1;
    for (index, sheet) in book.sheets().iter().enumerate() {
        expected.push((sheet.meta.name.clone(), start));
        start += pages[index];
    }
    assert_eq!(outline_items(&doc), expected);
}

/// Книга с одним листом проходит batch-путём и не отличается от одиночного
/// экспорта ни страницами, ни закладкой.
#[test]
fn single_sheet_book_matches_the_sheet_path() {
    let book = open_book("size-500x5x1.xlsx");
    assert_eq!(book.sheets().len(), 1, "фикстура — книга с одним листом");

    let book_doc = parse(&export_book(&book));
    let sheet_doc = parse(&export_sheet(&book, 0));
    assert_eq!(page_count(&book_doc), page_count(&sheet_doc));
    assert!(
        page_count(&book_doc) > 1,
        "лист длиннее страницы: сравнение должно быть на нескольких страницах"
    );
    assert_eq!(
        outline_items(&book_doc),
        outline_items(&sheet_doc),
        "закладка книги и листа — та же"
    );
}

/// Пустой лист не роняет экспорт: он даёт страницу (как и в одиночном пути),
/// и закладка на него тоже есть.
#[test]
fn empty_sheet_does_not_break_the_book() {
    let book = open_book("sheets-empty-second.xlsx");
    let doc = parse(&export_book(&book));

    assert_eq!(page_count(&doc), 2, "пустой лист занимает свою страницу");
    assert_eq!(
        outline_items(&doc),
        vec![("С данными".to_owned(), 1), ("Пустой".to_owned(), 2)]
    );
    assert!(
        page_text(&doc, 1).contains('a'),
        "содержимое первого листа на месте"
    );
}

/// Скрытый лист попадает в документ: список листов просмотрщика строится по
/// модели книги (`SheetInfo.hidden` — пометка, а не фильтр), и терять лист
/// молча batch-путь не должен.
#[test]
fn hidden_sheets_are_exported() {
    let book = open_book("sheets-hidden.xlsx");
    let doc = parse(&export_book(&book));

    assert_eq!(page_count(&doc), 2);
    assert_eq!(
        outline_items(&doc),
        vec![("Видимый".to_owned(), 1), ("Скрытый".to_owned(), 2)]
    );
}

/// Одиночный экспорт не изменился: лист выбирается по индексу, а не по
/// порядку в книге. Это регресс-барьер для прежнего пути.
#[test]
fn single_sheet_export_is_unchanged() {
    let book = open_book("sheets-three.xlsx");
    let doc = parse(&export_sheet(&book, 1));

    assert_eq!(page_count(&doc), 1);
    let text = page_text(&doc, 1);
    assert!(
        text.contains("значение 2"),
        "экспортирован не тот лист: {text:?}"
    );
    assert!(!text.contains("значение 1"));
    assert_eq!(
        outline_items(&doc),
        vec![("Лист2".to_owned(), 1)],
        "закладка — на экспортированный лист"
    );
}
