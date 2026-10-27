//! Ошибки разбора ODF.
//!
//! Единая точка для всех уровней: контейнер ODF (`doc-converter-core`),
//! каталог пакета (`META-INF/manifest.xml`) и части формата. Обёртка над
//! [`doc_converter_core::Error`] объявлена `transparent`, чтобы диагностика
//! `miette` из нижних слоёв доходила до пользователя без потери меток.

use miette::Diagnostic;
use thiserror::Error;

/// Ошибка разбора ODF.
///
/// `non_exhaustive`: по мере подключения стилей, печати, картинок и форматов
/// ODT/ODP набор вариантов будет расти, и это не должно ломать сопоставления
/// у потребителей.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum OdfError {
    /// Ошибка уровня контейнера: ZIP, лимиты архива, XML.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(#[from] doc_converter_core::Error),

    /// Часть пакета нарушает ODF 1.2.
    #[error("malformed `{part}`: {reason}")]
    #[diagnostic(help(
        "the package is damaged or was produced by a tool that does not follow ODF 1.2"
    ))]
    Malformed {
        /// Часть пакета, где нашлось нарушение (`content.xml`).
        part: String,
        /// Что именно не так.
        reason: String,
    },

    /// Пакет или его часть относятся к возможности, которой в этом срезе нет.
    #[error("unsupported ODF feature: {0}")]
    Unsupported(String),

    /// Данные превысили предел, который защищает память.
    ///
    /// Отдельно от [`Self::Malformed`]: файл может быть синтаксически целым, но
    /// повторять строки или ячейки столько раз, что модель не поместится в
    /// память.
    #[error("ODF limit exceeded: {0}")]
    Limit(String),
}

impl OdfError {
    /// Собрать [`OdfError::Malformed`].
    #[must_use]
    pub fn malformed(part: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Malformed {
            part: part.into(),
            reason: reason.into(),
        }
    }
}

/// Результат разбора ODF.
pub type Result<T> = std::result::Result<T, OdfError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_passes_through() {
        let inner = doc_converter_core::Error::MissingPart("content.xml".into());
        let rendered = inner.to_string();
        let err = OdfError::from(inner);

        assert!(matches!(err, OdfError::Core(_)));
        // `transparent` сохраняет текст нижнего слоя как есть.
        assert_eq!(err.to_string(), rendered);
    }

    #[test]
    fn malformed_keeps_part_and_reason() {
        let err = OdfError::malformed("content.xml", "table has no name");

        assert_eq!(
            err.to_string(),
            "malformed `content.xml`: table has no name"
        );
    }
}
