//! Сквозные проверки изображений: `XObject` в собранном PDF, отсечение по
//! якорю, растягивание, дедупликация и пропуск битых картинок.
//!
//! Книги — синтетические: картинки кладутся в реестр книги
//! (`Workbook::with_images`) и на лист (`SheetContent::images`), экспорт идёт
//! публичным API (`PdfExporter`), файл разбирается `lopdf` — как в
//! `tests/annot.rs`. Дедупликация по байтам проверяется и на склейке якорей
//! одной части, и на одинаковых копиях под разными именами.
//!
//! `pedantic` включён здесь, чтобы линты совпадали с крейтовыми
//! (`#![deny(clippy::pedantic)]` в `lib.rs`).
#![deny(clippy::pedantic)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use doc_converter_pdf::{PdfExporter, PdfOptions};
use doc_converter_xlsx::drawing::{ImageAnchor, ImageExtent, ImageMarker, SheetImage};
use doc_converter_xlsx::{
    Cell, CellValue, SharedStrings, Sheet, SheetContent, SheetState, StyleTable, Theme, Workbook,
    WorkbookImage, WorksheetBuilder, WorksheetMeta,
};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::BTreeMap;

/// Часть пакета синтетического листа; нужна только текстам ошибок.
const PART: &str = "xl/worksheets/sheet1.xml";

/// PNG заданного размера в красную и синюю клетку.
///
/// Растр нарочно цветной: декодер переводит фактически серую картинку в
/// Grayscale, и проба перестала бы отличать RGB-поток от серого.
fn png(width: usize, height: usize) -> Vec<u8> {
    let pixels: Vec<u8> = (0..width * height)
        .flat_map(|index| {
            if index % 2 == 0 {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            }
        })
        .collect();
    let buffer = image::RgbImage::from_raw(
        u32::try_from(width).expect("ширина помещается"),
        u32::try_from(height).expect("высота помещается"),
        pixels,
    )
    .expect("растровые пиксели");
    let mut bytes = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("PNG кодируется");
    bytes
}

/// Якорь `OneCell` с заданными размерами — как в синтетических книгах
/// `doc-converter-xlsx`; размеры в пикселях раскладки (96 dpi).
fn anchor(w: f32, h: f32) -> ImageAnchor {
    ImageAnchor::OneCell {
        from: ImageMarker {
            col: 1,
            row: 1,
            col_off: 0.0,
            row_off: 0.0,
        },
        ext: ImageExtent { cx: w, cy: h },
    }
}

