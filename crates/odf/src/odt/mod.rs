//! ODT: текстовый документ ODF 1.2 в модель `doc-converter-docx`.
//!
//! Пакет ODF — это ZIP с `mimetype`, каталогом `META-INF/manifest.xml` и
//! частями `content.xml`/`styles.xml`/`meta.xml`. Текст лежит в
//! `office:body/office:text`: `text:h`/`text:p` — абзацы, `text:span` — прогоны,
//! `text:list` — списки, `table:table` — таблицы, `draw:frame`+`draw:image` —
//! картинки. Модель — [`Document`], поэтому
//! раскладка, кадр и PDF берутся у DOCX-семейства (ADR-0046), а не пишутся
//! заново.
//!
//! # Ограничения первого среза
//!
//! - Маркеры списков (`text:list-style`) не разрешаются: элементы `text:list`
//!   идут плоскими абзацами, вложенность сохраняется уровнем (TODO).
//! - Колонтитулы из `style:master-page` не переносятся (TODO): поля
//!   `Document::headers`/`Document::footers` пусты.
//! - Границы и отступы ячеек таблицы (`fo:border`, `fo:padding`) не читаются;
//!   заливка (`fo:background-color`) переносится, объединения — только по
//!   горизонтали (TODO).
//! - Тема и нумерация не переносятся: `StyleTable` и `NumberingTable` пусты,
//!   свойства знака и абзаца кладутся прямо в `RawRPr`/`RawPPr`.

pub mod content;
pub mod styles;

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_core::{Archive, ZipLimits};
use doc_converter_docx::{Color, Document, Justification, Metadata, Twips};
use quick_xml::events::{BytesStart, Event};

use crate::error::{OdfError, Result};
use crate::manifest::{self, Manifest, PackageKind};
use crate::xml::{find, map_core, resolve_reference};

/// Часть с содержимым документа: телом и автоматическими стилями.
pub const CONTENT_PART: &str = "content.xml";

/// Часть с общими стилями: именованные стили, раскладки и мастер-страницы.
pub const STYLES_PART: &str = "styles.xml";

/// Часть с метаданными документа (`dc:title`, `dc:creator`, …).
pub const META_PART: &str = "meta.xml";

/// Открыть ODT из сырых байтов.
///
/// `mimetype` обязан объявлять текстовый документ; `.ods`/`.odp` дают
/// [`OdfError::Unsupported`], а не пустой документ. Архив переходит во владение
/// разбора: модель собирается целиком, и открытый архив после `open` не нужен.
///
/// # Errors
/// [`OdfError::Core`] — байты не ZIP, в пакете нет `content.xml` или нарушены
/// лимиты архива ([`ZipLimits`] по умолчанию);
/// [`OdfError::Malformed`] — `mimetype`/каталог/XML нарушают ODF 1.2;
/// [`OdfError::Unsupported`] — пакет не ODT.
pub fn open(bytes: Vec<u8>) -> Result<Document> {
    let mut archive = Archive::new(bytes, ZipLimits::default())?;
    let manifest = manifest::read(&mut archive)?;
    if manifest.kind != PackageKind::Odt {
        return Err(OdfError::Unsupported(format!(
            "`{}` packages are not supported yet: this slice reads ODT only",
            manifest.kind.extension()
        )));
    }

    let content = archive.part(CONTENT_PART)?;

    // Именованные стили, раскладки страниц и мастер-страницы лежат в styles.xml,
    // автоматические — в content.xml. Собираем до тела: абзац сразу получает
    // готовые свойства, а не разрешает имена стилей задним числом.
    let mut book = styles::StyleBook::default();
    if archive.contains(STYLES_PART) {
        let bytes = archive.part(STYLES_PART)?;
        styles::collect(&bytes, STYLES_PART, &mut book)?;
    }
    styles::collect(&content, CONTENT_PART, &mut book)?;

    // Метаданные необязательны: пакет без `meta.xml` открывается с пустыми
    // свойствами, а не отвергается.
    let metadata = if archive.contains(META_PART) {
        let bytes = archive.part(META_PART)?;
        parse_meta(&bytes, META_PART)?
    } else {
        Metadata::default()
    };

    content::parse(
        &content,
        CONTENT_PART,
        &book,
        &manifest,
        &mut archive,
        metadata,
    )
}

