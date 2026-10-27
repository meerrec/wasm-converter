//! Общие помощники чтения XML ODF.
//!
//! `doc-converter-core` уже разбирает атрибуты и разворачивает сущности; здесь
//! остаётся только приписать ошибке имя части пакета — в `core` его нет, а в
//! сообщении пользователю часть важнее, чем номер байта.

use quick_xml::events::BytesRef;

use crate::error::{OdfError, Result};

pub(crate) use doc_converter_core::xml::find;
// `is_true` нужен только разбору числовых форматов ODS; в остальных срезах
// реэкспорт был бы неиспользуемым импортом.
#[cfg(feature = "ods")]
pub(crate) use doc_converter_core::xml::is_true;

/// Приписать ошибке `core` имя части пакета.
pub(crate) fn map_core(part: &str, error: doc_converter_core::Error) -> OdfError {
    match error {
        doc_converter_core::Error::Malformed(reason) => OdfError::Malformed {
            part: part.to_owned(),
            reason,
        },
        other => OdfError::Core(other),
    }
}

/// Развернуть ссылку на сущность (`&amp;`, `&#65;`, `&#x41;`) в текст.
///
/// # Errors
/// [`OdfError::Malformed`] — ссылка не декодируется или неизвестна.
pub(crate) fn resolve_reference(reference: &BytesRef<'_>, part: &str) -> Result<String> {
    doc_converter_core::xml::resolve_reference(reference).map_err(|error| map_core(part, error))
}
