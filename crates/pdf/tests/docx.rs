//! Сквозные проверки: документ DOCX → PDF.
//!
//! Фикстуры — те же из `test-fixtures/docx`, что читает `crates/docx`. Проверки
//! структурные: файл разбирается `lopdf`, размер страницы берётся из
//! `/MediaBox`, текст — из потока содержимого по карте `ToUnicode` (те же
//! приёмы, что у XLSX-тестов, `tests/export.rs`).
//!
//! Эталонные числа берутся у самой раскладки (`layout_document`): PDF обязан
//! повторить её страницы один в один, а не совпасть с числами, вписанными в
//! тест. Исключение — проверки геометрии: там рядом стоят размеры,
//! посчитанные в тесте руками, иначе `layout_document` и экспорт могли бы
//! разойтись одинаково и тест этого не заметил бы.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use doc_converter_docx::layout::{layout_document, LayoutItem, LayoutOptions, PageLayout};
use doc_converter_docx::{BlockItem, Document, Inline, Margins, PageSize, Toggle, Twips};
use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};
use lopdf::Document as Pdf;

/// Пикселей раскладки в точке PDF: 96 dpi против 72 dpi.
const PX_PER_POINT: f32 = 96.0 / 72.0;
/// Допуск сверки размера страницы, точки (приёмка спринта — 1 pt).
const SIZE_TOLERANCE_PT: f32 = 1.0;

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
        &mut FontRegistry::new(DEFAULT_FONT_CACHE),
    )
    .expect("раскладка собирается")
}

/// Экспортировать документ настройками по умолчанию.
fn export(doc: &Document) -> Vec<u8> {
    PdfExporter::new(PdfOptions::default())
        .export_docx(doc)
        .expect("документ экспортируется")
}

/// Размер страницы `id` в точках: ширина и высота `/MediaBox`.
fn page_size(pdf: &Pdf, id: lopdf::ObjectId) -> (f32, f32) {
    let page = pdf.get_dictionary(id).expect("словарь страницы");
    let media = page
        .get(b"MediaBox")
        .expect("/MediaBox")
        .as_array()
        .expect("/MediaBox — массив");
    let value = |i: usize| media[i].as_float().expect("число");
    (value(2) - value(0), value(3) - value(1))
}

/// Размеры страниц PDF по порядку номеров.
fn page_sizes(bytes: &[u8]) -> Vec<(f32, f32)> {
    let pdf = Pdf::load_mem(bytes).expect("PDF разбирается lopdf");
    pdf.get_pages()
        .values()
        .map(|&id| page_size(&pdf, id))
        .collect()
}

