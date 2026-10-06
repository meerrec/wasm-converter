//! Картинки листа → изображения-`XObject` на страницах PDF.
//!
//! Из реестра книги приходят mime-тип, байты media-части и прямоугольник
//! якоря в точках; модуль декодирует байты в [`RawImage`], регистрирует его в
//! [`PdfResources::xobjects`] и отдаёт операции страницы — отсечение по
//! прямоугольнику и `Op::UseXobject` с матрицей под его размер.
//!
//! Форматы: PNG и JPEG. Их декодирует printpdf, если включены его фичи
//! `png`/`jpeg` (включены в `Cargo.toml` крейта). Остальные форматы Office
//! (gif, bmp, webp, tiff, svg) пропускаются с ошибкой
//! [`DecodeError::UnsupportedFormat`]: каждый декодер — это вес wasm-модуля,
//! а такие вставки в книгах редки. Пропуск не роняет экспорт листа: [`place`]
//! возвращает ошибку про одно изображение, а [`place_all`] собирает ошибки
//! всех в отчёт и отдаёт операции уцелевших.
//!
//! Картинка растягивается на весь прямоугольник якоря: так же её кладёт
//! canvas-путь (`drawImage` с явными `dw`/`dh`), и так устроен OOXML без
//! `a:srcRect` — умолчание `a:stretch` заполняет якорь, не сохраняя пропорций.
//! Отсечение по якорю (`W n`) остаётся: пиксели не имеют права выезжать за
//! него ни при каких обстоятельствах. Настоящая обрезка появится, когда
//! парсер начнёт читать `a:srcRect`: тогда вписывание станет обрезкой по
//! данным файла, а не по несовпадению пропорций.
//!
//! Повторно вставленная media-часть не заводит второй `XObject`: имя выводится
//! из отпечатка байтов, и [`register`] переиспользует уже собранное
//! изображение. Это и место, и процессор: книга с одной картинкой в сотне
//! ячеек декодирует её один раз.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use printpdf::{
    LinePoint, Op, PaintMode, PdfResources, Point, Polygon, PolygonRing, Pt, Px, RawImage, Rect,
    WindingOrder, XObject, XObjectId, XObjectTransform,
};

/// DPI, при котором первый масштаб из `XObjectTransform::get_ctms` —
/// тождественный (1 px = 1 pt). Дальше размер картинки на странице задают
/// только `scale_x`/`scale_y`, то есть растягивание на прямоугольник якоря.
const DPI: f32 = 72.0;

/// Ошибка размещения изображения.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// Формат медиачасти не поддержан.
    #[error("unsupported image format {mime}: only PNG and JPEG are supported")]
    UnsupportedFormat {
        /// Mime-тип из `[Content_Types].xml`.
        mime: String,
    },
    /// Байты не разобрал ни один встроенный декодер: битый файл или формат,
    /// для которого фичи printpdf не включены.
    #[error("cannot decode image: {0}")]
    Decode(String),
    /// У прямоугольника размещения неположительная сторона.
    #[error("image rectangle is degenerate: {width_pt} x {height_pt} pt")]
    DegenerateRect {
        /// Ширина прямоугольника в точках.
        width_pt: f32,
        /// Высота прямоугольника в точках.
        height_pt: f32,
    },
}

/// Декодирует байты media-части в [`RawImage`].
///
/// Формат определяется по содержимому (`RawImage::decode_from_bytes` зовёт
/// `guess_format`) — mime нужен только чтобы отсеять заведомо неподдержанные
/// `image/*` до разбора байтов и назвать формат в ошибке. Пустой mime и mime
/// не-картинки не отсеивают: часть могла приехать без `Content_Types`, и
/// последнее слово всё равно за содержимым.
///
/// # Errors
/// * [`DecodeError::UnsupportedFormat`] — mime начинается с `image/`, но это
///   не PNG и не JPEG (в том числе `image/gif` и `image/svg+xml`);
/// * [`DecodeError::Decode`] — байты не разобрал встроенный декодер.
pub fn decode(mime: &str, bytes: &[u8]) -> Result<RawImage, DecodeError> {
    let normalized = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized.starts_with("image/") && !is_supported(&normalized) {
        return Err(DecodeError::UnsupportedFormat {
            mime: mime.to_owned(),
        });
    }
    RawImage::decode_from_bytes(bytes, &mut Vec::new()).map_err(DecodeError::Decode)
}

