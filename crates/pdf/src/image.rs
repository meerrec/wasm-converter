//! Картинки листа → изображения-`XObject` на страницах PDF.
//!
//! Из реестра книги приходят mime-тип, байты media-части и прямоугольник
//! якоря в точках; модуль декодирует байты (`pdf::image::decode`, ADR-0022),
//! ведёт реестр «байты → имя `XObject`» и отдаёт операции
//! страницы — отсечение по прямоугольнику и `Op::UseXobject` с матрицей,
//! растягивающей картинку на якорь.
//!
//! Форматы: PNG и JPEG. Остальные форматы Office (gif, bmp, webp, tiff, svg)
//! пропускаются с ошибкой `DecodeError::UnsupportedFormat`: каждый декодер —
//! это вес wasm-модуля, а такие вставки в книгах редки. Пропуск не роняет
//! экспорт листа: `place_all` собирает ошибки всех в отчёт и отдаёт
//! операции уцелевших.
//!
//! Картинка растягивается на весь прямоугольник якоря: так же её кладёт
//! canvas-путь (`drawImage` с явными `dw`/`dh`), и так устроен OOXML без
//! `a:srcRect` — умолчание `a:stretch` заполняет якорь, не сохраняя пропорций.
//! Отсечение по якорю (`W n`) остаётся: пиксели не имеют права выезжать за
//! него ни при каких обстоятельствах. Настоящая обрезка появится, когда
//! парсер начнёт читать `a:srcRect`: тогда вписывание станет обрезкой по
//! данным файла, а не по несовпадению пропорций.
//!
//! Повторно вставленная media-часть не заводит второй `XObject`: реестр
//! `Registry` дедуплицирует по байтам, и книга с одной картинкой в сотне
//! ячеек декодирует её один раз. Имена («X0», «X1», …) раздаёт сессия в
//! порядке регистрации (`Session::add_image`); painter
//! кладёт их в реестр, и `place_all` ссылается на них по байтам.
//!
//! Путь DOCX живёт в `crate::docx::images`: общие с ним только декодер
//! (`crate::pdf::image`) и реестр сессии, размещение изображений у путей своё.

use std::collections::BTreeMap;

use crate::pdf::image::DecodedImage;
use crate::pdf::ops::{
    LinePoint, Op, PaintMode, Point, Polygon, PolygonRing, Pt, Rect, WindingOrder, XObjectId,
    XObjectTransform,
};

/// DPI, при котором размер картинки в пикселях совпадает с её размером в
/// точках (1 px = 1 pt). В матрице [`XObjectTransform`] остаётся только
/// масштаб на прямоугольник якоря: произведение «размер исходника ×
/// растягивание» не зависит от пиксельных размеров и сокращается.
const DPI: f32 = 72.0;

/// Ошибка размещения изображения.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DecodeError {
    /// Формат медиачасти не поддержан.
    #[error("unsupported image format {mime}: only PNG and JPEG are supported")]
    UnsupportedFormat {
        /// Mime-тип из `[Content_Types].xml`.
        mime: String,
    },
    /// Байты не разобрал ни один встроенный декодер: битый файл или формат,
    /// для которого фичи не включены.
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

/// Изображение к размещению на странице.
#[derive(Debug, Clone)]
pub struct Placement<'a> {
    /// Идентификатор media-части (`xl/media/image1.png`). Дедупликация на него
    /// не опирается: ключ — байты, так что разные id с одинаковым содержимым
    /// дают один `XObject`.
    pub id: &'a str,
    /// Mime-тип из `[Content_Types].xml`.
    pub mime: &'a str,
    /// Байты media-части.
    pub bytes: &'a [u8],
    /// Прямоугольник в точках от левого нижнего угла страницы.
    pub rect: Rect,
}

/// Результат [`place_all`]: операции уцелевших изображений.
#[derive(Debug, Default)]
pub struct Batch {
    /// Операции для страницы, в порядке переданных изображений.
    pub ops: Vec<Op>,
}

