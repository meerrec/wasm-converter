//! Экспорт документа в Markdown: блочные конструкции CommonMark/GFM и inline-разметка.

use crate::layout::cascade::{ResolvedParagraph, StyleCache};
use crate::model::inline::{BreakKind, Paragraph};
use crate::model::numbering::NumId;
use crate::model::table::{Cell, Table};
use crate::model::BlockItem;
use crate::Document;

use super::{
    heading_level, list_ref, paragraph_layout, segments, text, trim_end_newlines, ListNumbering,
    Marks, Piece, Segment,
};

/// Экспорт документа в Markdown.
///
/// Разметка: заголовки — по уровню структуры абзаца (`#`…`#########`), пункты списков —
/// `- ` или номером уровня, таблицы — GFM-таблицей, у которой первая строка объявлена
/// заголовком, начертания — `**полужирный**`, `*курсив*`, `<u>подчёркнутый</u>`,
/// `~~зачёркнутый~~`, ссылки — `[текст](адрес)`, разрыв строки — обратный слэш в конце
/// строки. Текст экранируется от символов разметки; абзацы и таблицы разделяются пустой
/// строкой.
#[must_use]
pub fn to_markdown(document: &Document) -> String {
    let mut cache = StyleCache::new();
    let mut numbering = ListNumbering::default();
    let mut last_list: Option<NumId> = None;
    let mut out = String::new();
    for item in &document.body.items {
        block(
            document,
            item,
            &mut cache,
            &mut numbering,
            &mut last_list,
            &mut out,
        );
    }
    trim_end_newlines(&mut out);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Блок тела документа: абзац или таблица; `w:sectPr` и неподдержанные блоки
/// разметки не имеют.
fn block(
    document: &Document,
    item: &BlockItem,
    cache: &mut StyleCache,
    numbering: &mut ListNumbering,
    last_list: &mut Option<NumId>,
    out: &mut String,
) {
    match item {
        BlockItem::Paragraph(paragraph) => {
            paragraph_markdown(document, paragraph, cache, numbering, last_list, out);
        }
        BlockItem::Table(table) => {
            *last_list = None;
            table_markdown(document, table, cache, out);
        }
        BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
    }
}

/// Абзац: заголовок, пункт списка или обычный абзац. Пустые абзацы пропускаются —
/// разделителем блоков и так служит пустая строка.
fn paragraph_markdown(
    document: &Document,
    paragraph: &Paragraph,
    cache: &mut StyleCache,
    numbering: &mut ListNumbering,
    last_list: &mut Option<NumId>,
    out: &mut String,
) {
    let resolved = paragraph_layout(document, paragraph, cache);
    let content = inline(document, paragraph, &resolved, cache);
    if content.trim().is_empty() {
        return;
    }
    let text = if let Some(level) = heading_level(paragraph, &resolved) {
        end_block(out);
        *last_list = None;
        format!("{} {content}", "#".repeat(usize::from(level)))
    } else if let Some((num_id, ilvl)) = list_ref(paragraph, &resolved) {
        let indent = "  ".repeat(usize::from(ilvl));
        let marker = numbering.marker(&document.numbering, num_id, ilvl);
        // Пункты одного списка идут подряд, без пустой строки: пустая строка
        // сделала бы список «свободным» и разошлась бы с тем, как список выглядит
        // в документе.
        if *last_list == Some(num_id) {
            out.push('\n');
        } else {
            end_block(out);
        }
        *last_list = Some(num_id);
        format!("{indent}{marker}{content}")
    } else {
        end_block(out);
        *last_list = None;
        escape_block_start(&content)
    };
    out.push_str(&text);
}

/// Таблица: первая строка — заголовок, вторая — разделитель, дальше строки данных.
/// Ячейка-строка собирается из абзацев ячейки через `<br>`; вложенная таблица
/// разворачивается в текст (разметка таблицы внутрь ячейки не вкладывается).
fn table_markdown(document: &Document, table: &Table, cache: &mut StyleCache, out: &mut String) {
    let columns = columns(table);
    if table.rows.is_empty() || columns == 0 {
        return;
    }
    let mut lines: Vec<String> = Vec::with_capacity(table.rows.len() + 1);
    for row in &table.rows {
        let mut cells: Vec<String> = row
            .cells
            .iter()
            .map(|cell| cell_markdown(document, cell, cache))
            .collect();
        cells.resize(columns.max(cells.len()), String::new());
        lines.push(format!("| {} |", cells.join(" | ")));
        if lines.len() == 1 {
            let separator = vec!["---"; columns].join(" | ");
            lines.push(format!("| {separator} |"));
        }
    }
    end_block(out);
    out.push_str(&lines.join("\n"));
}

/// Число столбцов таблицы: по сетке (`w:tblGrid`) и по ячейкам строк с учётом `w:gridSpan`.
fn columns(table: &Table) -> usize {
    let by_cells = table
        .rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| usize::try_from(cell.grid_span).unwrap_or(1))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    by_cells.max(table.grid.len())
}

