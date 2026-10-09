//! Ошибки разбора DOCX (ADR-0016 §2, «фатальные»).
//!
//! Фатальность здесь означает, что продолжать разбор нельзя: без главной части
//! или с нечитаемым XML модель получилась бы недостоверной. Всё, что поддаётся
//! восстановлению, копится в предупреждениях документа, а не здесь.

use miette::Diagnostic;
use thiserror::Error;

/// Ошибка разбора DOCX.
///
/// `non_exhaustive`: набор вариантов будет расти вместе со слайсами парсеров.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    /// Ошибка уровня контейнера OOXML: zip, relationships, чтение XML.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(doc_converter_core::Error),

    /// В пакете нет обязательной части.
    #[error("missing required part: `{0}`")]
    MissingPart(String),

    /// В пакете нет главной части — разбирать нечего.
    #[error("missing `word/document.xml`")]
    MissingDocumentXml,

    /// XML части не разбирается.
    #[error("malformed XML in `{part}` at byte {position}: {message}")]
    XmlFatal {
        /// Часть пакета: `word/document.xml`, `word/styles.xml`, …
        part: String,
        /// Смещение в байтах от начала части.
        position: u64,
        /// Сообщение `quick-xml`.
        message: String,
    },

    /// Документ зашифрован: вместо OOXML-пакета лежит OLE-контейнер.
    #[error("encrypted OOXML documents are not supported")]
    EncryptedDocument,

    /// Документ с макросами: `.docm` вместо `.docx`.
    #[error("macro-enabled documents (.docm) are not supported")]
    MacroEnabledDocument,

    /// Старый бинарный формат `.doc` — не OOXML.
    #[error("legacy binary .doc is not supported")]
    LegacyFormat,

    /// Strict OOXML не поддержан: модель описывает Transitional (ADR-0017).
    #[error("Strict OOXML is not supported (ADR-0017); save as Transitional")]
    StrictNotSupported,

    /// Предупреждений стало больше допустимого — их список оборван, разбор
    /// останавливается (ADR-0016 §6).
    #[error("too many parse warnings: {0}")]
    TooManyWarnings(usize),

    /// Часть разобрана, но нарушает схему настолько, что модель недостоверна.
    #[error("malformed `{part}`: {reason}")]
    Malformed {
        /// Часть пакета.
        part: String,
        /// Что именно нарушено.
        reason: String,
    },
}

/// Результат разбора DOCX.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Короткий конструктор `Malformed` — им пользуются парсеры частей.
    #[must_use]
    pub fn malformed(part: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Malformed {
            part: part.into(),
            reason: reason.into(),
        }
    }
}

// `From` написан вручную, а не выведен через `#[from]`: ошибка
// `TooManyWarnings` из core обязана стать `Error::TooManyWarnings`, а не
// `Error::Core(...)` — на этот вариант завязаны контекст разбора и порог
// показа предупреждений пользователю (ADR-0016 §6).
impl From<doc_converter_core::Error> for Error {
    fn from(err: doc_converter_core::Error) -> Self {
        match err {
            doc_converter_core::Error::TooManyWarnings(count) => Self::TooManyWarnings(count),
            other => Self::Core(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn malformed_keeps_part_and_reason() {
        let err = Error::malformed("word/document.xml", "unexpected end of input");
        assert_eq!(
            err.to_string(),
            "malformed `word/document.xml`: unexpected end of input"
        );
    }

    #[test]
    fn too_many_warnings_stays_its_own_variant() {
        let err = Error::from(doc_converter_core::Error::TooManyWarnings(1000));
        assert!(matches!(err, Error::TooManyWarnings(1000)));
    }

    #[test]
    fn other_core_errors_are_wrapped_as_core() {
        let err = Error::from(doc_converter_core::Error::MissingPart(
            "word/document.xml".to_owned(),
        ));
        assert!(matches!(err, Error::Core(_)));
    }
}
