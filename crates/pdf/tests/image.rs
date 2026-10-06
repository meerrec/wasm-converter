//! Сквозные проверки модуля изображений: `XObject` в собранном PDF, отсечение
//! по якорю и пропуск битых картинок.
//!
//! `pedantic` включён здесь, чтобы линты совпадали с крейтовыми
//! (`#![deny(clippy::pedantic)]` в `lib.rs`).
#![deny(clippy::pedantic)]

use doc_converter_pdf::image::{place, place_all, Batch, Placement};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};
use printpdf::{
    Mm, Op, OutputImageFormat, PdfDocument, PdfPage, PdfSaveOptions, Pt, RawImage, RawImageData,
    RawImageFormat, Rect,
};

/// Сторона страницы в точках.
const PAGE_PT: f32 = 200.0;

/// PNG заданного размера в красную и синюю клетку.
///
/// Растр нарочно цветной: printpdf при сборке PDF переводит фактически серую
/// картинку в Grayscale (`auto_optimize`), и проба перестала бы отличать
/// RGB-поток от серого.
fn png(width: usize, height: usize) -> Vec<u8> {
    let pixels = (0..width * height)
        .flat_map(|index| {
            if index % 2 == 0 {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            }
        })
        .collect();
    let raw = RawImage {
        pixels: RawImageData::U8(pixels),
        width,
        height,
        data_format: RawImageFormat::RGB8,
        tag: Vec::new(),
    };
    raw.encode_to_bytes(&[OutputImageFormat::Png])
        .expect("PNG кодируется")
        .0
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect {
        x: Pt(x),
        y: Pt(y),
        width: Pt(w),
        height: Pt(h),
    }
}

/// PDF без сжатия потоков: содержимое страницы читается lopdf как есть.
fn save(doc: &PdfDocument) -> Vec<u8> {
    let options = PdfSaveOptions {
        optimize: false,
        ..PdfSaveOptions::default()
    };
    doc.save(&options, &mut Vec::new())
}

/// Страница размером [`PAGE_PT`] с операциями `ops`.
fn page(ops: Vec<Op>) -> PdfPage {
    PdfPage::new(Mm::from(Pt(PAGE_PT)), Mm::from(Pt(PAGE_PT)), ops)
}

/// Разыменовывает косвенные ссылки lopdf: в PDF почти всё — косвенные объекты.
fn deref<'a>(doc: &'a Document, object: &'a Object) -> &'a Object {
    let mut current = object;
    for _ in 0..8 {
        let Object::Reference(id) = current else {
            return current;
        };
        current = doc.get_object(*id).expect("ссылка разрешается");
    }
    panic!("цепочка ссылок не зациклена");
}

/// Словарь ресурсов страницы — прямой или косвенный.
fn resources(doc: &Document, page_id: ObjectId) -> &Dictionary {
    let (direct, indirect) = doc.get_page_resources(page_id).expect("ресурсы страницы");
    match (direct, indirect.first()) {
        (Some(dict), _) => dict,
        (None, Some(id)) => doc.get_dictionary(*id).expect("словарь ресурсов"),
        (None, None) => panic!("у страницы нет ресурсов"),
    }
}

/// Операции единственной страницы документа.
fn page_ops(doc: &Document) -> Content {
    let page_id = *doc.get_pages().values().next().expect("страница есть");
    let bytes = doc.get_page_content(page_id).expect("поток содержимого");
    Content::decode(&bytes).expect("операции разбираются")
}

/// Операнд-число: printpdf пишет координаты как `Real`.
#[allow(clippy::cast_precision_loss)] // координаты страницы далеки от 2^24
fn number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("координата не число: {other:?}"),
    }
}

/// Именованный `XObject`-поток страницы и имя, под которым он лежит в ресурсах.
fn image_xobject(doc: &Document, page_id: ObjectId) -> (Vec<u8>, &lopdf::Stream) {
    let resources = resources(doc, page_id);
    let xobjects = deref(doc, resources.get(b"XObject").expect("словарь XObject"))
        .as_dict()
        .expect("XObject — словарь");
    assert_eq!(xobjects.len(), 1, "одна картинка — один XObject");
    let (name, value) = xobjects.iter().next().expect("запись есть");
    let stream = deref(doc, value).as_stream().expect("XObject — поток");
    (name.clone(), stream)
}