/// Декодирует изображение, кладёт его в `resources` и собирает операции
/// страницы.
///
/// `rect` — прямоугольник в точках с началом координат в левом нижнем углу
/// страницы: то же, что отдаёт `RectPt::to_pdf`. Картинка растягивается на
/// него целиком и отсекается по нему же (подробности — [`placement_ops`]).
///
/// Повторный вызов с теми же байтами переиспользует `XObject` первого вызова.
///
/// # Errors
/// Те же, что у [`decode`], плюс [`DecodeError::DegenerateRect`] — проверяется
/// до декодирования, чтобы не засорять ресурсы `XObject`'ом без операций.
pub fn place(
    resources: &mut PdfResources,
    mime: &str,
    bytes: &[u8],
    rect: &Rect,
) -> Result<Vec<Op>, DecodeError> {
    check_rect(rect)?;
    let (id, width_px, height_px) = register(resources, mime, bytes)?;
    Ok(placement_ops(&id, width_px, height_px, rect))
}

/// Изображение к размещению на странице.
#[derive(Debug, Clone)]
pub struct Placement<'a> {
    /// Идентификатор media-части (`xl/media/image1.png`) — попадает в отчёт
    /// о пропуске, чтобы было видно, какую картинку потеряли. Дедупликация
    /// на него не опирается: ключ — байты ([`register`]), так что разные id
    /// с одинаковым содержимым дают один `XObject`.
    pub id: &'a str,
    /// Mime-тип из `[Content_Types].xml`.
    pub mime: &'a str,
    /// Байты media-части.
    pub bytes: &'a [u8],
    /// Прямоугольник в точках от левого нижнего угла страницы.
    pub rect: Rect,
}

/// Изображение, которое разместить не удалось.
#[derive(Debug)]
pub struct Skipped {
    /// Идентификатор media-части.
    pub id: String,
    /// Причина, по которой страница осталась без картинки.
    pub error: DecodeError,
}

/// Результат [`place_all`]: операции уцелевших изображений и пропуски.
#[derive(Debug, Default)]
pub struct Batch {
    /// Операции для страницы, в порядке переданных изображений.
    pub ops: Vec<Op>,
    /// Изображения, которые разместить не удалось.
    pub skipped: Vec<Skipped>,
}

/// Размещает изображения страницы, пропуская битые и неподдержанные.
///
/// Порядок операций — порядок `placements`: вызывающий сортирует их в порядке
/// наложения листа (`Sheet.images`). Ошибка одного изображения не мешает
/// остальным: причина ложится в [`Batch::skipped`], операции уцелевших — в
/// [`Batch::ops`], так что одно битое вложение не роняет экспорт листа.
#[must_use]
pub fn place_all(resources: &mut PdfResources, placements: &[Placement<'_>]) -> Batch {
    let mut batch = Batch::default();
    for placement in placements {
        match place(resources, placement.mime, placement.bytes, &placement.rect) {
            Ok(ops) => batch.ops.extend(ops),
            Err(error) => batch.skipped.push(Skipped {
                id: placement.id.to_owned(),
                error,
            }),
        }
    }
    batch
}

/// PNG и JPEG в написаниях, которые встречаются в книгах Office.
fn is_supported(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/x-png" | "image/jpeg" | "image/jpg" | "image/pjpeg"
    )
}

/// Проверяет, что прямоугольник не вырожден.
///
/// Сравнение `> 0.0` отсеивает и `NaN`: с ним масштаб и матрица в content
/// stream стали бы нечисловыми.
fn check_rect(rect: &Rect) -> Result<(), DecodeError> {
    if rect.width.0 > 0.0 && rect.height.0 > 0.0 {
        return Ok(());
    }
    Err(DecodeError::DegenerateRect {
        width_pt: rect.width.0,
        height_pt: rect.height.0,
    })
}

/// Кладёт изображение в ресурсы или находит уже собранный `XObject`.
///
/// Возвращает имя `XObject` и размеры исходника в пикселях. Имя выводится из
/// содержимого, поэтому одинаковые байты попадают в одну запись ресурсов:
/// [`RawImage`] переезжает по владению только при первом размещении, повторы
/// декодирования не требуют. Ключ — именно байты, а не id media-части из
/// модели: [`place`] никакого id не получает, а строка [`Placement::id`]
/// приходит от вызывающего и ничем не подтверждена — отпечаток байтов честнее
/// и вдобавок склеивает одинаковые копии, разложенные по разным частям пакета.
///
/// # Errors
/// [`DecodeError`] от [`decode`] — если такой картинки в ресурсах ещё нет.
fn register(
    resources: &mut PdfResources,
    mime: &str,
    bytes: &[u8],
) -> Result<(XObjectId, usize, usize), DecodeError> {
    let id = xobject_id(bytes);
    if let Some((Px(width), Px(height))) = resources
        .xobjects
        .map
        .get(&id)
        .and_then(XObject::get_width_height)
    {
        return Ok((id, width, height));
    }
    let image = decode(mime, bytes)?;
    let (width, height) = (image.width, image.height);
    resources
        .xobjects
        .map
        .insert(id.clone(), XObject::Image(image));
    Ok((id, width, height))
}