/// Реестр картинок документа: декодированные изображения и их имена.
///
/// Декодирование отделено от размещения: `XObject`'ы — глобальные ресурсы,
/// и все они обязаны лечь в сессию до первой страницы
/// ([`crate::pdf::session::Session::add_image`]), а операции собираются при
/// отрисовке каждой страницы ([`place_all`]). Ключ дедупликации — байты
/// media-части: имя части из [`Placement::id`] приходит от вызывающего и
/// ничем не подтверждено, а отпечаток байтов честнее и склеивает одинаковые
/// копии, разложенные по разным частям пакета.
#[derive(Debug, Default)]
pub struct Registry {
    /// Уникальные декодированные картинки в порядке первой встречи — порядок
    /// регистрации в сессии.
    images: Vec<DecodedImage>,
    /// Индекс в `images` по байтам части.
    index_by_bytes: BTreeMap<Vec<u8>, usize>,
    /// Байты, которые декодер не принял, — причина пропуска в [`place_all`].
    errors: BTreeMap<Vec<u8>, DecodeError>,
    /// Имя `XObject` по индексу `images`; назначает painter после
    /// [`crate::pdf::session::Session::add_image`] ([`Registry::assign`]).
    names: Vec<Option<XObjectId>>,
}

impl Registry {
    /// Декодировать новые картинки размещений; известные байты не трогаются.
    ///
    /// Битые и неподдержанные запоминаются с причиной: [`place_all`] пропустит
    /// такие размещения, не декодируя повторно.
    pub fn collect(&mut self, placements: &[Placement<'_>]) {
        for placement in placements {
            if self.index_by_bytes.contains_key(placement.bytes)
                || self.errors.contains_key(placement.bytes)
            {
                continue;
            }
            match decode(placement.mime, placement.bytes) {
                Ok(decoded) => {
                    let index = self.images.len();
                    self.index_by_bytes.insert(placement.bytes.to_vec(), index);
                    self.images.push(decoded);
                    self.names.push(None);
                }
                Err(error) => {
                    self.errors.insert(placement.bytes.to_vec(), error);
                }
            }
        }
    }

    /// Декодированные картинки в порядке регистрации — аргументы
    /// [`crate::pdf::session::Session::add_image`].
    #[must_use]
    pub fn decoded(&self) -> &[DecodedImage] {
        &self.images
    }

    /// Назначить имя `XObject` картинке с индексом в порядке регистрации.
    ///
    /// Имя отдаёт сессия («X0», «X1», …); вызов за пределами числа картинок
    /// игнорируется — рассинхрон порядка значил бы баг painter'а, а не битый PDF.
    pub fn assign(&mut self, index: usize, name: XObjectId) {
        if let Some(slot) = self.names.get_mut(index) {
            *slot = Some(name);
        }
    }

    /// Имя `XObject` по байтам; `None` — картинка не декодировалась или имя
    /// ещё не назначено.
    #[must_use]
    pub fn name(&self, bytes: &[u8]) -> Option<&XObjectId> {
        self.index_by_bytes
            .get(bytes)
            .and_then(|&index| self.names.get(index).and_then(Option::as_ref))
    }

    /// Причина пропуска по байтам, если картинка не декодировалась.
    #[must_use]
    pub fn error(&self, bytes: &[u8]) -> Option<&DecodeError> {
        self.errors.get(bytes)
    }
}

/// Размещает изображения страницы, пропуская битые и неподдержанные.
///
/// Порядок операций — порядок `placements`: вызывающий сортирует их в порядке
/// наложения листа (`Sheet.images`). Ошибка одного изображения не мешает
/// остальным: операции уцелевших ложатся в [`Batch::ops`], так что одно битое
/// вложение не роняет экспорт листа.
#[must_use]
pub fn place_all(registry: &Registry, placements: &[Placement<'_>]) -> Batch {
    let mut batch = Batch::default();
    for placement in placements {
        if let Ok(ops) = place(registry, placement) {
            batch.ops.extend(ops);
        }
    }
    batch
}

/// Операции размещения одного изображения.
///
/// # Errors
///
/// [`DecodeError::DegenerateRect`], если прямоугольник вырожден (проверяется
/// до имени: незарегистрированная картинка с нулевой стороной обязана
/// отчитаться о прямоугольнике, а не о реестре); [`DecodeError::Decode`],
/// если картинка не декодировалась или имя ей ещё не назначено.
fn place(registry: &Registry, placement: &Placement<'_>) -> Result<Vec<Op>, DecodeError> {
    check_rect(&placement.rect)?;
    if let Some(error) = registry.error(placement.bytes) {
        return Err(error.clone());
    }
    let Some(id) = registry.name(placement.bytes) else {
        return Err(DecodeError::Decode(String::from(
            "image was not registered in the document",
        )));
    };
    Ok(placement_ops(id, &placement.rect))
}

/// Декодирует байты media-части через свой бэкенд ([`crate::pdf::image`]).
///
/// Mime нужен только чтобы отсеять заведомо неподдержанные `image/*` до
/// разбора байтов и назвать формат в ошибке. Пустой mime и mime не-картинки
/// не отсеивают: часть могла приехать без `Content_Types`, и последнее слово
/// всё равно за содержимым.
///
/// # Errors
/// * [`DecodeError::UnsupportedFormat`] — mime начинается с `image/`, но это
///   не PNG и не JPEG (в том числе `image/gif` и `image/svg+xml`);
/// * [`DecodeError::Decode`] — байты не разобрал встроенный декодер.
fn decode(mime: &str, bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
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
    crate::pdf::image::decode(mime, bytes).map_err(|err| DecodeError::Decode(err.to_string()))
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

/// Операции отрисовки зарегистрированного `XObject`: `q`, отсечение, `Do`, `Q`.
///
/// Картинка растягивается на `rect` независимыми масштабами по осям — так же,
/// как её кладёт canvas-путь; пропорции исходника при этом не сохраняются.
/// PDF кладёт `Image`-`XObject` в квадрат 1×1, поэтому масштаб — сами стороны
/// прямоугольника (произведение «размер исходника × растягивание» от
/// пиксельных размеров не зависит). Отсечение по прямоугольнику якоря
/// остаётся: у Image-`XObject` нет ни `/BBox`, ни `/Rect`, зато клип
/// гарантирует, что пиксели не выйдут за якорь ни при каких обстоятельствах.
///
/// `rect` проверен [`check_rect`], поэтому деления на нулевую сторону нет.
fn placement_ops(id: &XObjectId, rect: &Rect) -> Vec<Op> {
    let transform = XObjectTransform {
        dpi: Some(DPI),
        scale_x: Some(rect.width.0),
        scale_y: Some(rect.height.0),
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{decode, place, place_all, placement_ops, DecodeError, Registry};
    use crate::pdf::ops::{Op, PaintMode, Pt, Rect, XObjectId};

    /// RGB-пиксели 2x2: красный, зелёный, синий, белый.
    const RGB: [u8; 12] = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];

    /// Кодирует RGB-буфер 2x2 в заданный формат — декодер получает байты,
    /// сгенерированные тем же крейтом `image`.
    fn encode_rgb(format: image::ImageFormat) -> Vec<u8> {
        let buffer = image::RgbImage::from_raw(2, 2, RGB.to_vec()).expect("2x2 x 3 байта");
        let mut bytes = Vec::new();
        buffer
            .write_to(&mut Cursor::new(&mut bytes), format)
            .expect("картинка кодируется");
        bytes
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x: Pt(x),
            y: Pt(y),
            width: Pt(w),
            height: Pt(h),
        }
    }

    /// Имена `XObject` из операций размещения, по порядку выводов.
    fn placed_ids(ops: &[Op]) -> Vec<XObjectId> {
        ops.iter()
            .filter_map(|op| match op {
                Op::UseXobject { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn png_is_decoded_pixel_to_pixel() {
        let decoded =
            decode("image/png", &encode_rgb(image::ImageFormat::Png)).expect("PNG разбирается");
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert!(matches!(
            decoded.format,
            crate::pdf::image::PixelFormat::Rgb
        ));
        assert_eq!(decoded.pixels, RGB);
        assert!(decoded.alpha.is_none(), "альфы в RGB нет");
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
    fn empty_or_foreign_mime_falls_through_to_bytes() {
        let bytes = encode_rgb(image::ImageFormat::Png);
        assert!(decode("", &bytes).is_ok());
        assert!(decode("application/octet-stream", &bytes).is_ok());
    }

    /// Реестр с одной зарегистрированной картинкой.
    fn registry(png: &[u8]) -> (Registry, Vec<u8>) {
        let mut registry = Registry::default();
        registry.collect(&[super::Placement {
            id: "xl/media/image1.png",
            mime: "image/png",
            bytes: png,
            rect: rect(0.0, 0.0, 20.0, 10.0),
        }]);
        registry.assign(0, XObjectId(String::from("X0")));
        (registry, png.to_vec())
    }

    #[test]
    fn repeated_bytes_reuse_one_name() {
        let bytes = encode_rgb(image::ImageFormat::Png);
        let (registry, bytes) = registry(&bytes);
        let placements = [
            super::Placement {
                id: "xl/media/image1.png",
                mime: "image/png",
                bytes: &bytes,
                rect: rect(0.0, 0.0, 20.0, 10.0),
            },
            super::Placement {
                id: "xl/media/image2.png",
                mime: "image/png",
                bytes: &bytes,
                rect: rect(0.0, 40.0, 40.0, 40.0),
            },
        ];
        let batch = place_all(&registry, &placements);
        assert_eq!(
            batch.ops.len(),
            8,
            "обе картинки размещаются: по четыре операции"
        );
        assert_eq!(registry.decoded().len(), 1, "одна декодированная картинка");
        let ids = placed_ids(&batch.ops);
        assert_eq!(ids.len(), 2, "два вывода");
        assert_eq!(ids[0], ids[1], "то же имя `XObject`");
    }

    #[test]
    fn different_bytes_get_different_names() {
        let red = encode_rgb(image::ImageFormat::Png);
        let other = encode_rgb(image::ImageFormat::Jpeg);
        let mut registry = Registry::default();
        registry.collect(&[
            super::Placement {
                id: "xl/media/image1.png",
                mime: "image/png",
                bytes: &red,
                rect: rect(0.0, 0.0, 20.0, 10.0),
            },
            super::Placement {
                id: "xl/media/image2.jpeg",
                mime: "image/jpeg",
                bytes: &other,
                rect: rect(0.0, 40.0, 20.0, 10.0),
            },
        ]);
        assert_eq!(registry.decoded().len(), 2);
        registry.assign(0, XObjectId(String::from("X0")));
        registry.assign(1, XObjectId(String::from("X1")));
        assert_ne!(registry.name(&red), registry.name(&other));
    }

    #[test]
    fn ops_clip_the_rect_and_stretch_the_image() {
        let id = XObjectId("test".to_owned());
        let ops = placement_ops(&id, &rect(10.0, 20.0, 48.0, 24.0));
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
        // Квадрат 1×1 картинки растягивается на сам якорь: масштабы — стороны
        // прямоугольника, перенос — его левый нижний угол.
        assert_eq!(transform.scale_x, Some(48.0));
        assert_eq!(transform.scale_y, Some(24.0));
        assert_eq!(transform.translate_x, Some(Pt(10.0)));
        assert_eq!(transform.translate_y, Some(Pt(20.0)));
    }

    #[test]
    fn degenerate_rect_gives_no_ops() {
        let bytes = encode_rgb(image::ImageFormat::Png);
        let (registry, bytes) = registry(&bytes);
        let placement = super::Placement {
            id: "xl/media/image1.png",
            mime: "image/png",
            bytes: &bytes,
            rect: rect(0.0, 0.0, 0.0, 10.0),
        };
        let batch = place_all(&registry, std::slice::from_ref(&placement));
        assert!(batch.ops.is_empty(), "нулевая ширина — операций нет");
        assert!(
            matches!(
                place(&registry, &placement),
                Err(DecodeError::DegenerateRect { .. })
            ),
            "причина пропуска — вырожденный прямоугольник"
        );
    }

    #[test]
    fn broken_image_does_not_break_the_batch() {
        let good = encode_rgb(image::ImageFormat::Png);
        let mut registry = Registry::default();
        let placements = [
            super::Placement {
                id: "xl/media/image1.png",
                mime: "image/png",
                bytes: &good,
                rect: rect(10.0, 10.0, 40.0, 20.0),
            },
            super::Placement {
                id: "xl/media/image2.png",
                mime: "image/png",
                bytes: b"broken",
                rect: rect(10.0, 40.0, 40.0, 20.0),
            },
        ];
        registry.collect(&placements);
        registry.assign(0, XObjectId(String::from("X0")));
        let batch = place_all(&registry, &placements);
        assert_eq!(
            placed_ids(&batch.ops),
            vec![XObjectId(String::from("X0"))],
            "битая картинка в операции не попала, уцелевшая — на месте"
        );
        assert_eq!(batch.ops.len(), 4, "операции уцелевшей картинки на месте");
        assert_eq!(registry.decoded().len(), 1);
        assert!(
            matches!(
                place(&registry, &placements[1]),
                Err(DecodeError::Decode(_))
            ),
            "причина пропуска — ошибка декодирования"
        );
    }

    #[test]
    fn empty_placements_give_empty_batch() {
        let registry = Registry::default();
        let batch = place_all(&registry, &[]);
        assert!(batch.ops.is_empty());
        assert!(registry.decoded().is_empty());
    }
}
