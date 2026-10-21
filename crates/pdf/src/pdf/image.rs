//! Декодирование картинок media-частей и их PDF-объекты (ADR-0022).
//!
//! Заменяет `RawImage` из вендоренного форка: PNG и JPEG раскладывает крейт `image`
//! (те же декодеры, что вендор включал фичами `png`/`jpeg`), объект
//! `Image`-`XObject` собирает этот модуль. Прозрачность выносится в отдельный
//! поток `/SMask` (`DeviceGray`, 8 бит): в PDF альфу иначе не записать.
//!
//! Mime отсеивает заведомо неподдержанные `image/*` до разбора байтов; пустой
//! и некартиночный mime не отсеивают — медиачасть могла приехать без
//! `Content_Types`, последнее слово за содержимым (как в `crate::image`).

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::PdfError;

/// Раскодированное изображение для встраивания.
#[derive(Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// RGB (3 байта/пиксель) или Gray (1 байт/пиксель).
    pub pixels: Vec<u8>,
    /// Цветовая модель пикселей.
    pub format: PixelFormat,
    /// Альфа-канал (1 байт/пиксель), если был.
    pub alpha: Option<Vec<u8>>,
}

/// Цветовая модель пикселей.
#[derive(Debug)]
pub enum PixelFormat {
    /// RGB, 3 байта/пиксель.
    Rgb,
    /// Серый, 1 байт/пиксель.
    Gray,
}

impl PixelFormat {
    /// Имя пространства PDF для `/ColorSpace`.
    fn color_space(&self) -> &'static str {
        match self {
            Self::Rgb => "DeviceRGB",
            Self::Gray => "DeviceGray",
        }
    }
}

/// Декодировать media-часть (PNG/JPEG) через крейт `image`.
///
/// # Errors
///
/// [`PdfError::Image`], если mime — картинка, но не PNG/JPEG, или байты не
/// разобрал ни один встроенный декодер.
pub fn decode(mime: &str, bytes: &[u8]) -> Result<DecodedImage, PdfError> {
    let normalized = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized.starts_with("image/") && !is_supported(&normalized) {
        return Err(PdfError::Image(format!(
            "unsupported format '{mime}': only PNG and JPEG are supported"
        )));
    }
    let image = image::load_from_memory(bytes)
        .map_err(|err| PdfError::Image(format!("decode failed: {err}")))?;
    let width = image.width();
    let height = image.height();
    let (pixels, format, alpha) = into_parts(image);
    Ok(DecodedImage {
        width,
        height,
        pixels,
        format,
        alpha,
    })
}

/// PNG и JPEG в написаниях, которые встречаются в книгах Office.
fn is_supported(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/x-png" | "image/jpeg" | "image/jpg" | "image/pjpeg"
    )
}

/// `DynamicImage` в пиксели, цветовую модель и отдельный альфа-канал.
///
/// 16-битные PNG приводятся к 8 битам: `/BitsPerComponent 8` зафиксирован в
/// словаре картинки, иных разрядностей PDF-путь не пишет.
fn into_parts(image: image::DynamicImage) -> (Vec<u8>, PixelFormat, Option<Vec<u8>>) {
    match image.color() {
        image::ColorType::L8 | image::ColorType::L16 => {
            (image.into_luma8().into_raw(), PixelFormat::Gray, None)
        }
        image::ColorType::La8 | image::ColorType::La16 => {
            let (gray, alpha) = split_luma_alpha(&image.into_luma_alpha8().into_raw());
            (gray, PixelFormat::Gray, Some(alpha))
        }
        image::ColorType::Rgb8 | image::ColorType::Rgb16 => {
            (image.into_rgb8().into_raw(), PixelFormat::Rgb, None)
        }
        image::ColorType::Rgba8 | image::ColorType::Rgba16 => {
            let (rgb, alpha) = split_rgb_alpha(&image.into_rgba8().into_raw());
            (rgb, PixelFormat::Rgb, Some(alpha))
        }
        // Декодеры PNG/JPEG других типов не возвращают; ветка — на случай
        // расширения списка фич: всё приводится к RGB (+ альфа).
        other => {
            if other.has_alpha() {
                let (rgb, alpha) = split_rgb_alpha(&image.into_rgba8().into_raw());
                (rgb, PixelFormat::Rgb, Some(alpha))
            } else {
                (image.into_rgb8().into_raw(), PixelFormat::Rgb, None)
            }
        }
    }
}

/// RGBA (4 байта/пиксель) в RGB и отдельный альфа-канал.
// `as_chunks` стабилен с Rust 1.88, а MSRV workspace — 1.82, поэтому
// предложение clippy здесь не применяется (как в `session.rs`).
#[allow(clippy::chunks_exact_to_as_chunks)]
fn split_rgb_alpha(rgba: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    let mut alpha = Vec::with_capacity(rgba.len() / 4);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
        alpha.push(pixel[3]);
    }
    (rgb, alpha)
}

/// Серый + альфа (2 байта/пиксель) в серый и отдельный альфа-канал.
#[allow(clippy::chunks_exact_to_as_chunks)] // MSRV 1.82, см. выше
fn split_luma_alpha(luma_alpha: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut gray = Vec::with_capacity(luma_alpha.len() / 2);
    let mut alpha = Vec::with_capacity(luma_alpha.len() / 2);
    for pixel in luma_alpha.chunks_exact(2) {
        gray.push(pixel[0]);
        alpha.push(pixel[1]);
    }
    (gray, alpha)
}

