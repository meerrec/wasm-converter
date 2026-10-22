//! Колонтитулы DOCX в собранном PDF: текст на каждой странице и пути элементов.
//!
//! Фикстуры — те же из `test-fixtures/docx`, что читает `crates/docx`. Проверки
//! структурные, теми же приёмами, что у `tests/docx_tables.rs`: PDF разбирается
//! `lopdf`, текст восстанавливается картами `ToUnicode`. Фикстуры колонтитулов
//! одностраничные, поэтому тело растягивается копиями (приём `crates/docx`):
//! вырожденный случай одной страницы не отличил бы «на каждой странице» от
//! «только на первой».
//!
//! `pedantic` включён здесь, чтобы линты совпадали с крейтовыми
//! (`#![deny(clippy::pedantic)]` в `lib.rs`).
#![deny(clippy::pedantic)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_core::NodeId;
use doc_converter_docx::layout::{layout_document, LayoutItem, LayoutOptions, PageLayout};
use doc_converter_docx::{
    BlockItem, Cell, CellBorders, CellMargins, CellVAlign, Document, GridCol, Row, Table,
    TableBorders, TableLayout, TableLook, Twips,
};
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::FontRegistry;
use lopdf::Document as Pdf;

/// Размер LRU-кэша метрик — как у крейта (`painter::FONT_CACHE`).
const FONT_CACHE: usize = 4096;

/// Путь к фикстуре в `test-fixtures/docx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/docx")
        .join(name)
}

/// Открыть фикстуру; её отсутствие роняет тест, а не пропускает его.
fn document(name: &str) -> Document {
    let path = fixture(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    doc_converter_docx::open(bytes).unwrap_or_else(|e| panic!("{name} не открылся: {e}"))
}

/// Разложить документ так же, как это сделает экспорт.
fn layout(doc: &Document) -> PageLayout {
    layout_document(
        doc,
        &LayoutOptions::default(),
        &mut FontRegistry::new(FONT_CACHE),
    )
    .expect("раскладка собирается")
}

/// Экспортировать документ настройками по умолчанию.
fn export(doc: &Document) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_docx(doc)
        .expect("документ экспортируется")
}

/// Повторить тело `times` раз, чтобы документ занял несколько страниц.
///
/// Копии вставляются перед финальным `w:sectPr` (приём `crates/docx`): раз-
/// множенный конец секции разрезал бы документ, и колонтитулы стало бы не с чем
/// сверять.
fn stretch(document: &mut Document, times: usize) {
    let body = document.body.items.clone();
    let at = document
        .body
        .items
        .iter()
        .rposition(|block| matches!(block, BlockItem::SectPr(_)))
        .unwrap_or(document.body.items.len());

    let mut items: Vec<BlockItem> = Vec::with_capacity(body.len() * times);
    for _ in 0..times {
        items.extend(body[..at].iter().cloned());
    }
    items.extend(body[at..].iter().cloned());
    document.body.items = items;
}

/// Текст страницы `index` (с нуля), восстановленный картами `ToUnicode`.
fn page_text(bytes: &[u8], index: usize) -> String {
    let pdf = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    let pages = pdf.get_pages();
    let id = *pages
        .values()
        .nth(index)
        .unwrap_or_else(|| panic!("страницы {index} нет"));
    text_runs(&pdf, id)
        .into_iter()
        .map(|(_, run)| run)
        .collect()
}

