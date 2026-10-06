//! Регрессия пагинации: точные числа страниц и эталон отрисовки.
//!
//! `export.rs` проверяет структуру PDF (страниц не меньше десяти и т.п.),
//! здесь — точные значения: выделение `paginate` в свой модуль (задача A1) не
//! должно было изменить вывод. Эталон — хеш отрисовки ([`drawing_hash`]), а не
//! байтов всего файла: printpdf берёт имена шрифтов и `/ID` документа из
//! глобального счётчика случайности (`printpdf::utils::RAND_SEED`), поэтому
//! каждый следующий экспорт в процессе даёт другие байты — побайтовый эталон
//! был бы нестабилен. Содержимое страниц от счётчика не зависит.
//!
//! Эталоны обновляются осознанно: фактические значения из сообщения упавшего
//! `assert`'а переносятся в [`PAGE_COUNTS`] и [`DRAWING`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use doc_converter_pdf::{PageConfig, PdfExporter, PdfOptions};
use lopdf::{Object, ObjectId};

/// Начальное значение FNV-1a.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Открыть книгу из общего набора фикстур.
fn open(name: &str) -> doc_converter_xlsx::Workbook {
    let path = fixture(name);
    let bytes = fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    doc_converter_xlsx::open(bytes).unwrap_or_else(|err| panic!("{name}: {err}"))
}

/// Экспортировать лист 0 книги настройками по умолчанию.
fn export(name: &str) -> Vec<u8> {
    export_with(name, PageConfig::default())
}

/// Экспортировать лист 0 книги с заданными настройками страницы.
fn export_with(name: &str, page: PageConfig) -> Vec<u8> {
    let book = open(name);
    PdfExporter::new(PdfOptions {
        page,
        ..PdfOptions::default()
    })
    .export_xlsx_sheet(&book, 0)
    .unwrap_or_else(|err| panic!("{name} не экспортировалась: {err}"))
}

/// Есть ли утилита в `PATH`: как в `external.rs` — спавн и есть проверка.
fn tool_available(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

/// Хеш FNV-1a: регрессии хватает и 64 бит, отдельная зависимость не нужна.
fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// Заменить случайные имена шрифтов заглушкой.
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

/// Хеш отрисовки: словари страниц и потоки операций по порядку страниц.
///
/// Ловит любую правку пагинации и раскладки, не завися от случайных имён
/// шрифтов и `/ID`.
fn drawing_hash(bytes: &[u8]) -> u64 {
    let doc = lopdf::Document::load_mem(bytes).expect("PDF разбирается lopdf");
    let mut hash = FNV_OFFSET;
    for (number, page_id) in doc.get_pages() {
        hash = fnv1a(format!("page {number}").as_bytes(), hash);
        let page = doc.get_dictionary(page_id).expect("словарь страницы");
        hash = fnv1a(format!("{page:?}").as_bytes(), hash);
        let content = doc.get_page_content(page_id).expect("поток содержимого");
        hash = fnv1a(&mask_random_font_names(&content), hash);
    }
    hash
}

/// Сколько страниц обязана давать книга при настройках по умолчанию.
///
/// Числа сняты с вывода: 600 строк отчёта — 13 страниц A4, 5000 строк — 105.
/// Широкие листы после разбивки по столбцам (A2) дают произведение полос
/// строк и столбцов: 100×20 — 2 полосы столбцов по 10 и 3 полосы строк по 48 —
/// 6 страниц; 20×100 — 10 полос по 10 столбцов и одна полоса строк — 10.
const PAGE_COUNTS: &[(&str, usize)] = &[
    ("scale-ten-pages.xlsx", 13),
    ("edge-many-rows.xlsx", 105),
    ("content-dense.xlsx", 6),
    ("size-20x100x1.xlsx", 10),
    ("content-empty-sheet.xlsx", 1),
];

/// Сколько текстовых секций (`BT`) в потоке содержимого страницы.
///
/// Каждая нарисованная ячейка с непустым текстом пишет свою секцию (по одной
/// на строку, `text::draw_cell_text`), других `BT` в потоке нет — счётчик и
/// есть число ячеек, дошедших до страницы.
fn text_sections(content: &[u8]) -> usize {
    content
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|token| *token == b"BT")
        .count()
}

