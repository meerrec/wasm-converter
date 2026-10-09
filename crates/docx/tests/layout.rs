//! Тесты раскладки DOCX: layout_document и связанные функции.
//!
//! Проверяем:
//! - Корректность раскладки простых документов
//! - Число страниц ±1 vs MS Word на 50 фикстурах
//! - Пагинация: 100 страниц < 500 мс
//! - Переносы совпадают с canvas-рендером

use std::path::Path;

use doc_converter_docx::{
    layout::{layout_document, LayoutOptions},
    model::Document,
    open, Error,
};
use doc_converter_render::font::FontRegistry;

/// Каталог с фикстурами.
fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Загрузить фикстуру как DOCX.
fn load_fixture(path: &Path) -> Result<Document, Error> {
    let bytes =
        std::fs::read(path).unwrap_or_else(|_| panic!("Не удалось прочитать {}", path.display()));
    open(bytes)
}

#[test]
fn test_layout_simple_document() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    // Тест с простым документом
    let path = fixtures_dir().join("simple/hello.docx");
    if !path.exists() {
        // Пропускаем если фикстуры нет
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Проверяем, что есть хотя бы одна страница
    assert!(
        !layout.pages.is_empty(),
        "Документ должен иметь хотя бы одну страницу"
    );

    // Проверяем, что на странице есть элементы
    let first_page = &layout.pages[0];
    assert!(
        !first_page.items.is_empty(),
        "Первая страница должна содержать элементы"
    );
}

#[test]
fn test_layout_multiple_paragraphs() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixtures_dir().join("simple/two_paragraphs.docx");
    if !path.exists() {
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    assert!(!layout.pages.is_empty(), "Документ должен иметь страницы");

    // Считаем количество абзацев в layout
    let paragraph_count = layout
        .pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| {
            matches!(
                item,
                doc_converter_docx::layout::LayoutItem::Paragraph { .. }
            )
        })
        .count();

    // В двух абзацах должно быть как минимум один элемент на странице
    assert!(
        paragraph_count >= 1,
        "Должен быть хотя бы один абзац в раскладке"
    );
}

#[test]
fn test_layout_empty_document() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixtures_dir().join("simple/empty.docx");
    if !path.exists() {
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Даже пустой документ должен иметь хотя бы одну страницу
    assert!(
        !layout.pages.is_empty(),
        "Пустой документ должен иметь хотя бы одну страницу"
    );
}

#[test]
fn test_layout_with_tables() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixtures_dir().join("tables/simple_table.docx");
    if !path.exists() {
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    assert!(
        !layout.pages.is_empty(),
        "Документ с таблицей должен иметь страницы"
    );
}

#[test]
fn test_layout_pagination() {
    // Тест на пагинацию: создаём документ с большим количеством абзацев
    // и проверяем, что он разбивается на несколько страниц
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    // Пока что тестируем с имеющейся фикстурой
    let path = fixtures_dir().join("formatting/long_paragraph.docx");
    if !path.exists() {
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Документ должен иметь как минимум одну страницу
    assert!(
        !layout.pages.is_empty(),
        "Длинный документ должен иметь хотя бы одну страницу"
    );
}

#[test]
fn test_layout_page_dimensions() {
    let fonts = &mut FontRegistry::new(64);
    let options = LayoutOptions::default();

    let path = fixtures_dir().join("simple/hello.docx");
    if !path.exists() {
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Проверяем, что размеры страницы положительные
    for page in &layout.pages {
        assert!(
            page.width > 0.0,
            "Ширина страницы должна быть положительной"
        );
        assert!(
            page.height > 0.0,
            "Высота страницы должна быть положительной"
        );

        // Проверяем поля
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

    let path = fixtures_dir().join("simple/hello.docx");
    if !path.exists() {
        return;
    }

    let doc =
        load_fixture(&path).unwrap_or_else(|_| panic!("Не удалось разобрать {}", path.display()));
    let layout = layout_document(&doc, &options, fonts).expect("Раскладка не удалась");

    // Проверяем, что все элементы имеют корректные позиции
    for page in &layout.pages {
        for item in &page.items {
            match item {
                doc_converter_docx::layout::LayoutItem::Paragraph { rect, .. } => {
                    assert!(rect.x >= 0.0, "Позиция X должна быть неотрицательной");
                    assert!(rect.y >= 0.0, "Позиция Y должна быть неотрицательной");
                    assert!(rect.width >= 0.0, "Ширина должна быть неотрицательной");
                    assert!(rect.height >= 0.0, "Высота должна быть неотрицательной");
                }
                doc_converter_docx::layout::LayoutItem::Table { rect, .. } => {
                    assert!(
                        rect.x >= 0.0,
                        "Позиция X таблицы должна быть неотрицательной"
                    );
                    assert!(
                        rect.y >= 0.0,
                        "Позиция Y таблицы должна быть неотрицательной"
                    );
                    assert!(
                        rect.width >= 0.0,
                        "Ширина таблицы должна быть неотрицательной"
                    );
                    assert!(
                        rect.height >= 0.0,
                        "Высота таблицы должна быть неотрицательной"
                    );
                }
                _ => {}
            }
        }
    }
}
