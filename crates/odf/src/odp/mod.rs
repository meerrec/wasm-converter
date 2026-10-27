//! ODP: презентация ODF 1.2 в модель `doc-converter-pptx`.
//!
//! Пакет ODF — это ZIP с `mimetype`, каталогом `META-INF/manifest.xml` и
//! частями `content.xml`/`styles.xml`. Слайды лежат в
//! `office:body/office:presentation/draw:page`; каждый `draw:page` — слайд,
//! `draw:frame`/`draw:text-box`/`draw:rect`/`draw:ellipse`/`draw:custom-shape` —
//! шейпы. Модель — [`Presentation`], поэтому раскладка, кадр и PDF берутся у
//! PPTX-семейства (`doc-converter-pptx-layout`, `doc-converter-pptx-render`,
//! `doc-converter-pptx-pdf`), а не пишутся заново (ADR-0046).
//!
//! # Ограничения первого среза
//!
//! - Маркеры списков (`text:list-style`) не разрешаются: элементы `text:list`
//!   идут плоскими абзацами, вложенность сохраняется отступом уровня (TODO).
//! - `draw:custom-shape` сводится к прямоугольнику: его `draw:enhanced-geometry`
//!   не читается (TODO).
//! - Тема берётся [`Theme::default`](doc_converter_pptx::model::Theme):
//!   гарнитуры и цвета темы ODP (`office:styles`) не переносятся (TODO).
//! - Анимации, переходы, вложенные презентации и диаграммы не поддержаны.

pub mod content;
pub mod styles;

use doc_converter_core::xml::Attr;
use doc_converter_core::{Archive, ZipLimits};
use doc_converter_pptx::model::{Align, Emu, Rgb};
use doc_converter_pptx::Presentation;

use crate::error::{OdfError, Result};
use crate::manifest::{self, PackageKind};
use crate::xml::find;

/// Часть с содержимым документа: телом презентации и автоматическими стилями.
pub const CONTENT_PART: &str = "content.xml";

/// Часть с общими стилями: именованные стили, раскладки и мастер-страницы.
pub const STYLES_PART: &str = "styles.xml";

/// Открыть ODP из сырых байтов.
///
/// `mimetype` обязан объявлять презентацию; `.odt`/`.ods` дают
/// [`OdfError::Unsupported`], а не пустую презентацию. Архив переходит во
/// владение разбора: модель собирается целиком, открытый архив после `open` не
/// нужен.
///
/// # Errors
/// [`OdfError::Core`] — байты не ZIP, в пакете нет `content.xml` или нарушены
/// лимиты архива ([`ZipLimits`] по умолчанию);
/// [`OdfError::Malformed`] — `mimetype`/каталог/XML нарушают ODF 1.2;
/// [`OdfError::Unsupported`] — пакет не ODP.
pub fn open(bytes: Vec<u8>) -> Result<Presentation> {
    let mut archive = Archive::new(bytes, ZipLimits::default())?;
    let manifest = manifest::read(&mut archive)?;
    if manifest.kind != PackageKind::Odp {
        return Err(OdfError::Unsupported(format!(
            "`{}` packages are not supported yet: this slice reads ODP only",
            manifest.kind.extension()
        )));
    }

    let content = archive.part(CONTENT_PART)?;

    // Именованные стили и раскладки страниц лежат в styles.xml, автоматические
    // — в content.xml. Собираем до тела: шейп и прогон сразу получают готовые
    // свойства, а не разрешают имена стилей задним числом.
    let mut book = styles::StyleBook::default();
    if archive.contains(STYLES_PART) {
        let bytes = archive.part(STYLES_PART)?;
        styles::collect(&bytes, STYLES_PART, &mut book)?;
    }
    styles::collect(&content, CONTENT_PART, &mut book)?;

    content::parse(&content, CONTENT_PART, &book, &manifest, &mut archive)
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

/// Длина ODF (`4cm`, `0.8in`, `12pt`, `96px`) в EMU (914400 EMU/дюйм).
///
/// Битые единицы и нечисловые значения дают `None`; вызывающий решает, что с
/// ними делать (умолчание или предупреждение).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn length_emu(raw: &str) -> Option<Emu> {
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
    // Оборона от переполнения при касте f64 → i64: враждебный файл может
    // прислать длину в световых годах.
    if rounded < i64::MIN as f64 || rounded > i64::MAX as f64 {
        return None;
    }
    Some(rounded as i64)
}

/// Кегль ODF (`18pt`, `0.25in`) в пунктах; голый номер трактуется как пункты.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn length_pt(raw: &str) -> Option<f32> {
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

/// `fo:font-weight` → жирность: `bold` или числовой вес ≥ 700.
fn bold_from_weight(raw: &str) -> bool {
    let raw = raw.trim();
    raw.eq_ignore_ascii_case("bold") || raw.parse::<u16>().is_ok_and(|weight| weight >= 700)
}

/// `fo:font-style` → курсив: `italic`/`oblique`.
fn italic_from_style(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "italic" | "oblique"
    )
}

/// `style:text-underline-style` → подчёркивание: всё, кроме `none`/пустого.
fn underline_from_style(raw: &str) -> bool {
    !matches!(raw.trim().to_ascii_lowercase().as_str(), "" | "none")
}

/// `fo:text-align` → [`Align`]: неизвестные значения — влево (умолчание модели).
fn align_from(raw: &str) -> Align {
    match raw.trim().to_ascii_lowercase().as_str() {
        "center" => Align::Center,
        "right" | "end" => Align::Right,
        _ => Align::Left,
    }
}

/// Заливка из `draw:fill`/`draw:fill-color`; `none` — без заливки.
fn fill_color(attrs: &[Attr<'_>]) -> Option<Rgb> {
    if find(attrs, "fill") == Some("none") {
        return None;
    }
    find(attrs, "fill-color").and_then(Rgb::from_hex)
}

/// Цвет контура из `draw:stroke`/`svg:stroke-color`; `none` — без контура.
fn stroke_color(attrs: &[Attr<'_>]) -> Option<Rgb> {
    if find(attrs, "stroke") == Some("none") {
        return None;
    }
    find(attrs, "stroke-color").and_then(Rgb::from_hex)
}