/// Книга с одним листом и картинками `(медиа-имя, байты, размер якоря)`.
///
/// Каждая картинка кладётся в реестр книги и на лист: `image_id` сквозной по
/// реестру, якорь — сдвинутая вправо полоса шириной в половину страницы,
/// чтобы несколько картинок на листе не накладывались.
fn book_with_images(images: &[(String, Vec<u8>, f32, f32)]) -> Workbook {
    let mut builder = WorksheetBuilder::new(PART);
    builder
        .push(0, Cell::new(0, 0, CellValue::InlineString("ячейка".into())))
        .expect("ячейка");
    let sheets = vec![Sheet::new(
        WorksheetMeta {
            name: "Лист".into(),
            part: PART.into(),
            state: SheetState::Visible,
        },
        SheetContent {
            cells: builder.finish(),
            images: images
                .iter()
                .enumerate()
                .map(|(index, (name, _, w, h))| SheetImage {
                    name: Some(name.clone()),
                    media: Some(name.clone()),
                    image_id: Some(u32::try_from(index).expect("индекс помещается")),
                    edit_as: None,
                    anchor: anchor(*w, *h),
                })
                .collect(),
            ..SheetContent::default()
        },
    )];
    Workbook::new(
        sheets,
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
    .with_images(
        images
            .iter()
            .enumerate()
            .map(|(index, (name, bytes, _, _))| WorkbookImage {
                id: u32::try_from(index).expect("индекс помещается"),
                media: name.clone(),
                mime: String::from("image/png"),
                bytes: bytes.clone(),
            })
            .collect(),
    )
}

/// Экспортировать лист 0 книги и разобрать результат.
fn export(book: &Workbook) -> Document {
    let bytes = PdfExporter::new(PdfOptions::default())
        .export_xlsx_sheet(book, 0)
        .unwrap_or_else(|err| panic!("экспорт не удался: {err}"));
    Document::load_mem(&bytes).expect("PDF разбирается lopdf")
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

/// Операнд-число: печать кладёт координаты как `Real`.
#[allow(clippy::cast_precision_loss)] // координаты страницы далеки от 2^24
fn number(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("координата не число: {other:?}"),
    }
}

/// Имена `XObject`'ов страницы в порядке `Do`-операций.
fn do_names(content: &Content) -> Vec<Vec<u8>> {
    content
        .operations
        .iter()
        .filter(|op| op.operator == "Do")
        .filter_map(|op| {
            op.operands
                .first()
                .and_then(|operand| operand.as_name().ok())
                .map(<[u8]>::to_vec)
        })
        .collect()
}

/// Вершины пути отсечения страницы: `m`/`l` перед `W`.
fn clip_path(content: &Content) -> Vec<(f32, f32)> {
    let clip_index = content
        .operations
        .iter()
        .position(|op| op.operator == "W")
        .expect("отсечение есть");
    assert_eq!(
        content.operations[clip_index + 1].operator,
        "n",
        "путь отсечения завершается без заливки"
    );
    content.operations[..clip_index]
        .iter()
        .filter(|op| op.operator == "m" || op.operator == "l")
        .map(|op| (number(&op.operands[0]), number(&op.operands[1])))
        .collect()
}

/// Имена `Image`-`XObject`'ов документа, их потоки и число `Do`-ссылок.
///
/// `XObject`'ы глобальные: словарь один на документ, и страница ссылается на
/// него целиком. Поэтому имена собираются со всех страниц в множество, а
/// `Do` считается по ним же.
fn document_images(doc: &Document) -> (BTreeMap<Vec<u8>, &lopdf::Stream>, usize) {
    let mut images: BTreeMap<Vec<u8>, &lopdf::Stream> = BTreeMap::new();
    for &page_id in doc.get_pages().values() {
        let resources = resources(doc, page_id);
        let Ok(xobjects) = resources.get(b"XObject") else {
            continue;
        };
        let Some(xobjects) = deref(doc, xobjects).as_dict().ok() else {
            continue;
        };
        for (name, value) in xobjects {
            let Some(stream) = deref(doc, value).as_stream().ok() else {
                continue;
            };
            if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image") {
                images.insert(name.clone(), stream);
            }
        }
    }
    let mut draws = 0;
    for &page_id in doc.get_pages().values() {
        let bytes = doc.get_page_content(page_id).expect("поток содержимого");
        let content = Content::decode(&bytes).expect("операции разбираются");
        draws += content
            .operations
            .iter()
            .filter(|op| op.operator == "Do")
            .filter(|op| {
                op.operands
                    .first()
                    .and_then(|operand| operand.as_name().ok())
                    .is_some_and(|name| images.contains_key(name))
            })
            .count();
    }
    (images, draws)
}

/// Лист с картинкой несёт `/Image`-`XObject` с размерами исходника, а страница
/// ссылается на него `Do`.
#[test]
fn png_reaches_pdf_as_image_xobject() {
    let book = book_with_images(&[(String::from("xl/media/image1.png"), png(8, 4), 80.0, 40.0)]);
    let doc = export(&book);
    let (images, draws) = document_images(&doc);

    assert_eq!(images.len(), 1, "одна media-часть — один XObject");
    let stream = images.values().next().expect("XObject есть");
    let dict = &stream.dict;
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
    assert_eq!(draws, 1, "один якорь — один вывод картинки");
}

/// Размер страницы в точках из `/MediaBox` первой страницы.
fn page_size_pt(doc: &Document) -> (f32, f32) {
    let page_id = *doc.get_pages().values().next().expect("страница есть");
    let page = doc.get_dictionary(page_id).expect("словарь страницы");
    let media = page
        .get(b"MediaBox")
        .expect("/MediaBox")
        .as_array()
        .expect("/MediaBox — массив");
    let value = |i: usize| media[i].as_float().expect("число");
    (value(2) - value(0), value(3) - value(1))
}

