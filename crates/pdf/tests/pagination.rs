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
use doc_converter_xlsx::{
    Cell, CellRef, CellValue, PrintSettings, SharedStrings, Sheet, SheetContent, SheetState,
    StyleTable, Theme, Workbook, WorksheetBuilder, WorksheetMeta,
};
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
/// Узкие листы делит только высота (`content-empty-sheet` — одна страница).
///
/// Широкие листы печатаются целиком: умолчание разрешения масштаба — уместить
/// по ширине (`resolve_page_scale`, уровень 5), поэтому полос столбцов у них
/// больше нет, а страницы даёт деление по высоте: `content-dense` (100 строк по
/// 20 столбцов) ужимается вдвое и делится на 2 страницы, `size-20x100x1`
/// (20 строк по 100 столбцов) — одна страница.
const PAGE_COUNTS: &[(&str, usize)] = &[
    ("scale-ten-pages.xlsx", 13),
    ("edge-many-rows.xlsx", 105),
    ("content-dense.xlsx", 2),
    ("size-20x100x1.xlsx", 1),
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
///
/// Явный `fit_to_height` нужен, чтобы ширина осталась естественной: умолчание
/// умещает лист в одну страницу по ширине (`resolve_page_scale`, уровень 5), и
/// полос столбцов при нём не бывает вовсе.
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

    let bytes = export_with(
        FIXTURE,
        PageConfig {
            fit_to_height: Some(1),
            ..PageConfig::default()
        },
    );
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
///
/// Все эталоны, кроме `content-empty-sheet` (там нечего сдвигать), пересчитаны
/// вместе с выравниванием листа по эталонной печати: поля страницы берутся из
/// `<pageMargins>` листа, а вставка текста в ячейке — точка (см.
/// `xlsx::paint::TEXT_PADDING`). До этого лист печатался с полями приложения и
/// начинался на 14–19 px @150 dpi левее эталона.
///
/// `edge-merged-styled` пересчитан после починки заливок: полигон строился от
/// верхней кромки (`Rect::to_polygon` в вендоренной 0.8.2), и каждая заливка
/// уезжала вниз на высоту своего прямоугольника. Эталон был снят с багованной
/// отрисовки — потому баг и не ловился.
///
/// Умолчание «уместить по ширине» (`resolve_page_scale`, уровень 5) пересчитало
/// широкие листы ещё раз: у `content-dense`, `size-20x100x1` и
/// `layout-column-widths` теперь одна полоса столбцов, а масштаб ужат под
/// ширину страницы.
///
/// Объявленные ширины столбцов (`xlsx::layout::width_to_px`) считаются по
/// метрике цифры шрифта, как в `LibreOffice`, а не по формовым 7 px Excel:
/// `scale-ten-pages` (столбец в 40 символов) и `layout-column-widths`
/// (объявленные ширины) отрисованы заново. У остальных фикстур столбцы
/// умолчания — их ширина не изменилась.
///
/// Все эталоны пересчитаны с переходом XLSX-пути на свой бэкенд (ADR-0022):
/// хеш включает словарь страницы, а он пишется иначе (свои `/MediaBox`,
/// нет `TrimBox`/`CropBox`), и поток содержимого — имена шрифтов в нём
/// детерминированные («F0», …), а не 32-значные случайные printpdf. Это
/// раскладка сериализации, а не отрисовки: побайтового совпадения с форком
/// ADR-0022 не обещает, сами потоки операций совпадают (см. `pdf::content`).
const DRAWING: &[(&str, u64)] = &[
    ("scale-ten-pages.xlsx", 0x21c1_e394_ee55_a5f1),
    ("edge-many-rows.xlsx", 0xd69c_b79f_8589_54bf),
    ("content-dense.xlsx", 0xe693_7825_b016_6ca2),
    ("size-20x100x1.xlsx", 0xf2f1_3047_a6da_47d2),
    ("values-strings.xlsx", 0x8bcf_7777_de1e_6970),
    ("layout-column-widths.xlsx", 0xff2a_ff3c_b9d4_be98),
    ("edge-merged-styled.xlsx", 0x681f_0895_a76e_26af),
    ("content-empty-sheet.xlsx", 0x3b95_a806_69ca_ba55),
];

#[test]
fn golden_drawing_is_unchanged() {
    for &(name, expected) in DRAWING {
        let actual = drawing_hash(&export(name));
        assert_eq!(
            actual, expected,
            "{name}: отрисовка разошлась с эталоном (факт {actual:#018x})"
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
///
/// `fit_to_height` — чтобы ширина осталась естественной: умолчание уместило бы
/// лист в одну страницу по ширине, и полос для повтора не стало бы.
#[test]
fn first_columns_repeat_on_every_page() {
    const FIXTURE: &str = "size-30x30x2.xlsx";
    let bytes = export_with(
        FIXTURE,
        PageConfig {
            repeat_first_columns: 1,
            fit_to_height: Some(1),
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

/// Синтетическая книга: лист `cols` столбцов на `rows` строк из чисел и
/// заданные настройки печати.
///
/// Фикстур с `fitToPage` в `test-fixtures/xlsx` нет (генератор их не пишет), а
/// зависеть от реальных книг пользователя тест не должен. Столбец здесь —
/// дефолтные 64 px, строка — 20 px (`xlsx::layout`): на A4 с полями по
/// умолчанию по ширине помещается 10 столбцов, масштаб 100 %.
fn fit_workbook(cols: u32, rows: u32, print: PrintSettings) -> Workbook {
    const PART: &str = "xl/worksheets/sheet1.xml";
    let mut builder = WorksheetBuilder::new(PART);
    for row in 0..rows {
        for col in 0..cols {
            builder
                .push(
                    row,
                    Cell::new(col, 0, CellValue::Number(f64::from(row * cols + col))),
                )
                .expect("ячейка");
        }
    }
    let content = SheetContent {
        cells: builder.finish(),
        print,
        ..SheetContent::default()
    };
    let meta = WorksheetMeta {
        name: "Лист1".to_owned(),
        part: PART.to_owned(),
        state: SheetState::Visible,
    };
    Workbook::new(
        vec![Sheet::new(meta, content)],
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
}

/// Экспортировать синтетический лист с заданными настройками страницы.
fn fit_export(book: &Workbook, page: PageConfig) -> Vec<u8> {
    PdfExporter::new(PdfOptions {
        page,
        ..PdfOptions::default()
    })
    .export_xlsx_sheet(book, 0)
    .expect("лист экспортируется")
}

/// Число страниц готового PDF.
fn fit_pages(bytes: &[u8]) -> usize {
    lopdf::Document::load_mem(bytes)
        .expect("PDF разбирается lopdf")
        .get_pages()
        .len()
}

/// Умолчание — уместить по ширине: широкий лист печатается одной полосой
/// столбцов, хотя без границ (контроль с естественной шириной) их две.
#[test]
fn wide_sheet_fits_width_by_default() {
    let book = fit_workbook(12, 3, PrintSettings::default());
    let natural = fit_export(
        &book,
        PageConfig {
            fit_to_height: Some(1),
            ..PageConfig::default()
        },
    );
    assert_eq!(
        fit_pages(&natural),
        2,
        "контроль: 12 столбцов по 64 px не влезают в 680 px одной полосы"
    );
    let default = fit_export(&book, PageConfig::default());
    assert_eq!(
        fit_pages(&default),
        1,
        "широкий лист не уместился по ширине"
    );
}

/// `fitToPage` листа задаёт сетку страниц: `fitToWidth = 1`, `fitToHeight = 0`
/// — одна полоса столбцов и столько страниц, сколько полос строк, а не
/// произведение полос (без листовой настройки их 2 × 2).
#[test]
fn sheet_fit_to_page_drives_pagination() {
    let book = fit_workbook(
        12,
        100,
        PrintSettings {
            fit_to_page: true,
            fit_to_width: 1,
            fit_to_height: 0,
            ..PrintSettings::default()
        },
    );
    // Явная ширина в три страницы 100 %-масштаб не трогает (12 столбцов и так
    // влезают в две полосы): видно произведение полос — 2 × 3.
    let natural = fit_export(
        &book,
        PageConfig {
            fit_to_width: Some(3),
            ..PageConfig::default()
        },
    );
    assert_eq!(
        fit_pages(&natural),
        6,
        "контроль: 2 полосы столбцов × 3 полосы строк"
    );
    let fitted = fit_export(&book, PageConfig::default());
    assert_eq!(
        fit_pages(&fitted),
        2,
        "одна полоса столбцов, две полосы строк"
    );
}

/// `fitToWidth = 2` листа — две страницы по ширине там, где умолчание
/// («уместить в одну») ужимает лист до одной: настройка листа сильнее.
#[test]
fn sheet_fit_to_width_counts_pages_wide() {
    let book = fit_workbook(
        20,
        3,
        PrintSettings {
            fit_to_page: true,
            fit_to_width: 2,
            fit_to_height: 0,
            ..PrintSettings::default()
        },
    );
    assert_eq!(
        fit_pages(&fit_export(&book, PageConfig::default())),
        2,
        "две страницы по ширине: 20 столбцов — две полосы по десять"
    );
    let plain = fit_workbook(20, 3, PrintSettings::default());
    assert_eq!(
        fit_pages(&fit_export(&plain, PageConfig::default())),
        1,
        "умолчание уместило бы те же 20 столбцов в одну страницу"
    );
}

/// Явный `fit_to_width` вызывающего сильнее листового `fitToWidth`.
#[test]
fn explicit_fit_beats_sheet_fit() {
    let book = fit_workbook(
        27,
        3,
        PrintSettings {
            fit_to_page: true,
            fit_to_width: 1,
            fit_to_height: 0,
            ..PrintSettings::default()
        },
    );
    let page = PageConfig {
        fit_to_width: Some(3),
        ..PageConfig::default()
    };
    assert_eq!(
        fit_pages(&fit_export(&book, page)),
        3,
        "явные три страницы по ширине сильнее листовой одной"
    );
}

/// Умолчание только ужимает: лист, помещающийся по ширине, печатается в
/// 100 % — отрисовка та же, что с естественной шириной.
#[test]
fn narrow_sheet_keeps_full_scale() {
    let book = fit_workbook(6, 3, PrintSettings::default());
    let default = fit_export(&book, PageConfig::default());
    let natural = fit_export(
        &book,
        PageConfig {
            fit_to_height: Some(1),
            ..PageConfig::default()
        },
    );
    assert_eq!(fit_pages(&default), 1);
    assert_eq!(
        drawing_hash(&default),
        drawing_hash(&natural),
        "умолчание увеличило узкий лист"
    );
}

/// Масштаб `scale` листа применяется, когда `fitToPage` нет, и даёт ту же
/// отрисовку, что явный масштаб вызывающего.
#[test]
fn sheet_scale_applies_without_fit_to_page() {
    let sheet_book = fit_workbook(
        6,
        3,
        PrintSettings {
            scale: 50,
            ..PrintSettings::default()
        },
    );
    let plain_book = fit_workbook(6, 3, PrintSettings::default());
    let from_sheet = fit_export(&sheet_book, PageConfig::default());
    let from_caller = fit_export(
        &plain_book,
        PageConfig {
            scale: 0.5,
            ..PageConfig::default()
        },
    );
    assert_eq!(
        drawing_hash(&from_sheet),
        drawing_hash(&from_caller),
        "листовые 50 % должны совпасть с явными 0.5"
    );
}

/// Явный `scale` вызывающего сильнее листового `scale`.
#[test]
fn explicit_scale_beats_sheet_scale() {
    let sheet_book = fit_workbook(
        6,
        3,
        PrintSettings {
            scale: 50,
            ..PrintSettings::default()
        },
    );
    let plain_book = fit_workbook(6, 3, PrintSettings::default());
    let page = PageConfig {
        scale: 0.25,
        ..PageConfig::default()
    };
    assert_eq!(
        drawing_hash(&fit_export(&sheet_book, page.clone())),
        drawing_hash(&fit_export(&plain_book, page)),
        "явные 0.25 должны перебить листовые 50 %"
    );
}

/// Книга `rows` × `cols` из чисел с объединением (границы включительно), чья
/// левая верхняя ячейка несёт текст `text`.
fn merge_workbook(rows: u32, cols: u32, merge: (u32, u32, u32, u32), text: &str) -> Workbook {
    const PART: &str = "xl/worksheets/sheet1.xml";
    let (first_row, first_col, last_row, last_col) = merge;
    let mut builder = WorksheetBuilder::new(PART);
    for row in 0..rows {
        for col in 0..cols {
            let value = if row == first_row && col == first_col {
                CellValue::InlineString(text.into())
            } else {
                CellValue::Number(f64::from(row * cols + col))
            };
            builder.push(row, Cell::new(col, 0, value)).expect("ячейка");
        }
    }
    let mut content = SheetContent {
        cells: builder.finish(),
        ..SheetContent::default()
    };
    content.merges.push(doc_converter_xlsx::Range {
        first: CellRef::new(first_row, first_col),
        last: CellRef::new(last_row, last_col),
    });
    let meta = WorksheetMeta {
        name: "Лист1".to_owned(),
        part: PART.to_owned(),
        state: SheetState::Visible,
    };
    Workbook::new(
        vec![Sheet::new(meta, content)],
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
}

/// Сколько раз текст встречается на странице.
fn text_count(doc: &lopdf::Document, page_id: ObjectId, needle: &str) -> usize {
    page_text(doc, page_id).matches(needle).count()
}

/// Объединение шире полосы столбцов: на страницах-продолжениях часть таблицы
/// не должна пропадать — объединение печатается на каждой полосе, которую
/// пересекает.
#[test]
fn merged_cell_survives_column_bands() {
    const TEXT: &str = "СКВОЗНОЙЗАГОЛОВОК";
    // 30 столбцов по 64 px — три полосы по десять; A1:M1 (0..13) накрывает
    // первую и вторую и не достаёт до третьей. `fit_to_height` оставляет ширину
    // естественной: умолчание уместило бы лист в одну страницу по ширине, и
    // полос не стало бы.
    let book = merge_workbook(2, 30, (0, 0, 0, 12), TEXT);
    let bytes = fit_export(
        &book,
        PageConfig {
            fit_to_height: Some(1),
            ..PageConfig::default()
        },
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 3, "30 столбцов — три полосы");
    // Полосы идут в порядке страниц: объединение нужно первой и второй, третьей
    // (за границей 13-го столбца) — нет.
    let expected = [1, 1, 0];
    for ((number, page_id), expected) in pages.iter().zip(expected) {
        assert_eq!(
            text_count(&doc, *page_id, TEXT),
            expected,
            "страница {number}: объединение не в своей полосе"
        );
    }
}

/// Объединение выше полосы строк: его вторая половина печатается на
/// странице-продолжении.
#[test]
fn merged_cell_survives_row_bands() {
    const TEXT: &str = "ВЕРТИКАЛЬНОЕОБЪЕДИНЕНИЕ";
    // 60 строк по 20 px не помещаются в область содержимого: полосы строк —
    // 0..48 и 48..60; A41:A56 пересекает границу.
    let book = merge_workbook(60, 2, (40, 0, 55, 0), TEXT);
    let bytes = fit_export(&book, PageConfig::default());
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 2, "60 строк — две полосы");
    for (number, page_id) in &pages {
        assert_eq!(
            text_count(&doc, *page_id, TEXT),
            1,
            "страница {number}: объединения нет в её полосе"
        );
    }
}

/// Объединение внутри одной полосы по-прежнему печатается один раз.
#[test]
fn merged_cell_inside_one_band_prints_once() {
    const TEXT: &str = "ВНУТРИПОЛОСЫ";
    let book = merge_workbook(2, 30, (0, 0, 0, 2), TEXT);
    let bytes = fit_export(
        &book,
        PageConfig {
            fit_to_height: Some(1),
            ..PageConfig::default()
        },
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 3, "30 столбцов — три полосы");
    let total: usize = pages
        .values()
        .map(|page_id| text_count(&doc, *page_id, TEXT))
        .sum();
    assert_eq!(total, 1, "объединение внутри полосы размножилось");
}
