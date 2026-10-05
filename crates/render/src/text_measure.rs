//! Измерение и перенос текста поверх [`crate::font::FontRegistry`].
//!
//! Единый источник ширин для canvas и PDF (ADR-0005): обе стороны зовут один
//! и тот же код, поэтому точки разрыва у них не разъезжаются. Ширина строки —
//! сумма advance'ов глифов; перенос — жадный word wrap, детерминированный и
//! без shaping: кернинг и лигатуры — надстройка следующих фаз.

use std::ops::Range;

use crate::font::{FontId, FontRegistry};

/// Ширина строки в пикселях при заданном кегле.
#[must_use]
pub fn measure_text(fonts: &mut FontRegistry, font_id: FontId, size_px: f32, text: &str) -> f32 {
    fonts.measure(font_id, size_px, text).width
}

/// Разбивает текст на строки, которые помещаются в `max_width`.
///
/// Диапазоны — байтовые (`Range<usize>`) по исходному `text`, без пробелов по
/// краям: разрыв ставится только между словами. Правила жадного переноса:
///
/// - слова — группы не-whitespace символов, соседние пробелы склеиваются;
/// - `\n` форсирует разрыв;
/// - слово, не влезающее в остаток строки, переносится на следующую;
/// - слово, шире всей строки, режется по символам (каждый кусок непустой,
///   кусок шире `max_width` остаётся один);
/// - пустой или только из пробелов текст и `max_width <= 0` дают пустой
///   результат.
#[must_use]
pub fn break_lines(
    fonts: &mut FontRegistry,
    font_id: FontId,
    size_px: f32,
    text: &str,
    max_width: f32,
) -> Vec<Range<usize>> {
    if text.is_empty() || max_width <= 0.0 {
        return Vec::new();
    }

    let space = measure_text(fonts, font_id, size_px, " ");

    let mut lines = Vec::new();
    // Байтовый диапазон собираемой строки: от первого символа первого слова
    // до последнего символа последнего; пробелы в диапазон не входят.
    let mut line_start: Option<usize> = None;
    let mut line_end = 0usize;
    let mut line_width = 0.0;

    let mut byte = 0usize;
    while byte < text.len() {
        let ch = text[byte..]
            .chars()
            .next()
            .expect("byte is a char boundary");
        if ch == '\n' {
            byte += 1;
            if let Some(start) = line_start.take() {
                lines.push(start..line_end);
            }
            line_width = 0.0;
            continue;
        }
        if ch.is_whitespace() {
            byte += ch.len_utf8();
            continue;
        }

        // Слово — до первого пробела или перевода строки.
        let word_start = byte;
        let mut word_end = byte + ch.len_utf8();
        while word_end < text.len() {
            let next = text[word_end..]
                .chars()
                .next()
                .expect("byte is a char boundary");
            if next.is_whitespace() {
                break;
            }
            word_end += next.len_utf8();
        }
        let width = measure_text(fonts, font_id, size_px, &text[word_start..word_end]);
        byte = word_end;

        // Слово встаёт в текущую строку; слишком широкое для пустой строки
        // режется посимвольно, для занятой — открывает новую.
        let fits = width <= max_width;
        match line_start {
            None => {
                if fits {
                    line_start = Some(word_start);
                    line_end = word_end;
                    line_width = width;
                } else {
                    lines.extend(break_word(
                        fonts,
                        font_id,
                        size_px,
                        &text[word_start..word_end],
                        word_start,
                        max_width,
                    ));
                }
            }
            Some(start) => {
                if line_width + space + width <= max_width {
                    line_width += space + width;
                    line_end = word_end;
                } else {
                    lines.push(start..line_end);
                    line_start = None;
                    line_width = 0.0;
                    if fits {
                        line_start = Some(word_start);
                        line_end = word_end;
                        line_width = width;
                    } else {
                        lines.extend(break_word(
                            fonts,
                            font_id,
                            size_px,
                            &text[word_start..word_end],
                            word_start,
                            max_width,
                        ));
                    }
                }
            }
        }
    }
    if let Some(start) = line_start {
        lines.push(start..line_end);
    }
    lines
}

