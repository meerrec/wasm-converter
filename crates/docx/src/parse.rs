//! Точка входа разбора: сборка `Document` из частей пакета (слайс S12).

use doc_converter_core::zip_limits::ZipLimits;

use crate::error::Result;
use crate::model::Document;

/// Разбирает пакет DOCX в модель.
///
/// # Errors
/// Фатальные ошибки разбора — [`crate::Error`]; восстановимые копятся в предупреждениях.
pub(crate) fn parse_docx(bytes: &[u8], limits: ZipLimits) -> Result<Document> {
    let _ = (bytes, limits);
    todo!("parse_docx — Спринт 8, слайс S12")
}
