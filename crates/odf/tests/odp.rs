//! Сквозные проверки ODP на фикстурах `test-fixtures/odf/odp/`.
//!
//! Фикстуры собирает `scripts/generate_odf_fixtures.ts`, поэтому значения в них
//! известны заранее: тест сверяет не «что-то разобралось», а конкретные слайды,
//! шейпы, текст и свойства прогонов.
#![cfg(feature = "odp")]

use std::io::{Cursor, Write};
use std::path::Path;

use doc_converter_core::Error as CoreError;
use doc_converter_core::ZipLimits;
use doc_converter_odf::error::OdfError;
use doc_converter_odf::manifest::{ODP_MIME, ODS_MIME};
use doc_converter_odf::odp::open;
use doc_converter_odf::plugin::OdpPlugin;
use doc_converter_pptx::model::{Presentation, Rgb, Run, ShapeKind};
use doc_converter_render::display_list::{DisplayList, DL_MAGIC};
use doc_converter_render::plugin::{FormatPlugin, RenderContext};
use zip::write::SimpleFileOptions;

/// Фикстура из общего корпуса ODF.
fn fixture(name: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/odf/odp");
    std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("фикстура {name}: {error}"))
}

/// Все прогоны текста презентации в порядке слайдов и шейпов.
fn all_runs(pres: &Presentation) -> Vec<&Run> {
    pres.slides
        .iter()
        .flat_map(|slide| slide.shapes.iter())
        .filter_map(|shape| match &shape.kind {
            ShapeKind::Text(text) => Some(&text.body),
            _ => None,
        })
        .flat_map(|body| body.paragraphs.iter())
        .flat_map(|paragraph| paragraph.runs.iter())
        .collect()
}

/// Прогон по точному тексту.
fn run<'a>(pres: &'a Presentation, text: &str) -> Option<&'a Run> {
    all_runs(pres).into_iter().find(|run| run.text == text)
}

/// Сравнение f32 без `float_cmp`: кегли приходят из `fo:font-size` и равны
/// ровно (40/24/18/14/11), но сравнение с допуском не зависит от округления.
fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.001
}

#[test]
fn simple_fixture_has_two_slides_text_and_picture() {
    let pres = open(fixture("simple.odp")).expect("simple.odp разбирается");

    assert_eq!(pres.slides.len(), 2);
    assert_eq!(pres.slides[0].name, "Slide 1");
    assert_eq!(pres.slides[1].name, "Slide 2");

    // Размер слайда приходит из `style:page-layout` мастер-страницы: 28×15.75 см.
    assert_eq!(pres.size.width, 10_080_000);
    assert_eq!(pres.size.height, 5_670_000);

    // Заголовок первого слайда — текст с кеглем 32 pt из `T_title`.
    let title = run(&pres, "Slides in ODF").expect("заголовок первого слайда");
    assert!(title.props.size.is_some_and(|size| approx(size, 32.0)));
    assert!(title.props.bold);

    // Картинка: `draw:frame` с `draw:image` стал picture-шейпом, байты — в `images`.
    assert_eq!(pres.images.len(), 1);
    assert_eq!(pres.images[0].name, "Pictures/pixel.png");
    assert_eq!(pres.images[0].mime, "image/png");
    assert!(!pres.images[0].bytes.is_empty());
    let picture = pres.slides[0]
        .shapes
        .iter()
        .find(|shape| matches!(shape.kind, ShapeKind::Picture { image_id: Some(0) }))
        .expect("на первом слайде есть картинка");
    assert_eq!(picture.name, "Shape 4");
}

#[test]
fn text_styles_fixture_keeps_sizes_and_colors() {
    let pres = open(fixture("text_styles.odp")).expect("text_styles.odp разбирается");
    assert_eq!(pres.slides.len(), 1);
    assert_eq!(pres.slides[0].name, "Text styles");

    // Заголовок 40 pt, жирный, тёмно-синий.
    let title = run(&pres, "Text styles").expect("заголовок");
    assert!(title.props.size.is_some_and(|size| approx(size, 40.0)));
    assert!(title.props.bold);
    assert_eq!(title.props.color, Some(Rgb::new(0x1F, 0x38, 0x64)));

    // Подзаголовок 24 pt, курсив, красный.
    let sub = run(&pres, "Subtitle 24 pt").expect("подзаголовок");
    assert!(sub.props.size.is_some_and(|size| approx(size, 24.0)));
    assert!(sub.props.italic);
    assert_eq!(sub.props.color, Some(Rgb::new(0xC0, 0x00, 0x00)));

    // Мелкий серый текст 11 pt.
    let small = run(&pres, "Small 11 pt grey text.").expect("мелкий текст");
    assert!(small.props.size.is_some_and(|size| approx(size, 11.0)));
    assert_eq!(small.props.color, Some(Rgb::new(0x59, 0x59, 0x59)));

    // Акцентный прогон 14 pt оранжевый.
    let accent = run(&pres, "accent 14 pt").expect("акцентный прогон");
    assert!(accent.props.size.is_some_and(|size| approx(size, 14.0)));
    assert_eq!(accent.props.color, Some(Rgb::new(0xC5, 0x5A, 0x11)));

    // Каскад: «bold» в смешанном абзаце наследует 18 pt из `P_body`.
    let bold = run(&pres, "bold").expect("жирный прогон");
    assert!(bold.props.bold);
    assert!(bold.props.size.is_some_and(|size| approx(size, 18.0)));
}

#[test]
fn plugin_reports_two_pages_and_builds_a_frame() {
    let model = OdpPlugin
        .parse(&fixture("simple.odp"), &ZipLimits::default())
        .expect("ODP разбирается плагином");
    assert_eq!(model.page_count(), 2);

    let ctx = RenderContext {
        scale: 1.0,
        viewport_width: 800,
        viewport_height: 600,
    };
    let bytes = model.build_display_list(&ctx).expect("кадр собран");
    // DisplayList — единственный контракт кадра: магия заголовка на диске
    // little-endian, поэтому сравнивается u32, а не строка.
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

/// Минимальный `content.xml` с одним пустым слайдом.
const CONTENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" office:version="1.2">
  <office:body><office:presentation>
    <draw:page draw:name="Only"/>
  </office:presentation></office:body>
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
fn garbage_is_a_core_error() {
    let error = open(b"not a zip".to_vec()).expect_err("не ZIP");
    assert!(matches!(error, OdfError::Core(_)), "{error:?}");
}

#[test]
fn missing_content_is_a_missing_part() {
    let manifest = manifest(ODP_MIME);
    let bytes = package(
        &[("META-INF/manifest.xml", manifest.as_bytes())],
        Some(ODP_MIME),
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

    let error = open(bytes).expect_err("ODS в срезе ODP не поддержан");
    assert!(
        matches!(&error, OdfError::Unsupported(message) if message.contains("ods")),
        "{error:?}"
    );
}
