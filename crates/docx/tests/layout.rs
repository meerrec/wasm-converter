//! Тесты раскладки DOCX: `layout_document` и связанные функции.
//!
//! Каждый тест опирается на существующую фикстуру и падает, если её нет:
//! молчаливый `return` делал бы зелёный прогон ничего не значащим.
//! Ожидаемые числа берутся из самой фикстуры — `w:sectPr` в `document.xml`
//! и сайдкара `*.json` (`expectedParagraphs`, `expectedTables`).

use std::path::{Path, PathBuf};

use doc_converter_docx::{
    layout::{layout_document, LayoutItem, LayoutOptions, PageLayout},
    model::{BlockItem, Document},
    open, Error,
};
use doc_converter_render::font::FontRegistry;

/// Каталог с фикстурами.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Путь к фикстуре; отсутствие фикстуры роняет тест, а не пропускает его.
fn fixture_path(name: &str) -> PathBuf {
    let path = fixtures_dir().join(name);
    assert!(path.exists(), "нет фикстуры {}", path.display());
    path
}

/// Загрузить фикстуру как DOCX.
fn load_fixture(path: &Path) -> Result<Document, Error> {
    let bytes =
        std::fs::read(path).unwrap_or_else(|_| panic!("Не удалось прочитать {}", path.display()));
    open(bytes)
}

/// Число элементов-абзацев в раскладке.
///
/// `layout_paragraph` кладёт элемент на каждую строку абзаца, поэтому счётчик
/// равен числу строк, а не числу абзацев: у однострочного абзаца элемент один.
fn paragraph_items(layout: &PageLayout) -> usize {
    layout
        .pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| matches!(item, LayoutItem::Paragraph { .. }))
        .count()
}

/// Сравнить пиксельный размер с ожидаемым: `f32` из twips на равенство не проверяем.
fn assert_px(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{what}: ожидалось {expected} px, получено {actual} px"
    );
}

#[test]
fn test_layout_simple_document() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("simple/one_paragraph.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // simple/one_paragraph.json: expectedParagraphs = 1 — «Hello, World!» в одну строку.
    assert_eq!(
        paragraph_items(&layout),
        1,
        "Один однострочный абзац должен дать ровно один элемент-абзац"
    );
    assert_eq!(
        layout.pages.len(),
        1,
        "Единственный абзац должен лежать на одной странице"
    );
}

#[test]
fn test_layout_multiple_paragraphs() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("simple/multiple_paragraphs_0.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // simple/multiple_paragraphs_0.json: expectedParagraphs = 5, каждый абзац —
    // «Paragraph N» в одну строку, значит и элементов-абзацев ровно пять.
    assert_eq!(
        paragraph_items(&layout),
        5,
        "Пять однострочных абзацев должны дать ровно пять элементов-абзацев"
    );
    assert_eq!(
        layout.pages.len(),
        1,
        "Пять коротких абзацев должны уместиться на одной странице"
    );
}

#[test]
fn test_layout_empty_document() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("simple/empty.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Пустой документ — ровно одна страница: тело закрывает `w:sectPr`, но он
    // описывает последнюю секцию, а не разрыв, и лишнего листа за ним нет.
    assert_eq!(
        layout.pages.len(),
        1,
        "Пустой документ должен иметь ровно одну страницу"
    );
    // simple/empty.json: expectedParagraphs = 0 — тело пустое, элементов быть не должно.
    assert_eq!(
        paragraph_items(&layout),
        0,
        "В пустом документе не должно быть элементов-абзацев"
    );
}

#[test]
fn test_layout_with_tables() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("tables/simple_2x2.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));

    // tables/simple_2x2.json: expectedTables = 1 — без таблицы в модели тест
    // проверял бы не то, что заявлено.
    let tables = doc
        .body
        .items
        .iter()
        .filter(|block| matches!(block, BlockItem::Table(_)))
        .count();
    assert_eq!(tables, 1, "Фикстура должна содержать ровно одну таблицу");

    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    assert!(
        !layout.pages.is_empty(),
        "Документ с таблицей должен иметь страницы"
    );
    // Таблица доезжает и до страницы: `tables::layout_table` кладёт её элементом
    // с ячейками, а не только занимает высоту в потоке.
    let table_items: Vec<&LayoutItem> = layout
        .pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| matches!(item, LayoutItem::Table { .. }))
        .collect();
    assert_eq!(table_items.len(), 1, "Таблица должна попасть на страницу");
    match table_items[0] {
        LayoutItem::Table { cells, .. } => {
            assert_eq!(cells.len(), 4, "В таблице 2×2 должно быть четыре ячейки");
        }
        other => panic!("Ожидался элемент-таблица, получено {other:?}"),
    }
}

