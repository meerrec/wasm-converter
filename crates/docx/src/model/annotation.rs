//! Сноски и комментарии: `word/footnotes.xml`, `word/endnotes.xml`, `word/comments.xml`.

use doc_converter_core::NodeId;
use serde::{Deserialize, Serialize};

use super::raw::RawRPr;
use super::BlockItem;

/// Сноска или концевая сноска (`w:footnote`/`w:endnote`).
///
/// Служебные части (разделители и продолжение) приходят тем же элементом с отрицательным
/// `w:id` и различаются по `kind`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Footnote {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Номер из XML (`w:id`); у служебных частей Word он отрицательный.
    pub note_id: i32,
    /// Вид сноски.
    pub kind: NoteKind,
    /// Содержимое сноски: блоки те же, что и в теле документа.
    pub body: Vec<BlockItem>,
    /// Свойства знака сноски (`w:rPr` внутри `w:footnote`/`w:endnote`).
    pub mark_rpr: RawRPr,
}

/// Комментарий (`w:comment`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Comment {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Номер комментария из XML (`w:id`).
    pub comment_id: i32,
    /// Автор (`w:author`).
    pub author: Option<String>,
    /// Инициалы автора (`w:initials`).
    pub initials: Option<String>,
    /// Дата в том виде, как записана в XML (`w:date`, ISO-8601).
    ///
    /// Хранится строкой, а не через `chrono`: дата нужна только для показа и snapshot-тестов.
    pub date: Option<String>,
    /// Содержимое комментария.
    pub body: Vec<BlockItem>,
}

/// Вид сноски: обычная, концевая или служебная часть Word.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// Обычная сноска внизу страницы.
    Footnote,
    /// Концевая сноска.
    Endnote,
    /// Линия-разделитель между текстом и сносками.
    Separator,
    /// Разделитель продолжения сносок на следующей странице.
    ContinuationSeparator,
    /// Уведомление о продолжении сносок.
    ContinuationNotice,
}
