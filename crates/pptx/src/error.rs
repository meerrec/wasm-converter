//! Ошибки разбора, раскладки и экспорта PPTX.
//!
//! Ошибки ядра (`doc_converter_core::Error` — ZIP, лимиты, XML) проходят
//! прозрачно; собственные варианты — про структуру `PresentationML` и модель.

/// Ошибка PPTX.
///
/// `non_exhaustive`: набор вариантов будет расти вместе со слайсами парсеров.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Ошибка ядра: ZIP, лимиты ADR-0015, XML, relationships.
    #[error(transparent)]
    Core(#[from] doc_converter_core::Error),

    /// XML разобран, но структура `PresentationML` не сошлась.
    #[error("pptx: {0}")]
    Malformed(String),

    /// В презентации нет ни одного слайда: для PPTX это битый пакет.
    #[error("presentation has no slides")]
    NoSlides,

    /// Слайда с таким индексом нет.
    #[error("slide index {0} is out of range")]
    NoSuchSlide(usize),

    /// Операция отменена вызовом (ADR-0032): кооперативная отмена по флагу.
    #[error("operation cancelled")]
    Cancelled,
}

/// Результат операций крейта.
pub type Result<T> = std::result::Result<T, Error>;