#[test]
fn test_layout_pagination() {
    // Пагинация на длинном тексте: абзац из ста прогонов обязан перенестись
    // на несколько строк, а не схлопнуться в одну.
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("basic/many_runs_paragraph.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));

    // basic/many_runs_paragraph.json: expectedParagraphs = 1.
    let paragraphs = doc
        .body
        .items
        .iter()
        .filter(|block| matches!(block, BlockItem::Paragraph(_)))
        .count();
    assert_eq!(paragraphs, 1, "Фикстура должна содержать ровно один абзац");

    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    assert!(
        !layout.pages.is_empty(),
        "Длинный документ должен иметь хотя бы одну страницу"
    );
    assert!(
        paragraph_items(&layout) > 1,
        "Абзац из ста прогонов должен переноситься на несколько строк"
    );
}

#[test]
fn test_layout_page_dimensions() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("simple/one_paragraph.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Фикстура задаёт A4: `w:pgSz w="11906" h="16838"`, `w:pgMar` по 1134 twips
    // на все поля, `w:header`/`w:footer` по 709, `w:gutter` = 0. Один пиксель —
    // это 1/96 дюйма, один twip — 1/1440, значит 1 px = 15 twips.
    let page_width = 11906.0 / 15.0;
    let page_height = 16838.0 / 15.0;
    let margin = 1134.0 / 15.0;

    let first_page = &layout.pages[0];
    assert_px(first_page.width, page_width, "ширина страницы");
    assert_px(first_page.height, page_height, "высота страницы");
    assert_px(first_page.margins.left, margin, "левое поле");
    assert_px(first_page.margins.right, margin, "правое поле");
    assert_px(first_page.margins.top, margin, "верхнее поле");
    assert_px(first_page.margins.bottom, margin, "нижнее поле");
    assert_px(
        first_page.margins.header,
        709.0 / 15.0,
        "поле верхнего колонтитула",
    );
    assert_px(
        first_page.margins.footer,
        709.0 / 15.0,
        "поле нижнего колонтитула",
    );
    assert_px(first_page.margins.gutter, 0.0, "переплётный отступ");

    // Размер листа у всех страниц документа один и тот же.
    for page in &layout.pages {
        assert_px(page.width, page_width, "ширина страницы");
        assert_px(page.height, page_height, "высота страницы");
        assert!(
            page.margins.left >= 0.0,
            "Левое поле не может быть отрицательным"
        );
        assert!(
            page.margins.right >= 0.0,
            "Правое поле не может быть отрицательным"
        );
        assert!(
            page.margins.top >= 0.0,
            "Верхнее поле не может быть отрицательным"
        );
        assert!(
            page.margins.bottom >= 0.0,
            "Нижнее поле не может быть отрицательным"
        );
    }
}

#[test]
fn test_layout_item_positions() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("simple/one_paragraph.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Первый (и единственный) абзац начинается в левом верхнем углу полосы набора.
    let first_page = &layout.pages[0];
    let first_item = first_page
        .items
        .first()
        .expect("На странице должен быть элемент");
    match first_item {
        LayoutItem::Paragraph { rect, .. } => {
            assert_px(rect.x, first_page.margins.left, "отступ абзаца слева");
            assert_px(rect.y, first_page.margins.top, "отступ абзаца сверху");
        }
        other => panic!("Ожидался элемент-абзац, получено {other:?}"),
    }

    // Текст не вылезает за поля: начало — не левее левого поля и не выше верхнего,
    // конец строки — не правее правого.
    for page in &layout.pages {
        for item in &page.items {
            match item {
                LayoutItem::Paragraph { rect, .. } | LayoutItem::Table { rect, .. } => {
                    assert!(rect.x >= 0.0, "Позиция X должна быть неотрицательной");
                    assert!(rect.y >= 0.0, "Позиция Y должна быть неотрицательной");
                    assert!(rect.width >= 0.0, "Ширина должна быть неотрицательной");
                    assert!(rect.height >= 0.0, "Высота должна быть неотрицательной");
                    assert!(
                        rect.x + 0.01 >= page.margins.left,
                        "Элемент заходит левее левого поля: x = {}, поле = {}",
                        rect.x,
                        page.margins.left
                    );
                    assert!(
                        rect.y + 0.01 >= page.margins.top,
                        "Элемент заходит выше верхнего поля: y = {}, поле = {}",
                        rect.y,
                        page.margins.top
                    );
                    assert!(
                        rect.x + rect.width <= page.width - page.margins.right + 0.01,
                        "Элемент вылезает правее правого поля: {} > {}",
                        rect.x + rect.width,
                        page.width - page.margins.right
                    );
                }
                _ => {}
            }
        }
    }
}

#[test]
fn test_layout_sections_in_document_order() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("basic/sections.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));

    // Фикстура: первый абзац несёт `w:sectPr` внутри `w:pPr` и завершает книжную
    // секцию A4 (11906×16838 twips), финальный `w:sectPr` тела описывает
    // альбомную (16838×11906 twips). Поля обеих — 1134 twips.
    assert_eq!(doc.body.sections.len(), 2, "В теле две секции");

    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    assert_eq!(layout.pages.len(), 2, "Двум секциям — две страницы");
    assert_px(
        layout.pages[0].width,
        11906.0 / 15.0,
        "ширина книжной страницы",
    );
    assert_px(
        layout.pages[0].height,
        16838.0 / 15.0,
        "высота книжной страницы",
    );
    assert_px(
        layout.pages[1].width,
        16838.0 / 15.0,
        "ширина альбомной страницы",
    );
    assert_px(
        layout.pages[1].height,
        11906.0 / 15.0,
        "высота альбомной страницы",
    );

    // Абзац с концом секции принадлежит текущей секции, следующий — уже новой:
    // «First section» на книжной странице, «Second section» на альбомной.
    let texts_on = |index: usize| -> Vec<String> {
        layout.pages[index]
            .items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        texts_on(0),
        ["First section: portrait A4"],
        "Первый абзац остаётся на книжной странице"
    );
    assert_eq!(
        texts_on(1),
        ["Second section: landscape A4"],
        "Второй абзац уходит на альбомную страницу"
    );
}

/// Тексты элементов-абзацев в порядке раскладки.
fn texts_in(items: &[LayoutItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            LayoutItem::Paragraph { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Верх и высота каждого абзаца: колонтитул в этих фикстурах — абзацы.
fn paragraph_bounds(items: &[LayoutItem]) -> Vec<(f32, f32)> {
    items
        .iter()
        .filter_map(|item| match item {
            LayoutItem::Paragraph { rect, .. } => Some((rect.y, rect.height)),
            _ => None,
        })
        .collect()
}

/// Есть ли в элементах встроенный рисунок, у которого нашлись байты.
fn has_image(items: &[LayoutItem]) -> bool {
    items.iter().any(|item| {
        matches!(
            item,
            LayoutItem::Image {
                image_id: Some(_),
                ..
            }
        )
    })
}

/// Повторить тело `times` раз, чтобы документ занял несколько страниц.
///
/// Фикстуры колонтитулов одностраничные, а выбор `first`/`even` проверяется на
/// страницах 1..=3. Копии вставляются перед финальным `w:sectPr`: размноженный
/// конец секции разрезал бы документ, и колонтитулы стало бы не с чем сверять.
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

/// Раскладка фикстуры колонтитулов, растянутой на `times` копий тела.
fn chrome_layout(name: &str, times: usize) -> (Document, PageLayout) {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path(&format!("headers_footers/{name}.docx"));
    let mut doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    if times > 1 {
        stretch(&mut doc, times);
    }
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");
    (doc, layout)
}

#[test]
fn test_layout_default_header_and_footer_on_every_page() {
    let (_, layout) = chrome_layout("default_header_footer", 150);

    assert!(
        layout.pages.len() >= 3,
        "тело растянуто на несколько страниц, получено {}",
        layout.pages.len()
    );
    assert_eq!(
        layout.chrome.len(),
        layout.pages.len(),
        "колонтитул есть у каждой страницы"
    );

    for (index, chrome) in layout.chrome.iter().enumerate() {
        assert_eq!(
            texts_in(&chrome.header),
            ["Default header"],
            "верхний колонтитул страницы {}",
            index + 1
        );
        assert_eq!(
            texts_in(&chrome.footer),
            ["Default footer"],
            "нижний колонтитул страницы {}",
            index + 1
        );
    }

    // Колонтитул — не тело: в потоке страницы его текста быть не должно.
    let body: Vec<String> = layout
        .pages
        .iter()
        .flat_map(|page| texts_in(&page.items))
        .collect();
    assert!(
        body.iter()
            .all(|text| text != "Default header" && text != "Default footer"),
        "колонтитул не попадает в поток содержимого: {body:?}"
    );
}

#[test]
fn test_layout_header_sits_in_the_margin_band() {
    let (_, layout) = chrome_layout("default_header_footer", 1);
    let page = &layout.pages[0];
    let chrome = &layout.chrome[0];

    // `w:pgMar`: header и footer — 709 twips (47.27 px), верхнее и нижнее
    // поля — 1134 twips (75.6 px).
    let Some(&(header_y, header_h)) = paragraph_bounds(&chrome.header).first() else {
        panic!("в верхнем колонтитуле нет абзаца");
    };
    assert_px(
        header_y,
        709.0 / 15.0,
        "верхний колонтитул от края страницы",
    );
    assert!(
        header_y + header_h <= page.margins.top + 0.01,
        "колонтитул остаётся в верхнем поле: {} > {}",
        header_y + header_h,
        page.margins.top
    );

    let Some(&(footer_y, footer_h)) = paragraph_bounds(&chrome.footer).first() else {
        panic!("в нижнем колонтитуле нет абзаца");
    };
    assert_px(
        footer_y + footer_h,
        page.height - 709.0 / 15.0,
        "нижний колонтитул от края страницы",
    );
    assert!(
        footer_y >= page.height - page.margins.bottom - 0.01,
        "колонтитул ниже полосы набора: {} < {}",
        footer_y,
        page.height - page.margins.bottom
    );
}

#[test]
fn test_layout_title_pg_uses_the_first_page_header() {
    let (doc, layout) = chrome_layout("title_pg_first", 150);

    assert!(doc.body.sections[0].title_pg, "фикстура несёт `w:titlePg`");
    assert!(
        layout.pages.len() >= 2,
        "нужны первая и вторая страницы, получено {}",
        layout.pages.len()
    );

    assert_eq!(
        texts_in(&layout.chrome[0].header),
        ["First page header"],
        "на первой странице секции — колонтитул `first`"
    );
    assert_eq!(
        texts_in(&layout.chrome[1].header),
        ["Default header"],
        "на второй странице — колонтитул `default`"
    );
    assert!(
        layout.chrome[0].footer.is_empty() && layout.chrome[1].footer.is_empty(),
        "нижних колонтитулов в фикстуре нет"
    );
}

#[test]
fn test_layout_even_and_odd_headers_differ() {
    let (doc, layout) = chrome_layout("even_odd", 150);

    assert!(
        doc.settings.even_and_odd_headers,
        "фикстура включает `w:evenAndOddHeaders`"
    );
    assert!(
        layout.pages.len() >= 3,
        "нужны страницы 1..=3, получено {}",
        layout.pages.len()
    );

    // Первая страница секции: `w:titlePg` перевешивает чётность.
    assert_eq!(
        texts_in(&layout.chrome[0].header),
        ["First page header"],
        "страница 1"
    );
    assert_eq!(
        texts_in(&layout.chrome[1].header),
        ["Even page header"],
        "страница 2 — чётная"
    );
    assert_eq!(
        texts_in(&layout.chrome[2].header),
        ["Default header"],
        "страница 3 — нечётная"
    );
}

#[test]
fn test_layout_header_with_inline_image_is_drawn_in_the_margin() {
    let (_, layout) = chrome_layout("header_with_image", 1);
    let chrome = &layout.chrome[0];

    assert!(
        has_image(&chrome.header),
        "рисунок колонтитула раскладывается тем же путём, что и в теле"
    );
    let page = &layout.pages[0];
    let Some(&(first_y, _)) = paragraph_bounds(&chrome.header).first() else {
        panic!("в верхнем колонтитуле нет абзаца");
    };
    assert_px(
        first_y,
        page.margins.header,
        "содержимое колонтитула начинается от линии `w:header`",
    );

    // Рисунок идёт за строкой абзаца (долг встроенных рисунков: место внутри
    // строки текст колонтитула не занимает), но в полосу колонтитула он сдвинут
    // тем же сдвигом, что и текст.
    let rects: Vec<(f32, f32)> = chrome
        .header
        .iter()
        .filter_map(|item| match item {
            LayoutItem::Image {
                rect,
                image_id: Some(_),
                ..
            } => Some((rect.y, rect.height)),
            _ => None,
        })
        .collect();
    assert!(
        rects
            .iter()
            .any(|(y, height)| *y >= page.margins.header && *height > 0.0),
        "рисунок отсчитывается от линии `w:header` ({}): {rects:?}",
        page.margins.header
    );
}

#[test]
fn test_layout_page_without_headers_has_empty_chrome() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixture_path("simple/empty.docx");
    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    assert!(
        !layout.chrome.is_empty(),
        "страница есть и без колонтитулов"
    );
    assert!(
        layout
            .chrome
            .iter()
            .all(|chrome| chrome.header.is_empty() && chrome.footer.is_empty()),
        "нет ссылок на колонтитулы — нет и элементов"
    );
}

#[test]
fn test_layout_footer_with_page_field_is_laid_out_without_the_number() {
    let (_, layout) = chrome_layout("page_number_field", 1);
    let footer = texts_in(&layout.chrome[0].footer);

    // Статический текст колонтитула раскладывается, а само поле (`NUMPAGES`)
    // числом не разворачивается: вычисление полей — отдельный долг, раскладка
    // в него не заходит и на поле не падает.
    assert!(
        footer.iter().any(|text| text.contains("of")),
        "статический текст нижнего колонтитула на месте: {footer:?}"
    );
}
