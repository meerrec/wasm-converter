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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::display_list::{DisplayList, DrawCommand};
use doc_converter_render::viewport::Viewport;
use doc_converter_xlsx::paint::PaintOptions;

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
    let content = doc.get_page_content(page).expect("поток содержимого");
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
        // printpdf кладёт потомка встроенным словарём, но ссылку тоже надо
        // уметь разыменовать.
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
    // `get_plain_content`, а не `decompressed_content`: подмножество шрифта
    // printpdf пишет без фильтра.
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
    let content = doc.get_page_content(page).expect("поток содержимого");
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
    );

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