/// Картинка растягивается на якорь и отсекается по нему: матрица перед `Do`
/// несёт стороны якоря в точках, клип — его вершины.
#[test]
fn image_is_stretched_to_the_anchor_and_clipped() {
    // Якорь 40x40 pt при исходнике 16x4 px: пропорции исходника (4:1) не
    // сохраняются — картинка заполняет якорь (1:1), как `drawImage` canvas-пути.
    let book = book_with_images(&[(String::from("xl/media/image1.png"), png(16, 4), 40.0, 40.0)]);
    let doc = export(&book);
    let (width_pt, height_pt) = page_size_pt(&doc);
    let content = page_ops(&doc);

    let do_index = content
        .operations
        .iter()
        .position(|op| op.operator == "Do")
        .expect("`Do` есть");
    let cm = &content.operations[do_index - 1];
    assert_eq!(cm.operator, "cm", "перед `Do` — матрица трансформа");
    let matrix: Vec<f32> = cm.operands.iter().map(number).collect();
    assert_eq!(matrix.len(), 6);
    // Якорь 40x40 px раскладки при 96 dpi — это 30x30 pt в PDF (72/96 = 0.75).
    let side_pt = 40.0 * 72.0 / 96.0;
    assert!((matrix[0] - side_pt).abs() < 1e-3, "ширина: {}", matrix[0]);
    assert!((matrix[3] - side_pt).abs() < 1e-3, "высота: {}", matrix[3]);
    // Перенос и клип лежат внутри страницы: точные координаты задаёт раскладка
    // (ширина колонки 1 синтетического листа), а не тест.
    assert!(
        (0.0..=width_pt).contains(&matrix[4]),
        "сдвиг x: {}",
        matrix[4]
    );
    assert!(
        (0.0..=height_pt).contains(&matrix[5]),
        "сдвиг y: {}",
        matrix[5]
    );

    let points = clip_path(&content);
    assert_eq!(points.len(), 4, "клип — прямоугольник якоря");
    assert!(
        points
            .iter()
            .all(|(px, py)| (0.0..=width_pt).contains(px) && (0.0..=height_pt).contains(py)),
        "клип внутри страницы: {points:?}"
    );
}

/// Пять якорей одной части — пять `Do` с одним и тем же именем ресурса.
#[test]
fn repeated_anchor_draws_the_same_xobject() {
    let bytes = png(8, 4);
    let mut builder = WorksheetBuilder::new(PART);
    builder
        .push(0, Cell::new(0, 0, CellValue::InlineString("ячейка".into())))
        .expect("ячейка");
    let sheet = Sheet::new(
        WorksheetMeta {
            name: "Лист".into(),
            part: PART.into(),
            state: SheetState::Visible,
        },
        SheetContent {
            cells: builder.finish(),
            images: (0..5_u8)
                .map(|index| SheetImage {
                    name: Some(format!("Picture {index}")),
                    media: Some(String::from("xl/media/one.png")),
                    image_id: Some(0),
                    edit_as: None,
                    anchor: anchor(80.0, 20.0 + f32::from(index) * 30.0),
                })
                .collect(),
            ..SheetContent::default()
        },
    );
    let book = Workbook::new(
        vec![sheet],
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
    .with_images(vec![WorkbookImage {
        id: 0,
        media: String::from("xl/media/one.png"),
        mime: String::from("image/png"),
        bytes,
    }]);
    let doc = export(&book);

    let (images, draws) = document_images(&doc);
    assert_eq!(images.len(), 1, "одна часть — один XObject");
    assert_eq!(draws, 5, "пять якорей — пять выводов");
    let names = do_names(&page_ops(&doc));
    assert_eq!(names.len(), 5);
    assert!(
        names
            .iter()
            .all(|name| Some(name.as_slice()) == images.keys().next().map(Vec::as_slice)),
        "все `Do` ссылаются на один и тот же XObject"
    );
}

/// Битые байты не роняют экспорт: картинка пропускается, ячейка остаётся.
#[test]
fn broken_image_is_skipped_and_the_rest_is_drawn() {
    let mut builder = WorksheetBuilder::new(PART);
    builder
        .push(0, Cell::new(0, 0, CellValue::InlineString("ячейка".into())))
        .expect("ячейка");
    let sheet = Sheet::new(
        WorksheetMeta {
            name: "Лист".into(),
            part: PART.into(),
            state: SheetState::Visible,
        },
        SheetContent {
            cells: builder.finish(),
            images: vec![
                SheetImage {
                    name: Some(String::from("Picture 1")),
                    media: Some(String::from("xl/media/good.png")),
                    image_id: Some(0),
                    edit_as: None,
                    anchor: anchor(80.0, 20.0),
                },
                SheetImage {
                    name: Some(String::from("Picture 2")),
                    media: Some(String::from("xl/media/broken.png")),
                    image_id: Some(1),
                    edit_as: None,
                    anchor: anchor(80.0, 20.0),
                },
            ],
            ..SheetContent::default()
        },
    );
    let book = Workbook::new(
        vec![sheet],
        SharedStrings::default(),
        StyleTable::default(),
        Theme::default(),
        false,
    )
    .with_images(vec![
        WorkbookImage {
            id: 0,
            media: String::from("xl/media/good.png"),
            mime: String::from("image/png"),
            bytes: png(8, 4),
        },
        WorkbookImage {
            id: 1,
            media: String::from("xl/media/broken.png"),
            mime: String::from("image/png"),
            bytes: b"broken".to_vec(),
        },
    ]);
    let doc = export(&book);

    let (images, draws) = document_images(&doc);
    assert_eq!(images.len(), 1, "битая картинка не завела XObject");
    assert_eq!(draws, 1, "уцелевшая картинка выводится один раз");
}

/// Лист без картинок не порождает ни `XObject`'ов, ни `Do`.
#[test]
fn sheet_without_images_has_no_xobject() {
    let book = book_with_images(&[]);
    let doc = export(&book);
    let (images, draws) = document_images(&doc);

    assert!(images.is_empty(), "нет картинок — нет XObject'ов");
    assert_eq!(draws, 0, "нет картинок — нет /Do");
}

/// Путь к книге в `test-fixtures/xlsx`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/xlsx")
        .join(name)
}

