//! Общий сбор начертаний и символов для подрезки шрифтов.
//!
//! Один обход нужен всем путям экспорта: DOCX и XLSX внутри крейта и PPTX
//! отдельным крейтом (`doc-converter-pptx-pdf`, ADR-0033). Копия разошлась бы
//! с оригиналом на первой же правке (например, когда у [`Op::WriteText`]
//! появится новое поле), поэтому модуль публичен ровно в том объёме, в котором
//! его требует PPTX, — как и остальной постраничный бэкенд.

use std::collections::{BTreeMap, BTreeSet};

use crate::fonts::{Face, FaceSet};
use crate::pdf::ops::{FontId, Op, TextItem};

/// Начертания из пар (полужирный, курсив).
///
/// Обход элементов раскладки остаётся у вызывающего: у DOCX и PPTX он разный
/// (рекурсия по ячейкам таблиц против строк текста), общий у них только способ
/// вставки — [`Face::of`] и [`FaceSet::insert`], который здесь и живёт.
#[must_use]
pub fn collect_faces(faces: impl IntoIterator<Item = (bool, bool)>) -> FaceSet {
    let mut set = FaceSet::default();
    for (bold, italic) in faces {
        set.insert(Face::of(bold, italic));
    }
    set
}

/// Символы текста команд страницы по начертаниям.
///
/// Ключ — [`Face`], а не [`FontId`]: зонды идут до сессии, и id шрифтов ещё не
/// существует. Обратное сопоставление делает `face_of`: у DOCX и PPTX это
/// [`EmbeddedFonts::face`](crate::fonts::EmbeddedFonts::face), у XLSX — тот же
/// метод на его `fonts`.
///
/// Источник — те же операции, что уйдут в запись: пре-скан текста по элементам
/// раскладки разошёлся бы с отрисовкой при первой же правке.
#[must_use]
pub fn collect_text_glyphs<'a>(
    ops: impl IntoIterator<Item = &'a Op>,
    face_of: impl Fn(&FontId) -> Face,
) -> BTreeMap<Face, BTreeSet<char>> {
    let mut glyphs: BTreeMap<Face, BTreeSet<char>> = BTreeMap::new();
    for op in ops {
        if let Op::WriteText { font, items, .. } = op {
            let set = glyphs.entry(face_of(font)).or_default();
            for item in items {
                if let TextItem::Text(text) = item {
                    set.extend(text.chars());
                }
            }
        }
    }
    glyphs
}