/// Карта `ToUnicode` каждого шрифта страницы: имя ресурса → «код → символ».
fn page_cmaps(pdf: &Pdf, page: lopdf::ObjectId) -> HashMap<String, HashMap<u16, char>> {
    let mut cmaps = HashMap::new();
    for (name, font) in pdf.get_page_fonts(page).expect("шрифты страницы") {
        let Ok(to_unicode) = font.get(b"ToUnicode") else {
            continue;
        };
        let Ok(id) = to_unicode.as_reference() else {
            continue;
        };
        let Ok(stream) = pdf.get_object(id).and_then(lopdf::Object::as_stream) else {
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

/// Пары «код в потоке → символ Unicode» из `beginbfchar`-секции `CMap`.
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

/// Прогон текста в потоке содержимого: имя ресурса шрифта и строка.
fn text_runs(pdf: &Pdf, page: lopdf::ObjectId) -> Vec<(String, String)> {
    let cmaps = page_cmaps(pdf, page);
    let content = pdf.get_page_content(page);
    let text = String::from_utf8_lossy(&content);

    // Операторы `Tf`/`Tj` идут в потоке по порядку; в hex-строке ни `T`, ни
    // `j` встретиться не могут (это не hex-цифры), поэтому слияние позиций
    // токенов даёт настоящую последовательность прогонов (приём `tests/docx.rs`).
    let mut events: Vec<(usize, &str)> = Vec::new();
    events.extend(text.match_indices("Tf"));
    events.extend(text.match_indices("Tj"));
    events.sort_by_key(|(at, _)| *at);

    let mut runs = Vec::new();
    let mut font = String::new();
    for (at, token) in events {
        if token == "Tf" {
            if let Some(slash) = text[..at].rfind('/') {
                text[slash + 1..at]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .clone_into(&mut font);
            }
            continue;
        }
        let Some(open) = text[..at].rfind('<') else {
            continue;
        };
        let Some(close) = text[open..at].find('>') else {
            continue;
        };
        let hex = &text[open + 1..open + close];
        let mut decoded = String::new();
        for pair in hex.as_bytes().chunks(4) {
            let Ok(code) = u16::from_str_radix(&String::from_utf8_lossy(pair), 16) else {
                continue;
            };
            if let Some(ch) = cmaps.get(&font).and_then(|map| map.get(&code)) {
                decoded.push(*ch);
            }
        }
        runs.push((font.clone(), decoded));
    }
    runs
}

/// Таблица из одной строки и одной ячейки: фикстуры с таблицей в колонтитуле
/// нет, а через этот путь идёт содержимое, которое видно только диспетчеру.
fn a_table(items: Vec<BlockItem>) -> Table {
    Table {
        id: NodeId::new(9000),
        style_ref: None,
        grid: vec![GridCol {
            width: Twips::new(6000),
        }],
        rows: vec![Row {
            id: NodeId::new(9001),
            cells: vec![Cell {
                id: NodeId::new(9002),
                grid_span: 1,
                v_merge: None,
                width: None,
                margins: CellMargins::default(),
                v_align: CellVAlign::Top,
                borders: Box::new(CellBorders::default()),
                shading: None,
                items,
            }],
            height: None,
            cant_split: false,
            header: false,
        }],
        layout: TableLayout::Autofit,
        width: None,
        borders: TableBorders::default(),
        look: TableLook::default(),
        jc: None,
        indent: None,
        cell_margins: CellMargins::default(),
        bidi_visual: false,
    }
}

/// Текст колонтитула доходит до потока каждой страницы, а не только первой.
///
/// Фикстура — «Default header»/«Default footer» на всех страницах; тело
/// растянуто, чтобы страниц было несколько. Сверка идёт с числами раскладки:
/// сколько страниц разложилось — столько и обязано быть в PDF, и у каждой свой
/// колонтитул.
#[test]
fn header_and_footer_text_reaches_every_page() {
    let name = "headers_footers/default_header_footer.docx";
    let mut doc = document(name);
    stretch(&mut doc, 150);
    let layout = layout(&doc);
    assert!(
        layout.pages.len() >= 3,
        "{name}: тело растянуто на несколько страниц, получено {}",
        layout.pages.len()
    );
    assert_eq!(
        layout.chrome.len(),
        layout.pages.len(),
        "{name}: колонтитул у каждой страницы раскладки"
    );

    let bytes = export(&doc);
    let pdf = Pdf::load_mem(&bytes).expect("PDF разбирается lopdf");
    assert_eq!(
        pdf.get_pages().len(),
        layout.pages.len(),
        "{name}: страниц PDF столько же, сколько в раскладке"
    );

    for index in 0..layout.pages.len() {
        let text = page_text(&bytes, index);
        assert!(
            text.contains("Default header"),
            "{name}: страница {} без верхнего колонтитула: {text:?}",
            index + 1
        );
        assert!(
            text.contains("Default footer"),
            "{name}: страница {} без нижнего колонтитула: {text:?}",
            index + 1
        );
    }
}

/// `w:titlePg`: первая страница несёт свой колонтитул, вторая — `default`.
///
/// Проверка негативная в обе стороны: на первой странице не должно быть текста
/// `default`, на второй — текста `first`. Иначе обе страницы могли бы получить
/// один и тот же колонтитул, и совпадение `First page header` прошло бы молча.
#[test]
fn title_page_uses_its_own_header() {
    let name = "headers_footers/title_pg_first.docx";
    let mut doc = document(name);
    assert!(
        doc.body.sections[0].title_pg,
        "{name}: фикстура несёт `w:titlePg`"
    );
    stretch(&mut doc, 150);
    let layout = layout(&doc);
    assert!(
        layout.pages.len() >= 2,
        "{name}: нужны первая и вторая страницы, получено {}",
        layout.pages.len()
    );

    let bytes = export(&doc);
    let first = page_text(&bytes, 0);
    assert!(
        first.contains("First page header"),
        "{name}: первая страница без колонтитула `first`: {first:?}"
    );
    assert!(
        !first.contains("Default header"),
        "{name}: на первой странице колонтитул `default`: {first:?}"
    );
    let second = page_text(&bytes, 1);
    assert!(
        second.contains("Default header"),
        "{name}: вторая страница без колонтитула `default`: {second:?}"
    );
    assert!(
        !second.contains("First page header"),
        "{name}: на второй странице колонтитул `first`: {second:?}"
    );
}

/// Содержимое таблицы в колонтитуле доходит до PDF.
///
/// Раскладка колонтитула не знает, что блок попал в таблицу, поэтому PDF обязан
/// звать для колонтитула тот же диспетчер: текст живёт в ячейке, и без ветки
/// `LayoutItem::Table` его в потоке не было бы вовсе. Сам тест сначала
/// убеждается, что раскладка положила в колонтитул именно таблицу, — иначе он
/// проверял бы путь абзацев.
#[test]
fn table_in_header_reaches_the_pdf() {
    let name = "headers_footers/default_header_footer.docx";
    let mut doc = document(name);
    let header = doc
        .headers
        .get_mut("word/header1.xml")
        .expect("часть верхнего колонтитула есть в документе");
    let items = std::mem::take(&mut header.body.items);
    assert!(
        !items.is_empty(),
        "{name}: в колонтитуле есть что заворачивать в таблицу"
    );
    header.body.items.push(BlockItem::Table(a_table(items)));

    let layout = layout(&doc);
    let header_items = &layout.chrome[0].header;
    assert!(
        header_items
            .iter()
            .any(|item| matches!(item, LayoutItem::Table { .. })),
        "{name}: раскладка положила таблицу в колонтитул: {header_items:?}"
    );
    assert!(
        !header_items
            .iter()
            .any(|item| matches!(item, LayoutItem::Paragraph { .. })),
        "{name}: абзацев в колонтитуле не осталось — текст только в ячейке"
    );

    let bytes = export(&doc);
    let text = page_text(&bytes, 0);
    assert!(
        text.contains("Default header"),
        "{name}: текст ячейки колонтитула не дошёл до PDF: {text:?}"
    );
}
