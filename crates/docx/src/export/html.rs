//! Экспорт документа в HTML.
//!
//! Заглушка на шов: HTML — отдельный слайс, и заполняет он только этот файл.
//! Общие помощники уже здесь: `Marks`, `Piece`, `Segment`, `paragraph_layout`,
//! `segments`, `heading_level`, `list_ref` и `ListNumbering`.

use crate::Document;

/// Экспорт документа в HTML.
///
/// Пока заглушка: слайс HTML пишется отдельно и возвращает `String::new()`
/// только до тех пор, пока не реализован.
#[must_use]
pub fn to_html(document: &Document) -> String {
    let _ = document;
    // TODO (Спринт 10): HTML — отдельный слайс.
    String::new()
}