/// Имя `XObject` по содержимому media-части — ключ дедупликации.
///
/// `DefaultHasher` (SipHash-1-3) — 64 бита: на сотнях картинок книги коллизия
/// пренебрежимо маловероятна, а имя живёт только внутри одного сеанса сборки
/// PDF, поэтому межзапусковая стабильность хеша не нужна. Строка имени — из
/// одних hex-цифр: она становится именем ресурса в `/XObject`, и экранирование
/// спецсимволов пути media-части тут ни к чему.
fn xobject_id(bytes: &[u8]) -> XObjectId {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    XObjectId(format!("img{:016x}", hasher.finish()))
}

/// Операции отрисовки зарегистрированного `XObject`: `q`, отсечение, `Do`, `Q`.
///
/// Картинка растягивается на `rect` независимыми масштабами по осям — так же,
/// как её кладёт canvas-путь; пропорции исходника при этом не сохраняются.
/// Отсечение по прямоугольнику якоря остаётся: у Image-`XObject` нет ни
/// `/BBox`, ни `/Rect`, зато клип гарантирует, что пиксели не выйдут за якорь
/// ни при каких обстоятельствах.
///
/// `rect` проверен [`check_rect`], поэтому деления на нулевую сторону нет.
fn placement_ops(id: &XObjectId, width_px: usize, height_px: usize, rect: &Rect) -> Vec<Op> {
    let transform = XObjectTransform {
        dpi: Some(DPI),
        // Растягивание точно на якорь: так же картинку кладёт canvas-путь.
        scale_x: Some(rect.width.0 / px_to_pt(width_px)),
        scale_y: Some(rect.height.0 / px_to_pt(height_px)),
        translate_x: Some(rect.x),
        translate_y: Some(rect.y),
        rotate: None,
    };
    vec![
        Op::SaveGraphicsState,
        Op::DrawPolygon {
            polygon: clip_rect(rect),
        },
        Op::UseXobject {
            id: id.clone(),
            transform,
        },
        Op::RestoreGraphicsState,
    ]
}

/// Путь отсечения по прямоугольнику в координатах страницы (левый нижний угол).
fn clip_rect(rect: &Rect) -> Polygon {
    let right = Pt(rect.x.0 + rect.width.0);
    let top = Pt(rect.y.0 + rect.height.0);
    let points = [
        (rect.x, rect.y),
        (right, rect.y),
        (right, top),
        (rect.x, top),
    ]
    .into_iter()
    .map(|(x, y)| LinePoint {
        p: Point { x, y },
        bezier: false,
    })
    .collect();
    Polygon {
        rings: vec![PolygonRing { points }],
        mode: PaintMode::Clip,
        winding_order: WindingOrder::NonZero,
    }
}

