//! Сквозные проверки ODT на фикстурах `test-fixtures/odf/odt/`.
//!
//! Фикстуры собирает `scripts/generate_odf_fixtures.ts`, поэтому значения в них
//! известны заранее: тест сверяет не «что-то разобралось», а конкретные
//! заголовки, прогоны, таблицу, картинку и раскладку страницы.
#![cfg(feature = "odt")]

use std::io::{Cursor, Write};
use std::path::Path;

use doc_converter_core::Error as CoreError;
use doc_converter_core::ZipLimits;
use doc_converter_docx::export::text::to_text;
use doc_converter_docx::layout::{layout_document, LayoutOptions};
use doc_converter_docx::{BlockItem, Inline, Orientation, Toggle, Twips};
use doc_converter_odf::error::OdfError;
use doc_converter_odf::manifest::{ODS_MIME, ODT_MIME};
use doc_converter_odf::odt::open;
use doc_converter_odf::plugin::OdtPlugin;
use doc_converter_render::display_list::{DisplayList, DL_MAGIC};
use doc_converter_render::font::{FontRegistry, DEFAULT_FONT_CACHE};
use doc_converter_render::plugin::{FormatPlugin, RenderContext};
use zip::write::SimpleFileOptions;

/// Фикстура из общего корпуса ODF.
fn fixture(name: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/odf/odt");
    std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("фикстура {name}: {error}"))
}

