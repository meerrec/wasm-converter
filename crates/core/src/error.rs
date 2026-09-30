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

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

pub type Result<T> = std::result::Result<T, Error>;
