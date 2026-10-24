//! Чтение OOXML-атрибутов.
//!
//! Общие хелперы — разбор атрибутов, поиск по локальному имени, разворот
//! ссылок и разбор цвета — живут в [`doc_converter_core::xml`]. Здесь остаются
//! обёртки, которые добавляют к ошибкам имя части, и перевод цвета в модель
//! XLSX.
//!
//! Аллокаций по пути нет, пока в значении не встретится сущность: атрибуты
//! читаются на каждой ячейке, а их в книге бывает миллион.

use quick_xml::events::{BytesRef, BytesStart};
use smallvec::SmallVec;

use crate::error::{Result, XlsxError};
use crate::model::Color;

pub(crate) use doc_converter_core::xml::{find, is_true, Attr};

/// Разобрать атрибуты элемента.
///
/// # Errors
///
/// [`XlsxError::Malformed`] — атрибут не читается: битые байты или
/// некорректная XML-сущность в значении.
pub(crate) fn attributes<'a>(
    element: &'a BytesStart<'a>,
    part: &str,
) -> Result<SmallVec<[Attr<'a>; 4]>> {
    doc_converter_core::xml::attributes(element).map_err(|e| match e {
        doc_converter_core::Error::Malformed(reason) => XlsxError::malformed(part, reason),
        other => XlsxError::Core(other),
    })
}

/// Цвет из атрибутов `rgb`, `theme` или `indexed`.
///
/// `rgb` бывает восьмизначным (`AARRGGBB`) и шестизначным (`RRGGBB`); во втором
/// случае альфы в файле нет, и она считается непрозрачной.
pub(crate) fn color(attrs: &[Attr<'_>]) -> Color {
    use doc_converter_core::xml::ColorSpec;

    match doc_converter_core::xml::color(attrs) {
        ColorSpec::None => Color::None,
        ColorSpec::Rgb(value) => Color::Rgb(value),
        ColorSpec::Theme(value) => Color::Theme(value),
        ColorSpec::Indexed(value) => Color::Indexed(value),
    }
}

/// Развернуть ссылку на сущность (`&amp;`, `&#65;`, `&#x41;`) в текст.
///
/// Начиная с quick-xml 0.41 ссылки приходят отдельным событием, и разворачивать
/// их должен вызывающий. В OOXML встречаются только пять предопределённых
/// сущностей и числовые ссылки: внешние потребовали бы DTD, которого в этих
/// файлах не бывает, — поэтому незнакомая ссылка означает порчу файла, а не
/// повод что-то додумывать.
///
/// # Errors
///
/// [`XlsxError::Malformed`] — ссылка не декодируется, содержит недопустимый код
/// символа или ссылается на неизвестную сущность.
pub(crate) fn resolve_reference(reference: &BytesRef<'_>, part: &str) -> Result<String> {
    doc_converter_core::xml::resolve_reference(reference).map_err(|e| match e {
        doc_converter_core::Error::Malformed(reason) => XlsxError::malformed(part, reason),
        other => XlsxError::Core(other),
    })
}
