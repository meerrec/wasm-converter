//! Сквозные проверки: лист XLSX → PDF.
//!
//! Фикстуры — те же книги из `test-fixtures/xlsx`, что и у `crates/xlsx`
//! (собраны `scripts/gen-fixtures.ts`). Проверки структурные: файл парсится
//! `lopdf`, текст извлекается по `ToUnicode` — то же, что делает `pdftotext`
//! в приёмке спринта.
//!
//! Совпадение точек переноса с canvas-путём (DoD «Единая логика переноса»)
//! проверяется сравнением строк текста, а не пикселей: обе стороны рисуют
//! по `break_lines`.
//!
//! Фикстур с печатаемыми заголовками (`_xlnm.Print_Titles`) в наборе нет, а
//! счёт фикстур закреплён документацией: такая книга собирается в памяти
//! ([`package`]).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use doc_converter_pdf::{PageConfig, PdfExporter, PdfOptions};
use doc_converter_render::display_list::{DisplayList, DrawCommand};
use doc_converter_render::viewport::Viewport;
use doc_converter_xlsx::paint::PaintOptions;
use doc_converter_xlsx::CellRef;

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
/// Повторяет `pdftotext` из приёмки спринта: коды из `Tj` переводятся картой
/// `ToUnicode` обратно в символы. `lopdf::extract_text` на PDF своего бэкенда
/// работает, но свой разбор оставлен — он различает шрифты (см. `text_runs`).
fn decoded_text(bytes: &[u8]) -> String {
    let doc = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    let cmap = tounicode_map(&doc);
    let page = *doc.get_pages().values().next().expect("страница есть");
    let content = doc.get_page_content(page);
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

/// Сколько раз подстрока встречается в буфере.
fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Карта `ToUnicode` каждого шрифта страницы: имя ресурса → «код → символ».
fn page_cmaps(doc: &lopdf::Document, page: lopdf::ObjectId) -> HashMap<String, HashMap<u16, char>> {
    let mut cmaps = HashMap::new();
    for (name, font) in doc.get_page_fonts(page).expect("шрифты страницы") {
        let Ok(to_unicode) = font.get(b"ToUnicode") else {
            continue;
        };
        let Ok(id) = to_unicode.as_reference() else {
            continue;
        };
        let Ok(stream) = doc.get_object(id).and_then(lopdf::Object::as_stream) else {
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

/// Прогон текста в потоке содержимого: имя ресурса шрифта и строка.
fn text_runs(doc: &lopdf::Document, page: lopdf::ObjectId) -> Vec<(String, String)> {
    let cmaps = page_cmaps(doc, page);
    let content = doc.get_page_content(page);
    let text = String::from_utf8_lossy(&content);

    // Операторы `Tf`/`Tj` идут в потоке по порядку; в hex-строке ни `T`, ни
    // `j` встретиться не могут (это не hex-цифры), поэтому слияние позиций
    // токенов даёт настоящую последовательность прогонов.
    let mut events: Vec<(usize, &str)> = Vec::new();
    events.extend(text.match_indices("Tf"));
    events.extend(text.match_indices("Tj"));
    events.sort_by_key(|(at, _)| *at);

    let mut runs = Vec::new();
    let mut font = String::new();
    for (at, token) in events {
        if token == "Tf" {
            // Перед оператором стоит `/F2 11` — имя ресурса и кегль.
            if let Some(slash) = text[..at].rfind('/') {
                font = text[slash + 1..at]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
            }
            continue;
        }
        // Перед `Tj` стоит hex-строка `<…>`.
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

/// Байты встроенного `FontFile2` каждого шрифта страницы: ресурс → программа.
fn embedded_fonts(doc: &lopdf::Document, page: lopdf::ObjectId) -> HashMap<String, Vec<u8>> {
    let mut fonts = HashMap::new();
    for (name, font) in doc.get_page_fonts(page).expect("шрифты страницы") {
        let Some(bytes) = font_program(doc, font) else {
            continue;
        };
        fonts.insert(String::from_utf8_lossy(&name).into_owned(), bytes);
    }
    fonts
}

/// Программа шрифта: `Type0` → `DescendantFonts[0]` → `FontDescriptor` → `FontFile2`.
fn font_program(doc: &lopdf::Document, font: &lopdf::Dictionary) -> Option<Vec<u8>> {
    let descriptor = if let Ok(descendants) = font.get(b"DescendantFonts") {
        let first = descendants.as_array().ok()?.first()?;
        // Свой бэкенд кладёт потомка ссылкой; встроенный словарь тоже
        // разыменуем — тест не должен зависеть от формы записи.
        let descendant = match first {
            lopdf::Object::Reference(id) => doc.get_object(*id).ok()?,
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
    let descriptor = doc.get_object(descriptor).ok()?.as_dict().ok()?;
    let file = descriptor.get(b"FontFile2").ok()?.as_reference().ok()?;
    // `get_plain_content`, а не прямое чтение поля: он сам распаковывает
    // `FlateDecode` (`compress: true`) и отдаёт байты программы в любом случае.
    doc.get_object(file)
        .ok()?
        .as_stream()
        .ok()?
        .get_plain_content()
        .ok()
}

/// `macStyle` из таблицы `head` встроенного подмножества.
///
/// Бит 0 — полужирный, бит 1 — курсив: так начертание видно в самом шрифте,
/// а не только в имени ресурса.
fn mac_style(program: &[u8]) -> u16 {
    let num_tables = u16::from_be_bytes(program[4..6].try_into().expect("заголовок sfnt"));
    for index in 0..usize::from(num_tables) {
        let record = 12 + index * 16;
        if &program[record..record + 4] != b"head" {
            continue;
        }
        let offset = u32::from_be_bytes(
            program[record + 8..record + 12]
                .try_into()
                .expect("смещение таблицы"),
        ) as usize;
        return u16::from_be_bytes(
            program[offset + 44..offset + 46]
                .try_into()
                .expect("macStyle"),
        );
    }
    panic!("во встроенном шрифте нет таблицы head");
}

/// Прогон шрифта, которым набрана строка `needle`.
fn font_of_run(runs: &[(String, String)], needle: &str) -> String {
    runs.iter()
        .find(|(_, text)| text == needle)
        .unwrap_or_else(|| panic!("в потоке нет прогона {needle:?}: {runs:?}"))
        .0
        .clone()
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
    let content = doc.get_page_content(page);
    let content = String::from_utf8_lossy(&content);
    assert!(content.contains("BT"), "в потоке нет текстовых секций");
    assert!(
        content.contains("Tj") || content.contains("TJ"),
        "в потоке нет операторов вывода текста"
    );
}

#[test]
fn bold_and_italic_cells_use_their_own_faces() {
    // В фикстуре заняты все четыре начертания: Bold Italic, обычное, Bold,
    // Italic — значит, в PDF должны лежать четыре разных FontFile2.
    let bytes = export("styles-bold-italic.xlsx", 0);
    assert_eq!(
        count_occurrences(&bytes, b"FontFile2"),
        4,
        "встроены не все использованные начертания"
    );

    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let page = *doc.get_pages().values().next().expect("страница есть");
    let fonts = embedded_fonts(&doc, page);
    let runs = text_runs(&doc, page);

    // Строки фикстуры: 0 — жирный курсив, 1 — обычная, 2 — жирная,
    // 3 — курсив, 4 — жирная.
    let regular = font_of_run(&runs, "строка 1");
    let bold = font_of_run(&runs, "строка 2");
    let italic = font_of_run(&runs, "строка 3");
    let bold_italic = font_of_run(&runs, "строка 0");

    assert_ne!(bold, regular, "жирная ячейка набрана тем же шрифтом");
    assert_ne!(italic, regular, "курсивная ячейка набрана тем же шрифтом");
    assert_ne!(bold_italic, regular, "жирный курсив — тем же шрифтом");

    let style = |name: &String| mac_style(fonts.get(name).expect("шрифт встроен"));
    assert_eq!(
        style(&regular) & 0b11,
        0,
        "обычная ячейка набрана не regular"
    );
    assert_eq!(style(&bold) & 0b11, 0b01, "жирная ячейка набрана не Bold");
    assert_eq!(style(&italic) & 0b11, 0b10, "курсивная набрана не Italic");
    assert_eq!(
        style(&bold_italic) & 0b11,
        0b11,
        "жирный курсив набран не Bold Italic"
    );
}

#[test]
fn unused_faces_are_not_embedded() {
    // В книге нет выделенного текста: лишние подмножества — лишние ~90 КБ.
    let bytes = export("content-single-cell.xlsx", 0);

    assert_eq!(
        count_occurrences(&bytes, b"FontFile2"),
        1,
        "в PDF попало неиспользованное начертание"
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let page = *doc.get_pages().values().next().expect("страница есть");
    let fonts = embedded_fonts(&doc, page);
    let program = fonts.values().next().expect("шрифт встроен");

    assert_eq!(mac_style(program) & 0b11, 0, "встроено не regular");
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
fn cell_edit_reaches_the_page() {
    // Правка интерфейса перекрывает значение: PDF печатает сырой текст, как
    // его ввёл пользователь, — тем же путём, каким его видит кадр.
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture("values-strings.xlsx")).expect("фикстура читается"),
    )
    .expect("книга открывается");
    book.set_cell_text(0, CellRef::new(0, 0), "правка ячейки A1".to_owned());

    let bytes = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("PDF собирается");
    let text = decoded_text(&bytes);

    assert!(
        text.contains("правка ячейки A1"),
        "правка потерялась в PDF: {text:?}"
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
        let content = doc.get_page_content(page);
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
    let message = result
        .expect_err("несуществующий лист — ошибка")
        .to_string();

    // Сбой экспорта — не порча OOXML: у ядра для него свой вариант (ADR-0008).
    assert!(
        message.starts_with("export failed: pdf:"),
        "неверная семантика ошибки: {message}"
    );
    assert!(
        !message.contains("malformed OOXML"),
        "ошибка экспорта выдана за порчу книги: {message}"
    );
}

/// Строки текста, которые рисует canvas-путь, в порядке кадра.
fn canvas_lines(book: &doc_converter_xlsx::Workbook) -> Vec<String> {
    let mut display_list = DisplayList::new();
    let viewport = Viewport {
        x: 0.0,
        y: 0.0,
        // Окно заведомо вмещает лист целиком: обрезка по краю окна — не то,
        // что здесь проверяется.
        w: 10_000.0,
        h: 10_000.0,
        scale: 1.0,
    };
    let options = PaintOptions {
        show_grid: false,
        show_headers: false,
        ..PaintOptions::default()
    };
    doc_converter_xlsx::paint::build(
        book,
        &book.sheets()[0],
        viewport,
        &options,
        &mut display_list,
        &std::cell::Cell::new(false),
    )
    .expect("кадр собирается");

    (0..display_list.len())
        .filter_map(|index| display_list.cmd(index))
        .filter_map(|cmd| match cmd {
            DrawCommand::Text { text, .. } => Some(display_list.string(*text).to_owned()),
            _ => None,
        })
        .collect()
}

/// DoD «Единая логика переноса»: строки текста в PDF и на canvas совпадают.
///
/// Сравниваются именно точки разрыва — строки по каждой ячейке в порядке
/// обхода, а не пиксели. Тест падает, если PDF-путь сменит кегль, ширину
/// переноса или перестанет звать `break_lines`.
#[test]
fn pdf_wraps_text_exactly_like_canvas() {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture("text-cyrillic-wrap.xlsx")).expect("фикстура читается"),
    )
    .expect("книга открывается");

    let canvas = canvas_lines(&book);
    let bytes = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("лист экспортируется");
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let page = *doc.get_pages().values().next().expect("страница есть");
    let pdf: Vec<String> = text_runs(&doc, page)
        .into_iter()
        .map(|(_, text)| text)
        .collect();

    assert_eq!(canvas, pdf, "точки переноса PDF разошлись с canvas-путём");

    // Проверка не вырождена: в фикстуре есть и перенос, и «#####».
    assert!(
        canvas.len() > 10,
        "фикстура перестала переносить текст: {canvas:?}"
    );
    assert!(
        canvas.iter().any(|line| line == "#####"),
        "в фикстуре нет невоместившегося числа: {canvas:?}"
    );
    assert!(
        canvas.iter().any(|line| line.contains("гидро")),
        "в фикстуре нет длинного слова: {canvas:?}"
    );
}

/// Ячейка собираемой книги: строка (0-based), столбец буквой A1-нотации, текст.
type TestCell = (u32, &'static str, String);

/// Лист собираемой книги: имя и ячейки.
type TestSheet = (&'static str, Vec<TestCell>);

/// Собрать книгу XLSX в памяти: листы и `definedNames` книги.
///
/// Минимум частей, как в `crates/xlsx/tests/print_settings.rs`: текст ячеек
/// записан прямо в листе (`inlineStr`), общей таблицы строк и стилей нет.
fn package(sheets: &[TestSheet], defined_names: &str) -> Vec<u8> {
    let worksheet = |cells: &[TestCell]| {
        let mut rows: Vec<u32> = cells.iter().map(|(row, _, _)| *row).collect();
        rows.sort_unstable();
        rows.dedup();
        let mut data = String::new();
        for row in rows {
            data.push_str(&format!("<row r=\"{}\">", row + 1));
            for (_, column, text) in cells.iter().filter(|(at, _, _)| *at == row) {
                data.push_str(&format!(
                    "<c r=\"{column}{}\" t=\"inlineStr\"><is><t>{text}</t></is></c>",
                    row + 1
                ));
            }
            data.push_str("</row>");
        }
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<sheetData>{data}</sheetData></worksheet>"
        )
    };

    let sheet_entries: String = sheets
        .iter()
        .enumerate()
        .map(|(index, (name, _))| {
            format!(
                "<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
                index + 1,
                index + 1
            )
        })
        .collect();
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>{sheet_entries}</sheets>
  {defined_names}
</workbook>"#
    );
    let workbook_rels: String = sheets
        .iter()
        .enumerate()
        .map(|(index, _)| {
            format!(
                "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>",
                index + 1,
                index + 1
            )
        })
        .collect();
    let workbook_rels = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
{workbook_rels}</Relationships>"
    );
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
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        let mut parts: Vec<(String, String)> = vec![
            ("[Content_Types].xml".to_owned(), content_types.to_owned()),
            ("_rels/.rels".to_owned(), root_rels.to_owned()),
            ("xl/workbook.xml".to_owned(), workbook),
            ("xl/_rels/workbook.xml.rels".to_owned(), workbook_rels),
        ];
        for (index, (_, cells)) in sheets.iter().enumerate() {
            parts.push((
                format!("xl/worksheets/sheet{}.xml", index + 1),
                worksheet(cells),
            ));
        }
        for (name, body) in parts {
            zip.start_file(name, options).expect("часть в архив");
            zip.write_all(body.as_bytes()).expect("часть записана");
        }
        zip.finish().expect("пакет собран");
    }
    buf
}

/// Ячейки одного столбца: шапка в A1 и текст «{prefix} N» в строках под ней.
///
/// Строк заведомо больше страницы (48 строк по 20 px на A4): без разбивки на
/// страницы повторять шапку было бы негде.
fn sheet_cells(header: &str, prefix: &str, rows: u32) -> Vec<TestCell> {
    let mut cells = vec![(0, "A", header.to_owned())];
    for row in 1..=rows {
        cells.push((row, "A", format!("{prefix} {row}")));
    }
    cells
}

/// Тексты прогонов страницы, в порядке потока: по одному на строку набора.
fn page_lines(doc: &lopdf::Document, page: lopdf::ObjectId) -> Vec<String> {
    text_runs(doc, page)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Есть ли на странице прогон ровно с таким текстом.
fn has_line(lines: &[String], text: &str) -> bool {
    lines.iter().any(|line| line == text)
}

/// Тексты страниц документа в порядке номеров.
fn document_lines(bytes: &[u8]) -> Vec<Vec<String>> {
    let doc = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    doc.get_pages()
        .into_values()
        .map(|page| page_lines(&doc, page))
        .collect()
}

/// Печатаемые заголовки книги (`_xlnm.Print_Titles`) повторяют шапку на
/// каждой странице своего листа.
///
/// До этой связки `sheet.print` до PDF не доезжал, и шапка печаталась один
/// раз — на первой странице. У второго листа заголовков нет: его шапка
/// повторяться не должна — у каждого листа свои `Print_Titles`, а конфиг у
/// них общий.
#[test]
fn print_titles_repeat_rows_on_every_page() {
    let book = doc_converter_xlsx::open(package(
        &[
            ("Первый", sheet_cells("Шапка первого", "первая", 120)),
            ("Второй", sheet_cells("Шапка второго", "вторая", 120)),
        ],
        r#"<definedNames>
             <definedName name="_xlnm.Print_Titles">'Первый'!$1:$1</definedName>
           </definedNames>"#,
    ))
    .expect("книга открывается");

    let mut bytes = Vec::new();
    PdfExporter::new(PdfOptions::default())
        .export_xlsx_book_to(&book, &mut bytes)
        .expect("книга экспортируется");
    let pages = document_lines(&bytes);

    // Страницы первого листа — те, где есть его данные: тексты листов разные.
    let first: Vec<&Vec<String>> = pages
        .iter()
        .filter(|lines| lines.iter().any(|line| line.starts_with("первая ")))
        .collect();
    assert!(
        first.len() >= 2,
        "лист не разбит на страницы: страниц {}",
        first.len()
    );
    for lines in &first {
        assert!(
            has_line(lines, "Шапка первого"),
            "шапка пропала со страницы: {lines:?}"
        );
    }
    // Содержимое не потерялось: последняя страница листа — про последнюю строку.
    assert!(has_line(first.last().expect("страницы есть"), "первая 120"));

    let repeats = pages
        .iter()
        .filter(|lines| has_line(lines, "Шапка второго"))
        .count();
    assert_eq!(repeats, 1, "лист без Print_Titles повторил шапку");
}

/// Повторяемые столбцы книги (`$A:$A` в `_xlnm.Print_Titles`) печатаются на
/// каждой полосе столбцов, а не только на первой.
#[test]
fn print_titles_repeat_first_columns_on_every_band() {
    let mut cells = vec![
        (0, "A", "Шапка первого".to_owned()),
        (0, "K", "Шапка права".to_owned()),
    ];
    for row in 1..=120 {
        cells.push((row, "A", format!("метка {row}")));
        cells.push((row, "K", format!("право {row}")));
    }
    let book = doc_converter_xlsx::open(package(
        &[("Первый", cells)],
        r#"<definedNames>
             <definedName name="_xlnm.Print_Titles">'Первый'!$A:$A</definedName>
           </definedNames>"#,
    ))
    .expect("книга открывается");

    let bytes = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(&book, 0)
        .expect("лист экспортируется");
    let pages = document_lines(&bytes);
    assert!(pages.len() >= 4, "лист не разбит по столбцам и строкам");

    // Правая полоса столбцов: столбец K за границей страницы.
    let right: Vec<&Vec<String>> = pages
        .iter()
        .filter(|lines| lines.iter().any(|line| line.starts_with("право ")))
        .collect();
    assert!(!right.is_empty(), "лист не разбит по столбцам");
    let last = right
        .iter()
        .find(|lines| has_line(lines, "право 120"))
        .expect("страница последней строки");
    assert!(
        has_line(last, "метка 120"),
        "повторяемый столбец потерялся на полосе: {last:?}"
    );
    // Первая полоса строк: её правая полоса столбцов несёт и ячейку шапки из
    // повторяемого столбца.
    let first = right
        .iter()
        .find(|lines| has_line(lines, "право 1"))
        .expect("страница первой строки");
    assert!(
        has_line(first, "Шапка первого"),
        "шапка повторяемого столбца потерялась: {first:?}"
    );
    assert!(
        has_line(first, "метка 1"),
        "повторяемый столбец потерялся на полосе: {first:?}"
    );
}

/// Явный `PageConfig::repeat_header_rows` сильнее книги: вызывающий печатает
/// не «как Excel», а как ему сказали.
#[test]
fn explicit_page_config_beats_print_titles() {
    let book = doc_converter_xlsx::open(package(
        &[("Первый", sheet_cells("Шапка первого", "первая", 120))],
        r#"<definedNames>
             <definedName name="_xlnm.Print_Titles">'Первый'!$1:$1</definedName>
           </definedNames>"#,
    ))
    .expect("книга открывается");

    let bytes = PdfExporter::new(PdfOptions {
        page: PageConfig {
            repeat_header_rows: 2,
            ..PageConfig::default()
        },
        ..PdfOptions::default()
    })
    .export_xlsx_sheet(&book, 0)
    .expect("лист экспортируется");
    let pages = document_lines(&bytes);
    assert!(pages.len() >= 2, "лист не разбит на страницы");
    for lines in &pages {
        assert!(
            has_line(lines, "Шапка первого"),
            "шапка пропала со страницы: {lines:?}"
        );
        assert!(
            has_line(lines, "первая 1"),
            "явные две строки не победили книгу: {lines:?}"
        );
    }
}