/// Размах левых координат текста в потоке содержимого страницы, точки.
///
/// `text::draw_cell_text` ставит курсор оператором `Td`, других `Td` в потоке
/// нет, поэтому размах и есть полоса, занятая содержимым страницы.
fn cursor_x_range(content: &[u8]) -> (f32, f32) {
    let tokens: Vec<&[u8]> = content
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|token| !token.is_empty())
        .collect();
    let mut range = (f32::INFINITY, f32::NEG_INFINITY);
    for (index, token) in tokens.iter().enumerate() {
        if *token != b"Td" || index < 2 {
            continue;
        }
        let Ok(x) = std::str::from_utf8(tokens[index - 2])
            .unwrap_or_default()
            .parse::<f32>()
        else {
            continue;
        };
        range = (range.0.min(x), range.1.max(x));
    }
    range
}

/// DoD A2: при разбивке по столбцам ни одна ячейка листа не теряется.
///
/// Лист 30×30 даёт три полосы столбцов по десять; все страницы непустые, а
/// сумма нарисованных ячеек равна ячейкам листа. До разбивки ячейки правее
/// области содержимого молча отбрасывались (`painter.rs:208-211`).
#[test]
fn all_cells_survive_column_split() {
    const FIXTURE: &str = "size-30x30x2.xlsx";
    let book = open(FIXTURE);
    let sheet = &book.sheets()[0];
    let cells: usize = sheet.cells.rows().map(|(_, row)| row.len()).sum();
    assert_eq!(
        cells, 900,
        "{FIXTURE}: фикстура изменилась — сверьте ожидания"
    );

    let bytes = export(FIXTURE);
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 3, "30 столбцов — три полосы по десять");

    let mut drawn = 0;
    let mut ranges: Vec<(f32, f32)> = Vec::new();
    for (number, page_id) in pages {
        let content = doc.get_page_content(page_id).expect("поток содержимого");
        let on_page = text_sections(&content);
        assert!(on_page > 0, "страница {number} пуста");
        drawn += on_page;
        ranges.push(cursor_x_range(&content));
    }
    assert_eq!(drawn, cells, "часть ячеек листа не дошла до страниц");

    // Каждая полоса начинается от левого края области содержимого, поэтому
    // размах курсоров у всех страниц один и тот же: сдвиг полосы (offset_x)
    // вычтен, а не потерян или вычтен дважды.
    let (left, right) = ranges[0];
    for (index, (page_left, page_right)) in ranges.iter().enumerate() {
        assert!(
            (page_left - left).abs() < 0.01 && (page_right - right).abs() < 0.01,
            "страница {index} нарисована со сдвигом: {page_left}..{page_right} против {left}..{right}"
        );
    }
}