/// Пиксели изображения в точки при [`DPI`]: 1 px = 1 pt.
///
/// Погрешность `f32` (после 2^24 px по стороне) заведомо ниже точности
/// печати; картинок больше 16 млн пикселей по стороне не бывает.
#[allow(clippy::cast_precision_loss)]
fn px_to_pt(px: usize) -> f32 {
    px as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use printpdf::{OutputImageFormat, RawImageData, RawImageFormat};

    /// 2x2: красный, зелёный, синий, белый.
    fn rgb_pixels() -> Vec<u8> {
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]
    }

    fn raw_image() -> RawImage {
        RawImage {
            pixels: RawImageData::U8(rgb_pixels()),
            width: 2,
            height: 2,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        }
    }

    fn encoded(format: OutputImageFormat) -> Vec<u8> {
        raw_image()
            .encode_to_bytes(&[format])
            .expect("картинка кодируется")
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

    /// Имя `XObject` из операций размещения (`q`, клип, `Do`, `Q`).
    fn placed_id(ops: &[Op]) -> XObjectId {
        match ops.get(2) {
            Some(Op::UseXobject { id, .. }) => id.clone(),
            other => panic!("ожидался UseXobject, пришло {other:?}"),
        }
    }

    #[test]
    fn png_is_decoded_pixel_to_pixel() {
        let image = decode("image/png", &encoded(OutputImageFormat::Png)).expect("PNG разбирается");
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.data_format, RawImageFormat::RGB8);
        assert_eq!(image.pixels, RawImageData::U8(rgb_pixels()));
    }

    #[test]
    fn jpeg_is_decoded_with_its_dimensions() {
        let image =
            decode("image/jpeg", &encoded(OutputImageFormat::Jpeg)).expect("JPEG разбирается");
        assert_eq!((image.width, image.height), (2, 2));
        // JPEG с потерями: значения пикселей не сверяем, но формата и объёма
        // данных достаточно, чтобы поймать «декодер вернул пустышку».
        assert_eq!(image.data_format, RawImageFormat::RGB8);
        let RawImageData::U8(pixels) = image.pixels else {
            panic!("ожидались 8-битные пиксели");
        };
        assert_eq!(pixels.len(), rgb_pixels().len());
    }

    #[test]
    fn broken_bytes_are_reported_not_panicking() {
        let error = decode("image/png", b"not an image").expect_err("битые байты");
        match error {
            DecodeError::Decode(message) => assert!(!message.is_empty(), "есть причина"),
            other => panic!("ожидалась Decode, пришло {other}"),
        }
    }

    #[test]
    fn gif_is_rejected_by_mime_without_decoding() {
        let error = decode("image/gif", b"GIF89a").expect_err("gif не поддержан");
        match error {
            DecodeError::UnsupportedFormat { mime } => assert_eq!(mime, "image/gif"),
            other => panic!("ожидалась UnsupportedFormat, пришло {other}"),
        }
    }

    #[test]
    fn mime_parameters_and_case_do_not_break_png() {
        let bytes = encoded(OutputImageFormat::Png);
        assert!(decode("IMAGE/PNG; charset=binary", &bytes).is_ok());
    }

    #[test]
    fn empty_or_foreign_mime_falls_through_to_bytes() {
        let bytes = encoded(OutputImageFormat::Png);
        assert!(decode("", &bytes).is_ok());
        assert!(decode("application/octet-stream", &bytes).is_ok());
    }

    #[test]
    fn repeated_bytes_reuse_one_xobject() {
        let bytes = encoded(OutputImageFormat::Png);
        let mut resources = PdfResources::default();
        let first = place(
            &mut resources,
            "image/png",
            &bytes,
            &rect(0.0, 0.0, 20.0, 10.0),
        )
        .expect("первое размещение");
        let second = place(
            &mut resources,
            "image/png",
            &bytes,
            &rect(0.0, 40.0, 40.0, 40.0),
        )
        .expect("повтор размещается без декодирования");
        assert_eq!(resources.xobjects.map.len(), 1, "одна запись ресурсов");
        assert_eq!(placed_id(&first), placed_id(&second), "то же имя `XObject`");
    }

    #[test]
    fn different_bytes_get_different_xobjects() {
        let mut resources = PdfResources::default();
        let red = encoded(OutputImageFormat::Png);
        let other = RawImage {
            pixels: RawImageData::U8(vec![0, 255, 0, 0, 0, 0]),
            width: 2,
            height: 1,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        }
        .encode_to_bytes(&[OutputImageFormat::Png])
        .expect("картинка кодируется")
        .0;
        let first = place(
            &mut resources,
            "image/png",
            &red,
            &rect(0.0, 0.0, 20.0, 10.0),
        )
        .expect("первая картинка");
        let second = place(
            &mut resources,
            "image/png",
            &other,
            &rect(0.0, 40.0, 20.0, 10.0),
        )
        .expect("вторая картинка");
        assert_eq!(resources.xobjects.map.len(), 2);
        assert_ne!(placed_id(&first), placed_id(&second));
    }

    #[test]
    fn wide_image_is_stretched_to_the_anchor() {
        let id = XObjectId("test".to_owned());
        // 16x4 px в якорь 40x40 pt: масштабы по осям независимы (2.5 и 10),
        // пропорции исходника не сохраняются — паритет с canvas-путём.
        let ops = placement_ops(&id, 16, 4, &rect(10.0, 20.0, 40.0, 40.0));
        let Some(Op::UseXobject { transform, .. }) = ops.get(2) else {
            panic!("ожидался XObject");
        };
        assert_eq!(
            transform.scale_x,
            Some(2.5),
            "ширина якоря / ширина исходника"
        );
        assert_eq!(
            transform.scale_y,
            Some(10.0),
            "высота якоря / высота исходника"
        );
        assert_eq!(transform.translate_x, Some(Pt(10.0)));
        assert_eq!(transform.translate_y, Some(Pt(20.0)));
    }

    #[test]
    fn tall_image_is_stretched_to_the_anchor() {
        let id = XObjectId("test".to_owned());
        // 4x16 px в якорь 40x40 pt: 10 по x и 2.5 по y.
        let ops = placement_ops(&id, 4, 16, &rect(10.0, 20.0, 40.0, 40.0));
        let Some(Op::UseXobject { transform, .. }) = ops.get(2) else {
            panic!("ожидался XObject");
        };
        assert_eq!(transform.scale_x, Some(10.0));
        assert_eq!(transform.scale_y, Some(2.5));
        assert_eq!(transform.translate_x, Some(Pt(10.0)));
        assert_eq!(transform.translate_y, Some(Pt(20.0)));
    }

    #[test]
    fn ops_clip_the_rect_and_fit_the_image() {
        let id = XObjectId("test".to_owned());
        let ops = placement_ops(&id, 96, 48, &rect(10.0, 20.0, 48.0, 24.0));
        assert_eq!(ops.len(), 4, "q, отсечение, Do, Q");

        let Some(Op::DrawPolygon { polygon }) = ops.get(1) else {
            panic!("ожидался путь отсечения");
        };
        assert_eq!(polygon.mode, PaintMode::Clip);
        let ring = polygon.rings.first().expect("кольцо есть");
        let points: Vec<(f32, f32)> = ring
            .points
            .iter()
            .map(|line| (line.p.x.0, line.p.y.0))
            .collect();
        assert_eq!(
            points,
            vec![(10.0, 20.0), (58.0, 20.0), (58.0, 44.0), (10.0, 44.0)],
            "обрезка по прямоугольнику якоря"
        );

        let Some(Op::UseXobject { transform, .. }) = ops.get(2) else {
            panic!("ожидался XObject");
        };
        assert_eq!(transform.dpi, Some(DPI));
        assert_eq!(transform.translate_x, Some(Pt(10.0)));
        assert_eq!(transform.translate_y, Some(Pt(20.0)));
        // 96 px при 72 dpi — это 96 pt, поэтому scale_x = 48/96 = 0.5.
        let scale_x = transform.scale_x.expect("масштаб задан");
        let scale_y = transform.scale_y.expect("масштаб задан");
        assert!((scale_x - 0.5).abs() < f32::EPSILON, "scale_x = {scale_x}");
        assert!((scale_y - 0.5).abs() < f32::EPSILON, "scale_y = {scale_y}");
    }

    #[test]
    fn degenerate_rect_is_rejected_before_decoding() {
        let mut resources = PdfResources::default();
        let bytes = encoded(OutputImageFormat::Png);
        let error = place(
            &mut resources,
            "image/png",
            &bytes,
            &rect(0.0, 0.0, 0.0, 10.0),
        )
        .expect_err("нулевая ширина");
        assert!(matches!(error, DecodeError::DegenerateRect { .. }));
        assert!(
            resources.xobjects.map.is_empty(),
            "ресурсы не должны засоряться XObject'ом без операций"
        );
    }

    #[test]
    fn empty_placements_give_empty_batch() {
        let mut resources = PdfResources::default();
        let batch = place_all(&mut resources, &[]);
        assert!(batch.ops.is_empty());
        assert!(batch.skipped.is_empty());
        assert!(resources.xobjects.map.is_empty());
    }

    #[test]
    fn broken_image_does_not_break_the_batch() {
        let mut resources = PdfResources::default();
        let good = encoded(OutputImageFormat::Png);
        let placements = [
            Placement {
                id: "xl/media/image1.png",
                mime: "image/png",
                bytes: &good,
                rect: rect(10.0, 10.0, 40.0, 20.0),
            },
            Placement {
                id: "xl/media/image2.png",
                mime: "image/png",
                bytes: b"broken",
                rect: rect(10.0, 40.0, 40.0, 20.0),
            },
        ];
        let batch = place_all(&mut resources, &placements);
        assert_eq!(batch.skipped.len(), 1, "битая картинка пропущена");
        assert_eq!(batch.skipped[0].id, "xl/media/image2.png");
        assert_eq!(batch.ops.len(), 4, "операции уцелевшей картинки на месте");
        assert_eq!(resources.xobjects.map.len(), 1);
    }
}