/// Режет слово шире `max_width` по символам; каждый кусок непустой.
///
/// `base` — байтовое смещение слова в исходной строке: возвращаются абсолютные
/// диапазоны, пригодные для `&text[range]`.
fn break_word(
    fonts: &mut FontRegistry,
    font_id: FontId,
    size_px: f32,
    word: &str,
    base: usize,
    max_width: f32,
) -> Vec<Range<usize>> {
    let mut chunks = Vec::new();
    let mut chunk_start: Option<usize> = None;
    let mut chunk_end = 0usize;
    let mut chunk_width = 0.0;

    for (byte, ch) in word.char_indices() {
        let end = byte + ch.len_utf8();
        let advance = measure_text(fonts, font_id, size_px, &word[byte..end]);
        if chunk_start.is_some() && chunk_width + advance > max_width {
            chunks.push(base + chunk_start.expect("chunk has a start")..base + chunk_end);
            chunk_start = None;
            chunk_width = 0.0;
        }
        // Один символ шире строки всё равно образует кусок.
        if chunk_start.is_none() {
            chunk_start = Some(byte);
            chunk_end = end;
        } else {
            chunk_end = end;
        }
        chunk_width += advance;
    }
    if let Some(start) = chunk_start {
        chunks.push(base + start..base + chunk_end);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::DEFAULT_FONT_ID;

    const PX_11PT: f32 = 11.0 * 96.0 / 72.0;

    fn ranges<'a>(text: &'a str, lines: &[Range<usize>]) -> Vec<&'a str> {
        lines.iter().map(|r| &text[r.clone()]).collect()
    }

    #[test]
    fn short_text_is_one_line() {
        let mut fonts = FontRegistry::new(64);
        let text = "привет";
        let lines = break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, text, 1000.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(ranges(text, &lines), vec!["привет"]);
    }

    #[test]
    fn wraps_on_words() {
        let mut fonts = FontRegistry::new(64);
        let text = "aa bb cc";
        // «aa bb» влезает ровно впритык, «aa bb cc» — уже нет.
        let max = measure_text(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "aa bb");
        let lines = break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, text, max);
        assert_eq!(lines.len(), 2);
        assert_eq!(ranges(text, &lines), vec!["aa bb", "cc"]);
    }

    #[test]
    fn long_word_breaks_by_char() {
        let mut fonts = FontRegistry::new(64);
        let text = "abcdef";
        // Два символа влезают, третий — нет.
        let max = measure_text(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "ab");
        let lines = break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, text, max);
        assert_eq!(ranges(text, &lines), vec!["ab", "cd", "ef"]);
    }

    #[test]
    fn newline_forces_break() {
        let mut fonts = FontRegistry::new(64);
        let text = "aa\nbb";
        let lines = break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, text, 1000.0);
        assert_eq!(ranges(text, &lines), vec!["aa", "bb"]);
    }

    #[test]
    fn whitespace_only_is_empty() {
        let mut fonts = FontRegistry::new(64);
        assert!(break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "  \n\t ", 100.0).is_empty());
        assert!(break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "abc", 0.0).is_empty());
        assert!(break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "", 100.0).is_empty());
    }

    #[test]
    fn ranges_stay_on_utf8_boundaries() {
        let mut fonts = FontRegistry::new(64);
        let text = "файл большой";
        let max = measure_text(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "большой");
        let lines = break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, text, max);
        for line in &lines {
            assert!(text.is_char_boundary(line.start));
            assert!(text.is_char_boundary(line.end));
        }
        assert_eq!(ranges(text, &lines), vec!["файл", "большой"]);
    }

    #[test]
    fn chunk_wider_than_max_stays_alone() {
        let mut fonts = FontRegistry::new(64);
        let text = "ab";
        // Ширина одного символа больше max — кусок всё равно из одного символа.
        let max = measure_text(&mut fonts, DEFAULT_FONT_ID, PX_11PT, "a") - 0.01;
        let lines = break_lines(&mut fonts, DEFAULT_FONT_ID, PX_11PT, text, max);
        assert_eq!(ranges(text, &lines), vec!["a", "b"]);
    }
}