/// Разобрать `value` и `unit` из длины ODF (`28cm` → `(28.0, "cm")`).
fn split_length(raw: &str) -> Option<(f64, &str)> {
    let raw = raw.trim();
    let split = raw
        .find(|ch: char| !(ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+')))
        .unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    let value: f64 = number.parse().ok()?;
    value.is_finite().then_some((value, unit))
}

/// Длина ODF (`4cm`, `0.8in`, `12pt`) в twips (1/1440 дюйма).
///
/// Битые единицы и нечисловые значения дают `None`; вызывающий решает, что с
/// ними делать (умолчание или пропуск).
#[allow(clippy::cast_possible_truncation)]
fn length_twips(raw: &str) -> Option<Twips> {
    const TWIPS_PER_INCH: f64 = 1440.0;

    let (value, unit) = split_length(raw)?;
    let twips = match unit {
        "mm" => value * TWIPS_PER_INCH / 25.4,
        "cm" => value * TWIPS_PER_INCH / 2.54,
        "in" => value * TWIPS_PER_INCH,
        "pt" => value * 20.0,
        "pc" => value * 240.0,
        "px" => value * TWIPS_PER_INCH / 96.0,
        _ => return None,
    };
    let rounded = twips.round();
    // Оборона от переполнения при касте f64 → i32: враждебный файл может
    // прислать длину в световых годах.
    if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return None;
    }
    Some(Twips::new(rounded as i32))
}

/// Кегль ODF (`18pt`, `0.25in`) в пунктах; голый номер трактуется как пункты.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub(crate) fn length_pt(raw: &str) -> Option<f32> {
    const PT_PER_INCH: f64 = 72.0;

    let (value, unit) = split_length(raw)?;
    let pt = match unit {
        "in" => value * PT_PER_INCH,
        "cm" => value * PT_PER_INCH / 2.54,
        "mm" => value * PT_PER_INCH / 25.4,
        "pc" => value * 12.0,
        "px" => value * PT_PER_INCH / 96.0,
        // Реальные файлы пишут единицу, но голый номер надёжнее принять за
        // пункты, чем отбросить кегль целиком.
        "pt" | "" => value,
        _ => return None,
    };
    pt.is_finite().then_some(pt as f32)
}

/// Длина ODF (`1cm`, `2in`) в EMU (914400 EMU/дюйм) — для размера картинок.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub(crate) fn length_emu(raw: &str) -> Option<i64> {
    const EMU_PER_INCH: f64 = 914_400.0;

    let (value, unit) = split_length(raw)?;
    let emu = match unit {
        "mm" => value * EMU_PER_INCH / 25.4,
        "cm" => value * EMU_PER_INCH / 2.54,
        "in" => value * EMU_PER_INCH,
        "pt" => value * EMU_PER_INCH / 72.0,
        "pc" => value * EMU_PER_INCH / 6.0,
        "px" => value * EMU_PER_INCH / 96.0,
        _ => return None,
    };
    let rounded = emu.round();
    if rounded < i64::MIN as f64 || rounded > i64::MAX as f64 {
        return None;
    }
    Some(rounded as i64)
}

/// `fo:font-weight` → жирность: `bold` или числовой вес ≥ 700.
pub(crate) fn bold_from_weight(raw: &str) -> bool {
    let raw = raw.trim();
    raw.eq_ignore_ascii_case("bold") || raw.parse::<u16>().is_ok_and(|weight| weight >= 700)
}

/// `fo:font-style` → курсив: `italic`/`oblique`.
pub(crate) fn italic_from_style(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "italic" | "oblique"
    )
}

/// `style:text-underline-style` → подчёркивание: всё, кроме `none`/пустого.
pub(crate) fn underline_from_style(raw: &str) -> bool {
    !matches!(raw.trim().to_ascii_lowercase().as_str(), "" | "none")
}

/// Цвет ODF (`#RRGGBB`) → [`Color::Rgb`]; нечитаемый — `None`.
pub(crate) fn color_rgb(raw: &str) -> Option<Color> {
    let hex = raw.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(Color::Rgb(value))
}

/// `fo:text-align` → [`Justification`]; неизвестные значения — по левому краю.
pub(crate) fn justification_from(raw: &str) -> Justification {
    match raw.trim().to_ascii_lowercase().as_str() {
        "center" => Justification::Center,
        "right" | "end" => Justification::Right,
        "justify" => Justification::Both,
        _ => Justification::Left,
    }
}