#[test]
fn page_counts_are_pinned() {
    for &(name, expected) in PAGE_COUNTS {
        let bytes = export(name);
        let doc = lopdf::Document::load_mem(&bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(
            doc.get_pages().len(),
            expected,
            "{name}: число страниц изменилось"
        );
    }
}

/// Фикстура → хеш отрисовки при настройках по умолчанию.
///
/// Эталоны узких листов не менялись при разбивке по столбцам (A2): у них одна
/// полоса. Пересчитаны те, у кого ширина листа переросла страницу:
/// `content-dense` и `size-20x100x1` (правые ячейки раньше отбрасывались за
/// `max_x`) и `layout-column-widths` (пятый столбец шириной 60 символов уехал
/// на вторую полосу).
const DRAWING: &[(&str, u64)] = &[
    ("scale-ten-pages.xlsx", 0xc427_d500_5031_3cb4),
    ("edge-many-rows.xlsx", 0x8451_e5a1_99cc_39e7),
    ("content-dense.xlsx", 0x2ec6_1b78_f99d_8b35),
    ("size-20x100x1.xlsx", 0xd2eb_fa1b_4c50_b1a7),
    ("values-strings.xlsx", 0xc9ad_db68_a27a_67f7),
    ("layout-column-widths.xlsx", 0xc279_e1d4_8bc9_ace5),
    ("edge-merged-styled.xlsx", 0xf1fa_6ec1_b8db_84b6),
    ("content-empty-sheet.xlsx", 0x615d_6ab3_aa58_b40f),
];

#[test]
fn golden_drawing_is_unchanged() {
    for &(name, expected) in DRAWING {
        let bytes = export(name);
        assert_eq!(
            drawing_hash(&bytes),
            expected,
            "{name}: отрисовка разошлась с эталоном"
        );
    }
}

/// Число из токена `<XXXX>`.
fn hex_token(token: &str) -> Result<u32, std::num::ParseIntError> {
    u32::from_str_radix(token.trim_matches(['<', '>']), 16)
}

/// Таблица «код в потоке → символ» из ToUnicode-потока шрифта.
///
/// printpdf пишет расшифровку блоками `beginbfchar`; lopdf 0.35 этот CMap
/// разобрать не может («failed parsing ToUnicode CMap»), поэтому тест разбирает
/// его сам — формат простой.
fn cmap_mappings(cmap: &str) -> HashMap<u16, char> {
    let mut mappings = HashMap::new();
    let mut inside = false;
    for line in cmap.lines().map(str::trim) {
        if line.ends_with("beginbfchar") {
            inside = true;
        } else if line == "endbfchar" {
            inside = false;
        } else if inside {
            let mut tokens = line.split_whitespace();
            let (Some(code), Some(unicode)) = (tokens.next(), tokens.next()) else {
                continue;
            };
            let (Ok(code), Ok(unicode)) = (hex_token(code), hex_token(unicode)) else {
                continue;
            };
            if let (Ok(code), Some(ch)) = (u16::try_from(code), char::from_u32(unicode)) {
                mappings.insert(code, ch);
            }
        }
    }
    mappings
}

/// Шрифты страницы: имя ресурса → таблица «код → символ».
fn page_cmaps(doc: &lopdf::Document, page_id: ObjectId) -> HashMap<Vec<u8>, HashMap<u16, char>> {
    let mut cmaps = HashMap::new();
    let Ok(page) = doc.get_dictionary(page_id) else {
        return cmaps;
    };
    let Ok(resources) = page.get(b"Resources") else {
        return cmaps;
    };
    let Ok((_, resources)) = doc.dereference(resources) else {
        return cmaps;
    };
    let Ok(resources) = resources.as_dict() else {
        return cmaps;
    };
    let Ok(fonts) = resources.get(b"Font") else {
        return cmaps;
    };
    let Ok((_, fonts)) = doc.dereference(fonts) else {
        return cmaps;
    };
    let Ok(fonts) = fonts.as_dict() else {
        return cmaps;
    };
    for (name, font) in fonts.iter() {
        let Ok((_, font)) = doc.dereference(font) else {
            continue;
        };
        let Ok(font) = font.as_dict() else {
            continue;
        };
        let Ok(to_unicode) = font.get(b"ToUnicode") else {
            continue;
        };
        let Ok((_, stream)) = doc.dereference(to_unicode) else {
            continue;
        };
        let Ok(stream) = stream.as_stream() else {
            continue;
        };
        let Ok(bytes) = stream.decompressed_content() else {
            continue;
        };
        cmaps.insert(
            name.clone(),
            cmap_mappings(&String::from_utf8_lossy(&bytes)),
        );
    }
    cmaps
}

/// Дописать символы строки операции в `text`.
///
/// Встроенные шрифты записаны как Identity-H: код — два байта, значение — глиф
/// оригинального шрифта, расшифровка — по его ToUnicode.
fn push_decoded(
    text: &mut String,
    cmaps: &HashMap<Vec<u8>, HashMap<u16, char>>,
    font: Option<&[u8]>,
    object: &Object,
) {
    let Ok(bytes) = object.as_str() else {
        return;
    };
    let Some(map) = font.and_then(|name| cmaps.get(name)) else {
        return;
    };
    let (codes, _) = bytes.as_chunks::<2>();
    for code in codes {
        if let Some(ch) = map.get(&u16::from_be_bytes(*code)) {
            text.push(*ch);
        }
    }
}

/// Текст страницы: обход операций записи (`Tf` выбирает шрифт, `Tj`/`TJ` пишут).
fn page_text(doc: &lopdf::Document, page_id: ObjectId) -> String {
    let cmaps = page_cmaps(doc, page_id);
    let content = doc.get_page_content(page_id).expect("поток содержимого");
    let content = lopdf::content::Content::decode(&content).expect("операции разбираются");
    let mut text = String::new();
    let mut font: Option<&[u8]> = None;
    for op in &content.operations {
        match op.operator.as_str() {
            "Tf" => font = op.operands.first().and_then(|name| name.as_name().ok()),
            "Tj" => {
                if let Some(object) = op.operands.first() {
                    push_decoded(&mut text, &cmaps, font, object);
                }
            }
            "TJ" => {
                if let Some(Object::Array(items)) = op.operands.first() {
                    for item in items {
                        push_decoded(&mut text, &cmaps, font, item);
                    }
                }
            }
            _ => {}
        }
    }
    text
}

/// DoD A3: текст шапки присутствует на каждой странице ровно один раз.
///
/// Основная проверка разбирает поток содержимого сама и потому работает без
/// внешних утилит. Там, где есть `pdftotext` (в CI он ставится шагом «pdf
/// tooling»), те же страницы проверяются `-f N -l N`; локально без утилиты этот
/// шаг пропускается.
#[test]
fn repeat_header_rows_on_every_page() {
    const FIXTURE: &str = "scale-ten-pages.xlsx";
    // Шапка отчёта — «№ Наименование Артикул …»; берём слово, которого нет в
    // строках данных («Позиция N: …»).
    const HEADER_WORD: &str = "Наименование";
    let bytes = export_with(
        FIXTURE,
        PageConfig {
            repeat_header_rows: 1,
            ..PageConfig::default()
        },
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 13, "шапка не должна менять число страниц");

    for (number, page_id) in &pages {
        let text = page_text(&doc, *page_id);
        assert_eq!(
            text.matches(HEADER_WORD).count(),
            1,
            "страница {number}: шапка не ровно один раз в тексте {text:?}"
        );
    }

    // Без настройки шапка не повторяется: на второй странице её текста нет.
    let plain = export(FIXTURE);
    let plain_doc = lopdf::Document::load_mem(&plain).expect("PDF разбирается lopdf");
    let plain_pages = plain_doc.get_pages();
    let text = page_text(&plain_doc, plain_pages[&2]);
    assert!(
        !text.contains(HEADER_WORD),
        "шапка продублирована без настройки: {text:?}"
    );

    if !tool_available("pdftotext") {
        eprintln!("пропуск pdftotext: не найден в PATH (в CI ставится шагом «pdf tooling»)");
        return;
    }
    let path =
        std::env::temp_dir().join(format!("doc-converter-header-{}.pdf", std::process::id()));
    fs::write(&path, &bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    for number in pages.keys() {
        let page = number.to_string();
        let output = Command::new("pdftotext")
            .args(["-f", &page, "-l", &page])
            .arg(&path)
            .arg("-")
            .output()
            .expect("pdftotext запускается");
        assert!(
            output.status.success(),
            "pdftotext вернул {} для страницы {number}",
            output.status
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains(HEADER_WORD),
            "pdftotext, страница {number}: шапки нет в {text:?}"
        );
    }
    fs::remove_file(&path).expect("временный PDF удаляется");
}

/// A3: первые столбцы повторяются на каждой полосе столбцов.
#[test]
fn first_columns_repeat_on_every_page() {
    const FIXTURE: &str = "size-30x30x2.xlsx";
    let bytes = export_with(
        FIXTURE,
        PageConfig {
            repeat_first_columns: 1,
            ..PageConfig::default()
        },
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages();
    // Область содержимого A4 — 680,3 px, столбец фикстуры — 64 px. Повтор
    // отнимает 64 px, в полосу влезает 9 столбцов из 29 неповторяемых:
    // полосы 9 + 9 + 9 + 2, то есть четыре страницы.
    assert_eq!(pages.len(), 4, "повтор столбца не отнял место у полосы");

    // На каждой странице — все 30 ячеек повторяемого столбца плюс её полоса.
    let expected = [300, 300, 300, 90];
    for ((number, page_id), expected) in pages.iter().zip(expected) {
        let content = doc.get_page_content(*page_id).expect("поток содержимого");
        assert_eq!(
            text_sections(&content),
            expected,
            "страница {number}: первый столбец не повторён в полном составе"
        );
    }
}

/// Число нарисованных ячеек на каждой странице, по номеру страницы.
fn sections_per_page(doc: &lopdf::Document) -> Vec<usize> {
    doc.get_pages()
        .values()
        .map(|page_id| text_sections(&doc.get_page_content(*page_id).expect("поток содержимого")))
        .collect()
}

/// A4: `fit_to_height` укладывает лист в заданное число страниц.
#[test]
fn fit_to_height_limits_page_count() {
    const FIXTURE: &str = "scale-ten-pages.xlsx";
    let plain = lopdf::Document::load_mem(&export(FIXTURE)).expect("PDF разбирается lopdf");
    assert_eq!(plain.get_pages().len(), 13, "эталон фикстуры изменился");

    let bytes = export_with(
        FIXTURE,
        PageConfig {
            fit_to_height: Some(10),
            ..PageConfig::default()
        },
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    assert_eq!(
        doc.get_pages().len(),
        10,
        "лист не уложился в десять страниц"
    );
    // Ужатие не теряет содержимое: ячейки на месте и на первой странице, и на
    // последней (подбор масштаба не отбрасывает остаток листа).
    let sections = sections_per_page(&doc);
    assert_eq!(sections.iter().sum::<usize>(), 601 * 6);
    assert!(sections.last().copied().unwrap_or(0) > 0);
}

/// A4: при `avoid_row_break = false` неполная строка видна на обеих страницах:
/// разрыв идёт по нижней границе области содержимого, а не по верху строки.
#[test]
fn avoid_row_break_false_repeats_split_row() {
    const FIXTURE: &str = "scale-ten-pages.xlsx";
    let plain = lopdf::Document::load_mem(&export(FIXTURE)).expect("PDF разбирается lopdf");
    let split = lopdf::Document::load_mem(&export_with(
        FIXTURE,
        PageConfig {
            avoid_row_break: false,
            ..PageConfig::default()
        },
    ))
    .expect("PDF разбирается lopdf");

    let plain_sections = sections_per_page(&plain);
    let split_sections = sections_per_page(&split);
    assert_eq!(plain_sections.len(), 13, "эталон фикстуры изменился");
    // Разорванная строка не создаёт лишней страницы: её место занимает та же
    // позиция листа, с которой она продолжается.
    assert_eq!(split_sections.len(), 13, "разрыв строки добавил страницу");

    // В строке фикстуры шесть ячеек; на первой странице 48 целых строк, а с
    // разрывом к ним добавляется сорок девятая — частично видимая.
    assert_eq!(plain_sections[0], 48 * 6);
    assert_eq!(split_sections[0], 49 * 6);
    // На второй странице разорванная строка повторяется: 48 новых плюс одна
    // повторённая.
    assert_eq!(split_sections[1], (48 + 2) * 6);
    // Ни одна ячейка не потеряна: разорванных строк на 12 больше (по одной на
    // каждой странице, кроме последней).
    assert_eq!(plain_sections.iter().sum::<usize>(), 601 * 6);
    assert_eq!(split_sections.iter().sum::<usize>(), (601 + 12) * 6);
}