/// Добавить `XObject` (+ `SMask` при альфе) в scratch-`Document`, вернуть id
/// `XObject`.
///
/// Оба потока помечаются `allows_compression`: сжимать их или нет, решает
/// писатель по флагу `compress`, как для потоков содержимого и шрифтов.
pub(crate) fn write_image(img: &DecodedImage, doc: &mut Document) -> ObjectId {
    let mut dict = image_dict(img, img.format.color_space());
    if let Some(alpha) = &img.alpha {
        // SMask кладётся в документ раньше основного потока: его id вставляется
        // ссылкой в словарь картинки, а `Document` раздаёт id только
        // возрастающие.
        let smask_id = doc.add_object(
            Stream::new(image_dict(img, "DeviceGray"), alpha.clone()).with_compression(true),
        );
        dict.set("SMask", Object::Reference(smask_id));
    }
    doc.add_object(Stream::new(dict, img.pixels.clone()).with_compression(true))
}

/// Словарь `Image`-`XObject` без потока: общие ключи картинки и её `SMask`.
fn image_dict(img: &DecodedImage, color_space: &str) -> Dictionary {
    let mut dict = Dictionary::new();
    dict.set("Type", "XObject");
    dict.set("Subtype", "Image");
    dict.set("Width", i64::from(img.width));
    dict.set("Height", i64::from(img.height));
    dict.set("ColorSpace", color_space);
    dict.set("BitsPerComponent", 8_i64);
    dict
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{decode, PixelFormat};
    use crate::PdfError;

    /// RGB-пиксели 2x2: красный, зелёный, синий, белый.
    const RGB: [u8; 12] = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];

    /// Кодирует RGB-буфер 2x2 в заданный формат — декодер получает байты,
    /// сгенерированные тем же крейтом `image`.
    fn encode_rgb(format: image::ImageFormat, pixels: Vec<u8>) -> Vec<u8> {
        let buffer = image::RgbImage::from_raw(2, 2, pixels).expect("2x2 x 3 байта");
        let mut bytes = Vec::new();
        buffer
            .write_to(&mut Cursor::new(&mut bytes), format)
            .expect("картинка кодируется");
        bytes
    }

    #[test]
    fn png_rgb_decodes_pixel_to_pixel() {
        let decoded = decode(
            "image/png",
            &encode_rgb(image::ImageFormat::Png, RGB.to_vec()),
        )
        .expect("PNG разбирается");
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert!(matches!(decoded.format, PixelFormat::Rgb));
        assert_eq!(decoded.pixels, RGB);
        assert!(decoded.alpha.is_none(), "альфы в RGB нет");
    }

    #[test]
    fn png_rgba_splits_rgb_and_alpha() {
        let rgba = vec![
            255, 0, 0, 10, 0, 255, 0, 20, 0, 0, 255, 30, 255, 255, 255, 255,
        ];
        let buffer = image::RgbaImage::from_raw(2, 2, rgba).unwrap();
        let mut bytes = Vec::new();
        buffer
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();

        let decoded = decode("image/png", &bytes).unwrap();
        assert!(matches!(decoded.format, PixelFormat::Rgb));
        assert_eq!(decoded.pixels, RGB, "альфа отделена от RGB");
        assert_eq!(decoded.alpha.as_deref(), Some(&[10, 20, 30, 255][..]));
    }

    #[test]
    fn png_grayscale_stays_gray_without_alpha() {
        let gray = vec![0, 255, 128, 64];
        let buffer = image::GrayImage::from_raw(2, 2, gray.clone()).unwrap();
        let mut bytes = Vec::new();
        buffer
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();

        let decoded = decode("image/png", &bytes).unwrap();
        assert!(matches!(decoded.format, PixelFormat::Gray));
        assert_eq!(decoded.pixels, gray);
        assert!(decoded.alpha.is_none());
    }

    #[test]
    fn png_grayscale_with_alpha_keeps_alpha() {
        let luma_alpha = vec![0, 0, 255, 255, 128, 64, 200, 100];
        let buffer = image::GrayAlphaImage::from_raw(2, 2, luma_alpha).unwrap();
        let mut bytes = Vec::new();
        buffer
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();

        let decoded = decode("image/png", &bytes).unwrap();
        assert!(matches!(decoded.format, PixelFormat::Gray));
        assert_eq!(decoded.pixels, vec![0, 255, 128, 200]);
        assert_eq!(decoded.alpha.as_deref(), Some(&[0, 255, 64, 100][..]));
    }

    #[test]
    fn jpeg_decodes_by_mime() {
        let decoded = decode(
            "image/jpeg",
            &encode_rgb(image::ImageFormat::Jpeg, RGB.to_vec()),
        )
        .expect("JPEG разбирается");
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert!(matches!(decoded.format, PixelFormat::Rgb));
        // JPEG с потерями: значения пикселей не сверяем, но объёма данных
        // достаточно, чтобы поймать «декодер вернул пустышку».
        assert_eq!(decoded.pixels.len(), RGB.len());
        assert!(decoded.alpha.is_none());
    }

    #[test]
    fn unsupported_mime_is_an_error() {
        let error = decode("image/gif", b"GIF89a").expect_err("gif не поддержан");
        assert!(matches!(error, PdfError::Image(_)), "{error}");
    }

    #[test]
    fn broken_bytes_are_an_error() {
        let error = decode("image/png", b"not an image").expect_err("битые байты");
        assert!(matches!(error, PdfError::Image(_)), "{error}");
    }

    #[test]
    fn empty_or_foreign_mime_falls_through_to_bytes() {
        let bytes = encode_rgb(image::ImageFormat::Png, RGB.to_vec());
        assert!(decode("", &bytes).is_ok());
        assert!(decode("application/octet-stream", &bytes).is_ok());
    }
}