/// MIME-тип по расширению части — запас, когда каталог не назвал media-type.
fn mime_from_name(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Прочитать `meta.xml` в [`Metadata`].
///
/// ODF хранит свойства в `office:meta` под именами `dc:*` и `meta:*`; текстовые
/// значения переносятся в соответствующие поля модели DOCX.
///
/// # Errors
/// [`OdfError::Core`] — XML не разбирается; [`OdfError::Malformed`] — текст
/// элемента не UTF-8.
fn parse_meta(bytes: &[u8], part: &str) -> Result<Metadata> {
    let mut reader = XmlReader::preserving(bytes, part.to_owned());
    let mut meta = Metadata::default();
    // Имя захватываемого элемента: его текст попадёт в конкретное поле.
    let mut capture: Option<&[u8]> = None;

    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(element) => {
                capture = meta_field(element.local_name().as_ref());
            }
            // Самозакрытый элемент текста не несёт: захват снимается, чтобы
            // следующий текстовый узел не приписался чужому полю.
            Event::Empty(_) | Event::End(_) => capture = None,
            Event::Text(text) => {
                if let Some(field) = capture {
                    let value = text.decode().map_err(|error| {
                        OdfError::malformed(part, format!("text is not UTF-8: {error}"))
                    })?;
                    apply_meta(&mut meta, field, value.trim());
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(field) = capture {
                    let value = resolve_reference(&reference, part)?;
                    apply_meta(&mut meta, field, value.trim());
                }
            }
            _ => {}
        }
    }
    Ok(meta)
}

/// Поле [`Metadata`] по локальному имени элемента `meta.xml`; `None` — незнакомый.
fn meta_field(name: &[u8]) -> Option<&'static [u8]> {
    match name {
        b"title" => Some(b"title"),
        b"creator" => Some(b"creator"),
        b"subject" => Some(b"subject"),
        b"description" => Some(b"description"),
        b"keyword" => Some(b"keyword"),
        b"creation-date" => Some(b"creation-date"),
        b"date" => Some(b"date"),
        b"generator" => Some(b"generator"),
        _ => None,
    }
}

/// Записать значение в поле метаданных, названное локальным именем элемента.
fn apply_meta(meta: &mut Metadata, field: &[u8], value: &str) {
    match field {
        b"title" => meta.title = Some(value.to_owned()),
        b"creator" => meta.creator = Some(value.to_owned()),
        b"subject" => meta.subject = Some(value.to_owned()),
        b"description" => meta.description = Some(value.to_owned()),
        b"keyword" => meta.keywords = Some(value.to_owned()),
        b"creation-date" => meta.created = Some(value.to_owned()),
        b"date" => meta.modified = Some(value.to_owned()),
        b"generator" => meta.application = Some(value.to_owned()),
        _ => {}
    }
}

/// Атрибуты элемента с именем части в ошибке; `Vec` вместо `SmallVec` — атрибутов
/// у текстовых элементов единицы.
pub(crate) fn attrs<'a>(element: &'a BytesStart<'a>, part: &str) -> Result<Vec<Attr<'a>>> {
    let attrs =
        doc_converter_core::xml::attributes(element).map_err(|error| map_core(part, error))?;
    Ok(attrs.into_vec())
}

/// Значение атрибута как owned-строка.
pub(crate) fn owned(attrs: &[Attr<'_>], name: &str) -> Option<String> {
    find(attrs, name).map(str::to_owned)
}

/// MIME-тип картинки: из каталога пакета, иначе по расширению части.
pub(crate) fn media_type(manifest: &Manifest, href: &str) -> String {
    manifest
        .entry(href)
        .and_then(|entry| entry.media_type.as_deref())
        .map_or_else(|| mime_from_name(href).to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_convert_to_twips() {
        // A4: 21 × 29,7 см → 11906 × 16838 twips (округление до целого).
        assert_eq!(length_twips("21cm").map(Twips::value), Some(11906));
        assert_eq!(length_twips("29.7cm").map(Twips::value), Some(16838));
        // 1 дюйм = 1440 twips.
        assert_eq!(length_twips("1in").map(Twips::value), Some(1440));
        assert_eq!(length_twips("72pt").map(Twips::value), Some(1440));
        assert_eq!(length_twips("bogus"), None);
    }

    #[test]
    fn font_sizes_become_points() {
        assert_eq!(length_pt("18pt"), Some(18.0));
        assert_eq!(length_pt("0.5in"), Some(36.0));
        assert_eq!(length_pt("14"), Some(14.0));
        assert_eq!(length_pt("x"), None);
    }

    #[test]
    fn colors_parse_hex() {
        assert_eq!(color_rgb("#1F3864"), Some(Color::Rgb(0x1F_3864)));
        assert_eq!(color_rgb("#0563C1"), Some(Color::Rgb(0x05_63C1)));
        assert_eq!(color_rgb("#XYZ"), None);
        assert_eq!(color_rgb("1F3864"), None);
    }

    #[test]
    fn justification_maps_odf_values() {
        assert_eq!(justification_from("center"), Justification::Center);
        assert_eq!(justification_from("end"), Justification::Right);
        assert_eq!(justification_from("justify"), Justification::Both);
        assert_eq!(justification_from("weird"), Justification::Left);
    }
}