/// Сверка: страницы PDF повторяют страницы раскладки — число и размеры.
fn assert_pages_match_layout(name: &str, layout: &PageLayout, bytes: &[u8]) {
    let sizes = page_sizes(bytes);
    assert_eq!(
        sizes.len(),
        layout.pages.len(),
        "{name}: число страниц PDF против раскладки"
    );
    for (index, (size, page)) in sizes.iter().zip(&layout.pages).enumerate() {
        let expected = (page.width / PX_PER_POINT, page.height / PX_PER_POINT);
        assert!(
            (size.0 - expected.0).abs() <= SIZE_TOLERANCE_PT
                && (size.1 - expected.1).abs() <= SIZE_TOLERANCE_PT,
            "{name}: страница {index}: PDF {size:?} против раскладки {expected:?} pt"
        );
    }
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
///
/// Скопировано из `tests/export.rs`: шрифтов у страницы может быть несколько
/// (regular и полужирный), поэтому карты берутся по именам ресурсов, а не
/// одна первая.
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

/// Пары «код в потоке → символ Unicode» из `beginbfchar`-секции CMap.
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
    // токенов даёт настоящую последовательность прогонов.
    let mut events: Vec<(usize, &str)> = Vec::new();
    events.extend(text.match_indices("Tf"));
    events.extend(text.match_indices("Tj"));
    events.sort_by_key(|(at, _)| *at);

    let mut runs = Vec::new();
    let mut font = String::new();
    for (at, token) in events {
        if token == "Tf" {
            if let Some(slash) = text[..at].rfind('/') {
                font = text[slash + 1..at]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
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

/// Есть ли на страницах элемент `Float` — проверка, что фикстура задевает шов.
fn has_float(layout: &PageLayout) -> bool {
    layout.pages.iter().any(|page| {
        page.items
            .iter()
            .any(|item| matches!(item, LayoutItem::Float(_)))
    })
}

/// Число страниц, размер страницы и текст на трёх фикстурах разной геометрии.
///
/// `sections.docx` — единственная фикстура с двумя секциями: книжная и
/// альбомная страницы в одном документе.
#[test]
fn pages_repeat_layout_geometry() {
    for name in [
        "simple/multiple_paragraphs_0.docx",
        "edge_cases/multilingual.docx",
        "basic/sections.docx",
    ] {
        let doc = document(name);
        let layout = layout(&doc);
        let bytes = export(&doc);
        assert_pages_match_layout(name, &layout, &bytes);
    }
}

/// Альбомная секция не разворачивается обратно: у страниц свои ширина и высота.
#[test]
fn landscape_page_keeps_its_orientation() {
    let doc = document("basic/sections.docx");
    let layout = layout(&doc);
    let sizes = page_sizes(&export(&doc));

    let portrait = [595.28, 841.89];
    let landscape = [841.89, 595.28];
    let close = |size: (f32, f32), expected: [f32; 2]| {
        (size.0 - expected[0]).abs() <= SIZE_TOLERANCE_PT
            && (size.1 - expected[1]).abs() <= SIZE_TOLERANCE_PT
    };
    let orient = |size: (f32, f32)| {
        if size.0 > size.1 {
            "landscape"
        } else {
            "portrait"
        }
    };
    let kinds: Vec<&str> = sizes.iter().map(|&size| orient(size)).collect();

    assert!(
        kinds.contains(&"portrait") && kinds.contains(&"landscape"),
        "в фикстуре обе ориентации: {kinds:?}"
    );
    for size in sizes {
        assert!(
            close(size, portrait) || close(size, landscape),
            "страница {size:?} не совпала ни с A4 книжной, ни с альбомной"
        );
    }
    assert_pages_match_layout("basic/sections.docx", &layout, &export(&doc));
}

/// Альбомная секция посреди документа: страницы берут геометрию своей секции.
///
/// У `sections.docx` альбомная секция последняя, и ошибки «геометрия секции
/// залипает на весь документ» на ней не видно: книжных страниц после альбомной
/// нет. Здесь тот же материал фикстуры укладывается в «книжная → альбомная →
/// книжная»: так выглядят реальные документы с альбомными вставками (например,
/// широкими таблицами), и книжные страницы после вставки обязаны остаться
/// книжными.
#[test]
fn a_landscape_section_does_not_capture_its_neighbours() {
    /// A4 в пикселях раскладки: twips / 15 (96 dpi против 1440 twips в дюйме).
    const A4_W_PX: f32 = 11_906.0 / 15.0;
    const A4_H_PX: f32 = 16_838.0 / 15.0;

    let mut doc = document("basic/sections.docx");
    let portrait = doc.body.sections[0].clone();
    let landscape = doc.body.sections[1].clone();
    let paragraphs: Vec<_> = doc
        .body
        .items
        .iter()
        .filter_map(|item| match item {
            BlockItem::Paragraph(paragraph) => Some(paragraph.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(paragraphs.len(), 2, "в фикстуре два абзаца с текстом");

    // Абзац, которым кончается секция, остаётся в ней же, а следующий абзац
    // открывает уже новую секцию (`collect_sections` в `document.rs`).
    let mut first_break = paragraphs[0].clone();
    first_break.section_break = Some(Box::new(portrait.properties.clone()));
    let mut second_break = paragraphs[1].clone();
    second_break.section_break = Some(Box::new(landscape.properties.clone()));

    doc.body.items = vec![
        BlockItem::Paragraph(first_break),
        BlockItem::Paragraph(paragraphs[1].clone()),
        BlockItem::Paragraph(second_break),
        BlockItem::Paragraph(paragraphs[0].clone()),
        BlockItem::SectPr(portrait.properties.clone()),
    ];
    let portrait_again = portrait.clone();
    doc.body.sections = vec![portrait, landscape, portrait_again];

    let layout = layout(&doc);
    let bytes = export(&doc);
    let sizes = page_sizes(&bytes);

    // Альбомная страница — те же стороны, что у книжной, но в обратном порядке:
    // так их и переставляет сам OOXML-производитель, `w:orient` раскладка не
    // читает (см. `page_for` в `crates/docx/src/layout/pagination.rs`).
    let expected = [(A4_W_PX, A4_H_PX), (A4_H_PX, A4_W_PX), (A4_W_PX, A4_H_PX)];
    assert_eq!(sizes.len(), expected.len(), "трём секциям — три страницы");
    for (index, want) in expected.into_iter().enumerate() {
        let page = &layout.pages[index];
        assert!(
            (page.width - want.0).abs() < 0.01 && (page.height - want.1).abs() < 0.01,
            "страница {index}: раскладка {}x{} вместо {want:?} px",
            page.width,
            page.height
        );
        let got = sizes[index];
        assert!(
            (got.0 - want.0 / PX_PER_POINT).abs() <= SIZE_TOLERANCE_PT
                && (got.1 - want.1 / PX_PER_POINT).abs() <= SIZE_TOLERANCE_PT,
            "страница {index}: PDF {got:?} вместо {want:?} px — геометрия не из своей секции"
        );
    }
    assert_pages_match_layout(
        "sections.docx (книжная → альбомная → книжная)",
        &layout,
        &bytes,
    );
}

/// Размер и поля секции из документа доезжают до страницы PDF.
///
/// Геометрия задаётся прямо в модели (Letter вместо A4 и поля в дюйм): третья
/// фикстура «разной геометрии» из набора — с другими полями, которых больше
/// нет ни у одной.
#[test]
fn custom_page_size_and_margins_reach_the_page() {
    const LETTER: (i32, i32) = (12_240, 15_840);
    const MARGIN_TWIPS: i32 = 1_440;

    let mut doc = document("edge_cases/multilingual.docx");
    let section = doc
        .body
        .sections
        .first_mut()
        .expect("у фикстуры есть секция");
    let size = PageSize {
        width: Twips::new(LETTER.0),
        height: Twips::new(LETTER.1),
    };
    let margins = Margins {
        top: Twips::new(MARGIN_TWIPS),
        right: Twips::new(MARGIN_TWIPS),
        bottom: Twips::new(MARGIN_TWIPS),
        left: Twips::new(MARGIN_TWIPS),
        header: Some(Twips::new(720)),
        footer: Some(Twips::new(720)),
        gutter: None,
    };
    section.page_size = size.clone();
    section.properties.page_size = size;
    // Полосу набора раскладка берёт из `Section::margins`, но и сырые свойства
    // держим в согласии: так их заполняет и сам парсер.
    section.margins = margins.clone();
    section.properties.margins = margins;

    let layout = layout(&doc);
    let bytes = export(&doc);
    let sizes = page_sizes(&bytes);

    // Letter: 12240 × 15840 twips = 816 × 1056 px = 612 × 792 pt.
    assert_eq!(sizes.len(), 1, "три абзаца — одна страница");
    assert!(
        (sizes[0].0 - 612.0).abs() <= SIZE_TOLERANCE_PT
            && (sizes[0].1 - 792.0).abs() <= SIZE_TOLERANCE_PT,
        "страница {:?} вместо Letter 612 × 792 pt",
        sizes[0]
    );
    // Поля доехали до раскладки: 1440 twips = 96 px.
    let margins = layout.pages[0].margins;
    assert!(
        (margins.left - 96.0).abs() < 0.5 && (margins.top - 96.0).abs() < 0.5,
        "поля раскладки {margins:?} вместо 96 px"
    );
    assert_pages_match_layout("multilingual (Letter)", &layout, &bytes);
}

/// Текст абзаца действительно попадает в PDF, включая кириллицу.
#[test]
fn paragraph_text_reaches_the_pdf() {
    let doc = document("edge_cases/multilingual.docx");
    let bytes = export(&doc);
    let text = page_text(&bytes, 0);

    for expected in ["English", "Русский"] {
        assert!(
            text.contains(expected),
            "в тексте страницы нет {expected:?}: {text:?}"
        );
    }
}

/// Полужирный абзац выводится своим встроенным шрифтом, обычный — обычным.
///
/// Начертание берётся у отрезка абзаца, поэтому в полужирный переводится целый
/// абзац: раскладка склеивает run'ы с общим кеглем и цветом, и начертание у
/// склеенного отрезка — от первого run'а (`docx::layout::paragraph::segments`).
#[test]
fn bold_paragraph_uses_its_own_font() {
    let mut doc = document("simple/multiple_paragraphs_0.docx");
    let Some(BlockItem::Paragraph(paragraph)) = doc.body.items.first_mut() else {
        panic!("фикстура начинается с абзаца");
    };
    let Some(Inline::Run(run)) = paragraph.runs.first_mut() else {
        panic!("у абзаца есть run");
    };
    run.rpr.b = Some(Toggle::On);

    let bytes = export(&doc);
    let pdf = Pdf::load_mem(&bytes).expect("PDF разбирается lopdf");
    let page = *pdf.get_pages().values().next().expect("страница есть");
    let runs = text_runs(&pdf, page);
    let text: String = runs.iter().map(|(_, run)| run.as_str()).collect();
    assert!(
        text.contains("Paragraph 1") && text.contains("Paragraph 2"),
        "в тексте страницы нет обоих абзацев: {text:?}"
    );
    assert!(
        pdf.get_page_fonts(page).expect("шрифты страницы").len() >= 2,
        "полужирный абзац встраивается своим шрифтом"
    );
    let resource = |needle: &str| {
        runs.iter()
            .find(|(_, run)| run.contains(needle))
            .map(|(font, _)| font.clone())
            .unwrap_or_else(|| panic!("строка {needle:?} не найдена"))
    };
    assert_ne!(
        resource("Paragraph 1"),
        resource("Paragraph 2"),
        "полужирный абзац выведен шрифтом обычного"
    );
}

/// Таблица не паникует и не роняет экспорт; текст вокруг неё на месте.
///
/// Содержимое ячеек пока пропускается осознанно (`docx::tables`), поэтому тест
/// и не требует его в PDF: слайс таблиц добавит своё, не правя этот тест.
#[test]
fn table_fixture_exports_without_panic() {
    let doc = document("complex/text_and_table.docx");
    let layout = layout(&doc);
    let bytes = export(&doc);

    assert_pages_match_layout("complex/text_and_table.docx", &layout, &bytes);
    let text = page_text(&bytes, 0);
    for expected in ["Text before", "Text after"] {
        assert!(
            text.contains(expected),
            "в тексте страницы нет {expected:?}: {text:?}"
        );
    }
}

/// Плавающий рисунок не паникует и не роняет экспорт.
#[test]
fn float_fixture_exports_without_panic() {
    let name = "images/anchor_wrap_square.docx";
    let doc = document(name);
    let layout = layout(&doc);

    assert!(
        has_float(&layout),
        "{name}: фикстура без `Float` ничего не проверяет"
    );
    let bytes = export(&doc);
    assert_pages_match_layout(name, &layout, &bytes);
}

/// Ни одна фикстура не роняет экспорт: он либо отдаёт PDF, повторяющий
/// раскладку, либо возвращает ошибку — но не паникует.
#[test]
fn every_fixture_exports_or_fails_without_panic() {
    let mut checked = 0;
    for entry in walk(&fixture("")) {
        let path = entry;
        let name = path
            .strip_prefix(fixture(""))
            .unwrap_or(&path)
            .display()
            .to_string();
        let bytes = std::fs::read(&path).expect("фикстура читается");
        let Ok(doc) = doc_converter_docx::open(bytes) else {
            continue; // битые фикстуры (`broken/`) — не предмет этого слайса
        };
        checked += 1;

        let mut registry = FontRegistry::new(DEFAULT_FONT_CACHE);
        let Ok(layout) = layout_document(&doc, &LayoutOptions::default(), &mut registry) else {
            assert!(
                PdfExporter::new(PdfOptions::default())
                    .export_docx(&doc)
                    .is_err(),
                "{name}: без раскладки экспорт обязан вернуть ошибку"
            );
            continue;
        };
        let bytes = PdfExporter::new(PdfOptions::default())
            .export_docx(&doc)
            .unwrap_or_else(|e| panic!("{name}: экспорт упал: {e}"));
        assert_pages_match_layout(&name, &layout, &bytes);
    }
    assert!(
        checked >= 50,
        "обошли всего {checked} фикстур — проверять нечего"
    );
}

/// Все `*.docx` в каталоге рекурсивно.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "docx") {
            files.push(path);
        }
    }
    files.sort();
    files
}