#[test]
fn png_becomes_image_xobject_with_source_size() {
    let bytes = png(8, 4);
    let mut doc = PdfDocument::new("image-test");
    let ops = place(
        &mut doc.resources,
        "image/png",
        &bytes,
        &rect(10.0, 20.0, 80.0, 40.0),
    )
    .expect("картинка размещается");
    doc.with_pages(vec![page(ops)]);

    let parsed = Document::load_mem(&save(&doc)).expect("PDF разбирается lopdf");
    let page_id = *parsed.get_pages().values().next().expect("страница есть");
    let (name, stream) = image_xobject(&parsed, page_id);

    let dict = &stream.dict;
    assert_eq!(
        dict.get(b"Subtype").and_then(Object::as_name).ok(),
        Some(b"Image" as &[u8]),
        "XObject должен быть изображением"
    );
    assert_eq!(
        dict.get(b"Width").and_then(Object::as_i64).ok(),
        Some(8),
        "ширина исходника в пикселях"
    );
    assert_eq!(
        dict.get(b"Height").and_then(Object::as_i64).ok(),
        Some(4),
        "высота исходника в пикселях"
    );
    assert_eq!(
        dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
        Some(b"DeviceRGB" as &[u8])
    );

    let content = page_ops(&parsed);
    let do_op = content
        .operations
        .iter()
        .find(|op| op.operator == "Do")
        .expect("картинка выводится на страницу");
    assert_eq!(
        do_op.operands.first().and_then(|o| o.as_name().ok()),
        Some(name.as_slice()),
        "`Do` ссылается на имя из ресурсов"
    );
}

#[test]
fn placement_matrix_and_clip_match_the_anchor() {
    let bytes = png(8, 4);
    let mut doc = PdfDocument::new("image-test");
    let ops = place(
        &mut doc.resources,
        "image/png",
        &bytes,
        &rect(10.0, 20.0, 80.0, 40.0),
    )
    .expect("картинка размещается");
    doc.with_pages(vec![page(ops)]);

    let parsed = Document::load_mem(&save(&doc)).expect("PDF разбирается lopdf");
    let content = page_ops(&parsed);
    let operations = &content.operations;

    let do_index = operations
        .iter()
        .position(|op| op.operator == "Do")
        .expect("`Do` есть");
    let cm = &operations[do_index - 1];
    assert_eq!(cm.operator, "cm", "перед `Do` — матрица трансформа");
    let matrix: Vec<f32> = cm.operands.iter().map(number).collect();
    assert_eq!(matrix.len(), 6);
    assert!((matrix[0] - 80.0).abs() < 1e-3, "ширина: {}", matrix[0]);
    assert!((matrix[3] - 40.0).abs() < 1e-3, "высота: {}", matrix[3]);
    assert!((matrix[4] - 10.0).abs() < 1e-3, "x: {}", matrix[4]);
    assert!((matrix[5] - 20.0).abs() < 1e-3, "y: {}", matrix[5]);

    let clip_index = operations
        .iter()
        .position(|op| op.operator == "W")
        .expect("отсечение есть");
    assert!(clip_index < do_index, "отсечение ставится до картинки");
    assert_eq!(operations[clip_index + 1].operator, "n");
    let path: Vec<(f32, f32)> = operations[..clip_index]
        .iter()
        .filter(|op| op.operator == "m" || op.operator == "l")
        .map(|op| (number(&op.operands[0]), number(&op.operands[1])))
        .collect();
    assert_eq!(
        path,
        vec![(10.0, 20.0), (90.0, 20.0), (90.0, 60.0), (10.0, 60.0)],
        "путь отсечения — прямоугольник якоря"
    );
}

#[test]
fn broken_image_is_skipped_and_the_rest_is_drawn() {
    let good = png(8, 4);
    let mut doc = PdfDocument::new("image-test");
    let batch: Batch = place_all(
        &mut doc.resources,
        &[
            Placement {
                id: "xl/media/image1.png",
                mime: "image/png",
                bytes: &good,
                rect: rect(10.0, 20.0, 80.0, 40.0),
            },
            Placement {
                id: "xl/media/image2.png",
                mime: "image/png",
                bytes: b"broken",
                rect: rect(10.0, 90.0, 80.0, 40.0),
            },
        ],
    );
    assert_eq!(batch.skipped.len(), 1, "битая картинка пропущена");
    assert_eq!(batch.skipped[0].id, "xl/media/image2.png");
    doc.with_pages(vec![page(batch.ops)]);

    let parsed = Document::load_mem(&save(&doc)).expect("битая картинка не сломала PDF");
    let page_id = *parsed.get_pages().values().next().expect("страница есть");
    let (_, stream) = image_xobject(&parsed, page_id);
    assert_eq!(
        stream.dict.get(b"Width").and_then(Object::as_i64).ok(),
        Some(8)
    );
    let content = page_ops(&parsed);
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "Do")
            .count(),
        1,
        "уцелевшая картинка выводится один раз"
    );
}

#[test]
fn no_images_no_xobjects() {
    let doc = {
        let mut doc = PdfDocument::new("image-test");
        doc.with_pages(vec![page(Vec::new())]);
        doc
    };
    let parsed = Document::load_mem(&save(&doc)).expect("PDF разбирается lopdf");
    let content = page_ops(&parsed);
    assert!(content.operations.is_empty(), "страница без операций");
}