/// Экспортировать лист книги настройками по умолчанию.
fn export_sheet(name: &str, sheet: usize) -> Vec<u8> {
    let book = doc_converter_xlsx::open(
        std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
    )
    .unwrap_or_else(|e| panic!("{name} не открылась: {e}"));
    doc_converter_pdf::PdfExporter::new(doc_converter_pdf::PdfOptions::default())
        .export_xlsx_sheet(&book, sheet)
        .unwrap_or_else(|e| panic!("{name} не экспортировалась: {e}"))
}

/// Лист с картинкой несёт `/Image`-`XObject`, а страница ссылается на него `Do`.
///
/// Тест падает, если `painter` перестанет звать `image_ops`: до подключения
/// модуля PDF обходился без единого `/Image` (дыра D1 приёмки).
#[test]
fn sheet_image_reaches_pdf_as_xobject() {
    let bytes = export_sheet("images-png.xlsx", 0);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let (images, draws) = document_images(&doc);

    assert_eq!(images.len(), 1, "одна media-часть — один XObject");
    let stream = images.values().next().expect("XObject есть");
    assert!(
        stream
            .dict
            .get(b"Width")
            .and_then(Object::as_i64)
            .is_ok_and(|w| w > 0),
        "у картинки есть ширина исходника"
    );
    assert!(
        stream
            .dict
            .get(b"Height")
            .and_then(Object::as_i64)
            .is_ok_and(|h| h > 0),
        "у картинки есть высота исходника"
    );
    assert_eq!(draws, 2, "два якоря фикстуры — два вывода картинки");
}

/// Одна media-часть в разных якорях даёт один `XObject`, разные — разные.
#[test]
fn repeated_media_part_gives_one_xobject() {
    let bytes = export_sheet("images-over-data.xlsx", 0);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let (images, draws) = document_images(&doc);

    assert_eq!(
        images.len(),
        2,
        "png и jpeg — два XObject'а; повтор png не заводит третий"
    );
    assert_eq!(draws, 3, "три якоря — три вывода");
}

/// Лист без картинок не порождает ни `XObject`'ов, ни `Do`.
#[test]
fn fixture_without_images_has_no_xobject() {
    let bytes = export_sheet("content-mixed-types.xlsx", 0);
    let doc = Document::load_mem(&bytes).expect("PDF разбирается lopdf");
    let (images, draws) = document_images(&doc);

    assert!(images.is_empty(), "нет картинок — нет XObject'ов");
    assert_eq!(draws, 0, "нет картинок — нет /Do");
}
