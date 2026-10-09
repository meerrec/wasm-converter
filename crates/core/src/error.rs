use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("invalid ZIP archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("XML parse error in `{part}` at byte {position}: {message}")]
    Xml {
        part: String,
        position: u64,
        message: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("missing required OOXML part: `{0}`")]
    MissingPart(String),

    #[error("unresolved relationship target: `{0}`")]
    UnresolvedRel(String),

    #[error("malformed OOXML: {0}")]
    Malformed(String),

    /// Сбой экспорта во внешний формат (PDF, …).
    ///
    /// Ядро не знает, во что экспортируют: формат в сообщении не назван, его
    /// добавляет вызывающий слой.
    #[error("export failed: {0}")]
    Export(String),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    /// Предупреждений парсера стало больше, чем допускает ADR-0016 §6.
    #[error("too many parse warnings: {0}")]
    TooManyWarnings(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_error_reads_as_export_failure() {
        let err = Error::Export("sheet index 7 is out of range".into());

        assert_eq!(
            err.to_string(),
            "export failed: sheet index 7 is out of range"
        );
    }
}