/// Все абзацы тела, включая абзацы ячеек таблиц.
fn paragraphs(doc: &doc_converter_docx::Document) -> Vec<&doc_converter_docx::Paragraph> {
    let mut out = Vec::new();
    for item in &doc.body.items {
        match item {
            BlockItem::Paragraph(paragraph) => out.push(paragraph),
            BlockItem::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        for item in &cell.items {
                            if let BlockItem::Paragraph(paragraph) = item {
                                out.push(paragraph);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn simple_fixture_has_headings_text_and_runs() {
    let doc = open(fixture("simple.odt")).expect("simple.odt разбирается");
    let text = to_text(&doc);

    // Заголовки двух уровней.
    assert!(text.contains("ODF text fixture"), "текст: {text}");
    assert!(text.contains("Second heading"), "текст: {text}");

    // Абзацы с прогонами bold/italic.
    assert!(text.contains("bold"), "текст: {text}");
    assert!(text.contains("italic"), "текст: {text}");

    // Гиперссылка: текст ссылки присутствует.
    assert!(text.contains("example.com/odf"), "текст: {text}");

    // Список: плоские элементы не теряют текст.
    assert!(text.contains("First item"), "текст: {text}");
    assert!(text.contains("Nested item"), "текст: {text}");

    // Значимые пробелы (`xml:space="preserve"` и `text:s`).
    assert!(text.contains("Two  spaces  survive"), "текст: {text}");
}

#[test]
fn heading_outline_levels_are_preserved() {
    let doc = open(fixture("simple.odt")).expect("simple.odt разбирается");
    let levels: Vec<u8> = paragraphs(&doc)
        .iter()
        .filter_map(|paragraph| paragraph.ppr.outline_lvl)
        .collect();

    // `text:outline-level="1"` → 0, `text:outline-level="2"` → 1 (как `w:outlineLvl`).
    assert_eq!(levels, vec![0, 1], "уровни заголовков: {levels:?}");
}

#[test]
fn runs_carry_bold_italic_and_hyperlink() {
    let doc = open(fixture("simple.odt")).expect("simple.odt разбирается");

    let mut bold = false;
    let mut italic = false;
    let mut hyperlink = false;
    for paragraph in paragraphs(&doc) {
        for inline in &paragraph.runs {
            match inline {
                Inline::Run(run) => {
                    bold |= run.rpr.b == Some(Toggle::On);
                    italic |= run.rpr.i == Some(Toggle::On);
                }
                Inline::Hyperlink(link) if !link.runs.is_empty() => hyperlink = true,
                _ => {}
            }
        }
    }
    assert!(bold, "есть полужирный прогон");
    assert!(italic, "есть курсивный прогон");
    assert!(hyperlink, "есть гиперссылка");
}

#[test]
fn table_has_merged_header_and_image_media() {
    let doc = open(fixture("table_image.odt")).expect("table_image.odt разбирается");

    let table = doc
        .body
        .items
        .iter()
        .find_map(|item| match item {
            BlockItem::Table(table) => Some(table),
            _ => None,
        })
        .expect("в теле есть таблица");

    // Две строки; шапка — одна ячейка на два столбца, вторая строка — две.
    assert_eq!(table.grid.len(), 2, "сетка из двух столбцов");
    assert_eq!(table.rows.len(), 2, "две строки");
    assert_eq!(table.rows[0].cells.len(), 1, "шапка объединена");
    assert_eq!(
        table.rows[0].cells[0].grid_span, 2,
        "объединение на два столбца"
    );
    assert_eq!(
        table.rows[1].cells.len(),
        2,
        "вторая строка без объединений"
    );

    // Текст ячеек присутствует.
    let text = to_text(&doc);
    assert!(text.contains("Merged header"), "текст: {text}");
    assert!(text.contains("Left"), "текст: {text}");
    assert!(text.contains("Right"), "текст: {text}");

    // Картинка прочитана в media.
    assert_eq!(doc.media.len(), 1, "одна медиачасть");
    assert_eq!(doc.media[0].part, "Pictures/pixel.png");
    assert_eq!(doc.media[0].mime, "image/png");
}

#[test]
fn page_styles_give_two_sections_and_layout() {
    let doc = open(fixture("page_styles.odt")).expect("page_styles.odt разбирается");

    // Две секции: A4 портрет (Standard) и A4 альбом (Landscape).
    assert_eq!(doc.body.sections.len(), 2);
    let portrait = &doc.body.sections[0];
    let landscape = &doc.body.sections[1];

    assert_eq!(portrait.orientation, Orientation::Portrait);
    assert_eq!(portrait.page_size.width, Twips::new(11906));
    assert_eq!(portrait.page_size.height, Twips::new(16838));
    // Поля портрета: верх/низ 2 см, слева/справа 2,5 см.
    assert_eq!(portrait.margins.top, Twips::new(1134));
    assert_eq!(portrait.margins.left, Twips::new(1417));

    assert_eq!(landscape.orientation, Orientation::Landscape);
    assert_eq!(landscape.page_size.width, Twips::new(16838));
    assert_eq!(landscape.page_size.height, Twips::new(11906));

    // Раскладка даёт больше одной страницы и обе ориентации.
    let mut fonts = FontRegistry::new(DEFAULT_FONT_CACHE);
    let layout =
        layout_document(&doc, &LayoutOptions::default(), &mut fonts).expect("раскладка собирается");
    assert!(layout.pages.len() > 1, "страниц: {}", layout.pages.len());
    assert!(
        layout.pages.iter().any(|page| page.width > page.height),
        "есть альбомная страница"
    );
    assert!(
        layout.pages.iter().any(|page| page.width < page.height),
        "есть книжная страница"
    );
}

#[test]
fn plugin_parses_and_builds_a_frame() {
    let model = OdtPlugin
        .parse(&fixture("simple.odt"), &ZipLimits::default())
        .expect("ODT разбирается плагином");
    assert!(model.page_count() > 0);

    let ctx = RenderContext {
        scale: 1.0,
        viewport_width: 800,
        viewport_height: 600,
    };
    let bytes = model.build_display_list(&ctx).expect("кадр собран");
    let magic = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    assert_eq!(magic, DL_MAGIC, "кадр несёт магию DLST");
    assert!(
        DisplayList::from_bytes(&bytes).is_ok(),
        "кадр разбирается читателем DisplayList"
    );
}

/// Минимальный валидный каталог ODF с указанным media-type корня.
fn manifest(media_type: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
  <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="{media_type}"/>
  <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#
    )
}

/// `content.xml` с одним абзацем текста.
const CONTENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
  <office:body><office:text><text:p>Hello ODT</text:p></office:text></office:body>
</office:document-content>"#;

/// Собрать ZIP: `mimetype` первой записью и без сжатия — как требует ODF.
fn package(parts: &[(&str, &[u8])], mimetype: Option<&str>) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        if let Some(mime) = mimetype {
            writer
                .start_file(
                    "mimetype",
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(mime.as_bytes()).unwrap();
        }
        for (name, data) in parts {
            writer
                .start_file(
                    *name,
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    buf
}

#[test]
fn minimal_package_opens() {
    let manifest = manifest(ODT_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        Some(ODT_MIME),
    );

    let doc = open(bytes).expect("собранный пакет разбирается");
    assert!(to_text(&doc).contains("Hello ODT"));
}

#[test]
fn garbage_is_a_core_error() {
    let error = open(b"not a zip".to_vec()).expect_err("не ZIP");
    assert!(matches!(error, OdfError::Core(_)), "{error:?}");
}

#[test]
fn missing_mimetype_is_malformed() {
    let manifest = manifest(ODT_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        None,
    );

    let error = open(bytes).expect_err("нет mimetype");
    assert!(
        matches!(&error, OdfError::Malformed { part, .. } if part == "mimetype"),
        "{error:?}"
    );
}

#[test]
fn wrong_mimetype_is_unsupported() {
    let manifest = manifest(ODS_MIME);
    let bytes = package(
        &[
            ("META-INF/manifest.xml", manifest.as_bytes()),
            ("content.xml", CONTENT.as_bytes()),
        ],
        Some(ODS_MIME),
    );

    let error = open(bytes).expect_err("ODS в этом срезе не поддержан");
    assert!(
        matches!(&error, OdfError::Unsupported(message) if message.contains("ods")),
        "{error:?}"
    );
}

#[test]
fn missing_manifest_is_malformed() {
    let bytes = package(&[("content.xml", CONTENT.as_bytes())], Some(ODT_MIME));

    let error = open(bytes).expect_err("нет каталога");
    assert!(
        matches!(&error, OdfError::Malformed { part, .. } if part == "META-INF/manifest.xml"),
        "{error:?}"
    );
}

#[test]
fn missing_content_is_a_missing_part() {
    let manifest = manifest(ODT_MIME);
    let bytes = package(
        &[("META-INF/manifest.xml", manifest.as_bytes())],
        Some(ODT_MIME),
    );

    let error = open(bytes).expect_err("нет content.xml");
    assert!(
        matches!(
            &error,
            OdfError::Core(CoreError::MissingPart(part)) if part == "content.xml"
        ),
        "{error:?}"
    );
}
