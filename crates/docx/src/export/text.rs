//! Экспорт документа в обычный текст: содержимое без разметки.
//!
//! Ни начертаний, ни маркеров списков, ни границ таблиц: абзац — строка, ячейки
//! строки таблицы разделены табуляцией, разрывы — перевод строки. Каскад стилей
//! здесь не нужен: текст не зависит от того, чем он набран.

use crate::model::inline::{Inline, Paragraph, RunContent};
use crate::model::table::{Cell, Table};
use crate::model::BlockItem;
use crate::Document;

use super::trim_end_newlines;

/// Экспорт документа в обычный текст.
///
/// Вывод заканчивается переводом строки, если он не пуст.
#[must_use]
pub fn to_text(document: &Document) -> String {
    let mut out = String::new();
    for item in &document.body.items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                out.push_str(&paragraph_text(paragraph));
                out.push('\n');
            }
            BlockItem::Table(table) => table_text(table, &mut out),
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
    trim_end_newlines(&mut out);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Текст одного абзаца: содержимое run'ов, ссылок и полей; разрыв — перевод строки.
#[must_use]
pub fn paragraph_text(paragraph: &Paragraph) -> String {
    let mut out = String::new();
    inline_text(&paragraph.runs, &mut out);
    out
}

/// Текст inline-содержимого по порядку.
fn inline_text(items: &[Inline], out: &mut String) {
    for item in items {
        match item {
            Inline::Run(run) => {
                for content in &run.content {
                    match content {
                        RunContent::Text(text) => out.push_str(text),
                        RunContent::Tab => out.push('\t'),
                        RunContent::Break(_) => out.push('\n'),
                        RunContent::Symbol { char, .. } => out.push(*char),
                        RunContent::Drawing(_) | RunContent::Unknown { .. } => {}
                    }
                }
            }
            Inline::Hyperlink(hyperlink) => inline_text(&hyperlink.runs, out),
            Inline::Field(field) => inline_text(&field.result, out),
            Inline::Tab => out.push('\t'),
            Inline::Break(_) => out.push('\n'),
            Inline::Symbol { char, .. } => out.push(*char),
            Inline::Bookmark(_) | Inline::Drawing(_) | Inline::Unknown { .. } => {}
        }
    }
}

/// Таблица построчно: ячейки строки разделены табуляцией.
fn table_text(table: &Table, out: &mut String) {
    for row in &table.rows {
        let cells: Vec<String> = row.cells.iter().map(cell_text).collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
}

/// Текст ячейки одной строкой: абзацы и вложенные таблицы — через пробел,
/// переводы строк внутри абзаца схлопываются.
#[must_use]
pub fn cell_text(cell: &Cell) -> String {
    let mut parts: Vec<String> = Vec::new();
    for item in &cell.items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                let text = paragraph_text(paragraph).replace('\n', " ");
                if !text.trim().is_empty() {
                    parts.push(text);
                }
            }
            BlockItem::Table(nested) => {
                for row in &nested.rows {
                    for cell in &row.cells {
                        let text = cell_text(cell);
                        if !text.is_empty() {
                            parts.push(text);
                        }
                    }
                }
            }
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
    parts.join(" ")
}