/// Содержимое ячейки одной строкой таблицы.
fn cell_markdown(document: &Document, cell: &Cell, cache: &mut StyleCache) -> String {
    let mut parts: Vec<String> = Vec::new();
    for item in &cell.items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                let resolved = paragraph_layout(document, paragraph, cache);
                let content = inline(document, paragraph, &resolved, cache);
                let content = content.trim();
                if !content.is_empty() {
                    parts.push(content.to_owned());
                }
            }
            BlockItem::Table(nested) => {
                let flatten = nested
                    .rows
                    .iter()
                    .flat_map(|row| row.cells.iter())
                    .map(text::cell_text)
                    .filter(|cell| !cell.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !flatten.is_empty() {
                    parts.push(escape(&flatten));
                }
            }
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
    parts
        .join("<br>")
        .replace("\\\n", "<br>")
        .replace('\n', "<br>")
}

/// Содержимое абзаца разметкой: `document` нужен каскаду для начертаний и стилей ссылок.
fn inline(
    document: &Document,
    paragraph: &Paragraph,
    resolved: &ResolvedParagraph,
    cache: &mut StyleCache,
) -> String {
    render(&segments(document, resolved, &paragraph.runs, cache))
}

/// Кусок содержимого: экранирование текста, начертания и ссылка.
fn render(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match &segment.piece {
            Piece::Text(text) => out.push_str(&mark(escape(text), segment.marks)),
            Piece::Tab => out.push('\t'),
            // Обратный слэш в конце строки — жёсткий перенос CommonMark; разрыв
            // страницы или колонки переносом не выражается, но текст после него
            // начинает новый абзац.
            Piece::Break(BreakKind::Line | BreakKind::Unsupported(_)) => out.push_str("\\\n"),
            Piece::Break(_) => out.push_str("\n\n"),
            Piece::Image { part, alt } => {
                out.push_str("![");
                out.push_str(&escape(alt));
                out.push_str("](");
                out.push_str(&escape_url(part));
                out.push(')');
            }
            Piece::Link { target, inner } => {
                out.push('[');
                out.push_str(&render(inner));
                out.push_str("](");
                out.push_str(&escape_url(target));
                out.push(')');
            }
        }
    }
    out
}

/// Начертания разметкой; порядок вложенности — от внутренних к внешним.
///
/// Пробелы по краям остаются снаружи маркеров: run в модели кончается пробелом
/// постоянно (`**bold **` — не закрытый `strong` по `CommonMark`), а обёртка вокруг
/// пробелов меняла бы и расстановку пробелов в тексте.
fn mark(text: String, marks: Marks) -> String {
    if marks == Marks::default() {
        return text;
    }
    let core = text.trim();
    if core.is_empty() {
        return text;
    }
    let leading = &text[..text.len() - text.trim_start().len()];
    let trailing = &text[text.trim_end().len()..];
    let mut out = core.to_owned();
    if marks.underline {
        out = format!("<u>{out}</u>");
    }
    if marks.strike {
        out = format!("~~{out}~~");
    }
    if marks.italic {
        out = format!("*{out}*");
    }
    if marks.bold {
        out = format!("**{out}**");
    }
    format!("{leading}{out}{trailing}")
}

/// Экранирует символы, значимые в разметке: `\` `` ` `` `*` `_` `[` `]` `<` `>` `|` `~`.
fn escape(text: &str) -> String {
    const SPECIAL: [char; 9] = ['\\', '`', '*', '_', '[', ']', '<', '>', '|'];
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if SPECIAL.contains(&ch) || ch == '~' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Экранирует символы, которые сломали бы адрес ссылки.
fn escape_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for ch in url.chars() {
        match ch {
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            ' ' => out.push_str("%20"),
            _ => out.push(ch),
        }
    }
    out
}

/// Прячет за обратным слэшем символ в начале строки, который Markdown понял бы как
/// разметку: начало заголовка, списка, цитаты или нумерованного пункта.
fn escape_block_start(text: &str) -> String {
    let bytes = text.as_bytes();
    let marker = match bytes.first() {
        Some(b'#' | b'-' | b'+' | b'>' | b'=') => true,
        Some(b'0'..=b'9') => {
            let digits = bytes
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            matches!(bytes.get(digits), Some(b'.' | b')'))
                && matches!(bytes.get(digits + 1), None | Some(b' ' | b'\t'))
        }
        _ => false,
    };
    if marker {
        format!("\\{text}")
    } else {
        text.to_owned()
    }
}

/// Отделяет блок от предыдущего пустой строкой.
fn end_block(out: &mut String) {
    if out.is_empty() {
        return;
    }
    trim_end_newlines(out);
    out.push_str("\n\n");
}
