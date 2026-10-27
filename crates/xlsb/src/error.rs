//! Ошибки разбора XLSB.
//!
//! Единая точка для всех уровней: контейнер OOXML (`doc-converter-core`),
//! заголовки записей BIFF12 (`crate::reader`) и смысловые нарушения модели,
//! которую собирает `xlsx`. Обёртки объявлены `transparent`, чтобы диагностика
//! `miette` из нижних слоёв доходила до пользователя без потери меток.

use miette::Diagnostic;
use thiserror::Error;

/// Ошибка разбора XLSB.
///
/// `non_exhaustive`: по мере подключения стилей, формул и остальных частей
/// пакета набор вариантов будет расти, и это не должно ломать сопоставления
/// у потребителей.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum XlsbError {
    /// Ошибка уровня контейнера: ZIP, лимиты архива, relationships.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(#[from] doc_converter_core::Error),

    /// Часть пакета нарушает MS-XLSB.
    ///
    /// `offset` — смещение начала записи в распакованной части: оно указывает
    /// на запись, а не на конкретный байт внутри неё, потому что заголовок
    /// записи и её данные разбираются одним проходом.
    #[error("malformed `{part}` at byte {offset}: {reason}")]
    #[diagnostic(help(
        "the workbook is damaged or was produced by a tool that does not follow MS-XLSB"
    ))]
    Malformed {
        /// Часть пакета, где нашлось нарушение (`xl/workbook.bin`).
        part: String,
        /// Смещение записи в этой части.
        offset: usize,
        /// Что именно не так.
        reason: String,
    },

    /// Запись или лист относится к возможности, которой в первом срезе нет.
    ///
    /// Ошибка, а не тихий пропуск: не поддержанный лист изменил бы состав
    /// книги, а не только её вид.
    #[error("unsupported XLSB feature: {0}")]
    Unsupported(String),

    /// Данные превысили предел, который защищает память.
    #[error("XLSB limit exceeded: {0}")]
    Limit(String),
}

impl XlsbError {
    /// Собрать [`XlsbError::Malformed`].
    #[must_use]
    pub fn malformed(part: impl Into<String>, offset: usize, reason: impl Into<String>) -> Self {
        Self::Malformed {
            part: part.into(),
            offset,
            reason: reason.into(),
        }
    }
}

/// Результат разбора XLSB.
pub type Result<T> = std::result::Result<T, XlsbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_passes_through() {
        let inner = doc_converter_core::Error::MissingPart("xl/workbook.bin".into());
        let rendered = inner.to_string();
        let err = XlsbError::from(inner);

        assert!(matches!(err, XlsbError::Core(_)));
        // `transparent` сохраняет текст нижнего слоя как есть.
        assert_eq!(err.to_string(), rendered);
    }

    #[test]
    fn malformed_keeps_part_offset_and_reason() {
        let err = XlsbError::malformed("xl/worksheets/sheet1.bin", 42, "row is truncated");

        assert_eq!(
            err.to_string(),
            "malformed `xl/worksheets/sheet1.bin` at byte 42: row is truncated"
        );
    }
}
