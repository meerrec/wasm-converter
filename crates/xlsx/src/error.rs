//! Ошибки разбора XLSX.
//!
//! Единая точка для всех уровней: контейнер OOXML (`doc-converter-core`),
//! ссылки на ячейки ([`crate::cellref`]) и смысловые нарушения модели.
//! Обёртки объявлены `transparent`, чтобы диагностика `miette` из нижних
//! слоёв доходила до пользователя без потери меток и подсказок.

use miette::Diagnostic;
use thiserror::Error;

/// Ошибка разбора XLSX.
///
/// `non_exhaustive`: по мере подключения листов, строк и стилей набор вариантов
/// будет расти, и это не должно ломать сопоставления у потребителей.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum XlsxError {
    /// Ошибка уровня контейнера: ZIP, XML, relationships.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(#[from] doc_converter_core::Error),

    /// Ошибка разбора A1-ссылки.
    #[error(transparent)]
    #[diagnostic(transparent)]
    CellRef(#[from] crate::cellref::ParseError),

    /// Часть пакета нарушает ECMA-376.
    #[error("malformed `{part}`: {reason}")]
    #[diagnostic(help(
        "the workbook is damaged or was produced by a tool that does not follow ECMA-376"
    ))]
    Malformed {
        /// Часть пакета, где нашлось нарушение (`xl/worksheets/sheet1.xml`).
        part: String,
        /// Что именно не так.
        reason: String,
    },

    /// Операция отменена вызывающим (ADR-0032): кооперативный флаг взведён
    /// между полосами сборки кадра, состояние движка цело.
    #[error("operation cancelled")]
    Cancelled,

    /// Namespace корня `xl/workbook.xml` не Transitional и не Strict.
    ///
    /// Поддержаны оба варианта OOXML (ADR-0034); всё остальное — либо не
    /// OOXML, либо вариант, которого не существует.
    #[error(
        "unsupported OOXML namespace `{namespace}` in `{part}`; expected Transitional or Strict"
    )]
    UnknownVariant {
        /// Часть, по корню которой определялся вариант.
        part: String,
        /// Namespace, который объявил корневой элемент.
        namespace: String,
    },
}

impl XlsxError {
    /// Собрать [`XlsxError::Malformed`].
    #[must_use]
    pub fn malformed(part: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Malformed {
            part: part.into(),
            reason: reason.into(),
        }
    }
}

/// Результат разбора XLSX.
pub type Result<T> = std::result::Result<T, XlsxError>;

#[cfg(test)]
mod tests {
    use super::*;
    use doc_converter_core::{Archive, ZipLimits};

    #[test]
    fn core_error_passes_through() {
        let inner = doc_converter_core::Error::MissingPart("xl/workbook.xml".into());
        let rendered = inner.to_string();
        let err = XlsxError::from(inner);

        assert!(matches!(err, XlsxError::Core(_)));
        // `transparent` сохраняет текст нижнего слоя как есть.
        assert_eq!(err.to_string(), rendered);
    }

    #[test]
    fn cellref_error_passes_through() {
        let inner = crate::cellref::CellRef::parse("XFE1").unwrap_err();
        let rendered = inner.to_string();
        let err = XlsxError::from(inner);

        assert!(matches!(err, XlsxError::CellRef(_)));
        assert_eq!(err.to_string(), rendered);
    }

    #[test]
    fn question_mark_converts_core_error() {
        fn read_missing() -> Result<String> {
            let mut archive = Archive::new(Vec::new(), ZipLimits::default())?;
            let bytes = archive.part("nope.xml")?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }

        assert!(matches!(read_missing(), Err(XlsxError::Core(_))));
    }

    #[test]
    fn malformed_keeps_part_and_reason() {
        let err = XlsxError::malformed("xl/worksheets/sheet1.xml", "row 0 is not allowed");

        assert_eq!(
            err.to_string(),
            "malformed `xl/worksheets/sheet1.xml`: row 0 is not allowed"
        );
    }
}
