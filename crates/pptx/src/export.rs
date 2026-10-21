//! Экспорт презентации в текстовый конспект: Markdown, HTML и обычный текст.
//!
//! Конспект — текстовые рамки слайдов по порядку (ADR-0033 §1): заголовок
//! слайда, абзацы, маркеры списков. Картинки и диаграммы в текстовый конспект
//! не входят — они остаются в просмотре и PDF.

use crate::model::{Bullet, Paragraph, Presentation, Run, ShapeKind};

/// Формат текстовой выгрузки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFormat {
    /// Markdown: заголовки, списки.
    Markdown,
    /// Самостоятельная HTML-страница со встроенными стилями.
    Html,
    /// Обычный текст без разметки.
    Text,
}

impl TextFormat {
    /// Формат по строке из интерфейса (`markdown`/`md`, `html`, `text`/`txt`).
    #[must_use]
    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "markdown" | "md" => Some(Self::Markdown),
            "html" => Some(Self::Html),
            "text" | "txt" => Some(Self::Text),
            _ => None,
        }
    }
}

/// Экспортировать презентацию в Markdown.
///
/// Слайд — заголовок второго уровня, абзацы — текст, маркеры — `-` с
/// отступом уровня, автонумерация — `1.`. Пустой вывод только у презентации
/// без текста.
#[must_use]
pub fn to_markdown(presentation: &Presentation) -> String {
    let mut out = String::new();
    for (index, slide) in presentation.slides.iter().enumerate() {
        out.push_str("## ");
        out.push_str(&slide.name);
        out.push('\n');
        for shape in &slide.shapes {
            let ShapeKind::Text(text_box) = &shape.kind else {
                continue;
            };
            for paragraph in &text_box.body.paragraphs {
                out.push_str(&paragraph_markdown(paragraph));
                out.push('\n');
            }
        }
        if index + 1 < presentation.slides.len() {
            out.push('\n');
        }
    }
    out
}

/// Абзац в Markdown: маркер с отступом уровня и текст прогонов.
fn paragraph_markdown(paragraph: &Paragraph) -> String {
    let mut out = String::new();
    match paragraph.bullet {
        Some(Bullet::Char(_)) => {
            out.push_str(&"  ".repeat(usize::from(paragraph.level)));
            out.push_str("- ");
        }
        Some(Bullet::AutoNum { .. }) => {
            out.push_str(&"  ".repeat(usize::from(paragraph.level)));
            out.push_str("1. ");
        }
        None => {}
    }
    out.push_str(&paragraph_text(paragraph));
    out
}

/// Экспортировать презентацию в HTML — самостоятельную страницу.
#[must_use]
pub fn to_html(presentation: &Presentation) -> String {
    let mut out = String::from(
        "<!doctype html>\n<html>\n<head><meta charset=\"utf-8\">\n\
         <style>body{font-family:Calibri,Arial,sans-serif;margin:2em;}\
         h1{font-size:1.4em;border-bottom:1px solid #ccc;padding-bottom:0.3em;}\
         ul{margin:0.4em 0;}li{margin:0.15em 0;}</style>\n</head>\n<body>\n",
    );
    for slide in &presentation.slides {
        out.push_str("<h1>");
        out.push_str(&escape_html(&slide.name));
        out.push_str("</h1>\n");
        let mut list_open = false;
        for shape in &slide.shapes {
            let ShapeKind::Text(text_box) = &shape.kind else {
                continue;
            };
            for paragraph in &text_box.body.paragraphs {
                if paragraph.bullet.is_some() {
                    if !list_open {
                        out.push_str("<ul>\n");
                        list_open = true;
                    }
                    out.push_str("<li>");
                    out.push_str(&escape_html(&paragraph_text(paragraph)));
                    out.push_str("</li>\n");
                } else {
                    if list_open {
                        out.push_str("</ul>\n");
                        list_open = false;
                    }
                    out.push_str("<p>");
                    out.push_str(&escape_html(&paragraph_text(paragraph)));
                    out.push_str("</p>\n");
                }
            }
        }
        if list_open {
            out.push_str("</ul>\n");
        }
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Экспортировать презентацию в обычный текст.
///
/// Слайд — строка имени, абзацы — строки с отступом уровня; между слайдами —
/// пустая строка.
#[must_use]
pub fn to_text(presentation: &Presentation) -> String {
    let mut out = String::new();
    for (index, slide) in presentation.slides.iter().enumerate() {
        out.push_str(&slide.name);
        out.push('\n');
        for shape in &slide.shapes {
            let ShapeKind::Text(text_box) = &shape.kind else {
                continue;
            };
            for paragraph in &text_box.body.paragraphs {
                let mut line = String::new();
                match paragraph.bullet {
                    Some(Bullet::Char(_)) => line.push_str("  - "),
                    Some(Bullet::AutoNum { .. }) => line.push_str("  1. "),
                    None => line.push_str("  "),
                }
                line.push_str(&paragraph_text(paragraph));
                line.push('\n');
                out.push_str(&line);
            }
        }
        if index + 1 < presentation.slides.len() {
            out.push('\n');
        }
    }
    out
}

/// Текст абзаца: прогоны подряд.
#[must_use]
pub fn paragraph_text(paragraph: &Paragraph) -> String {
    let mut out = String::new();
    for run in &paragraph.runs {
        out.push_str(&run.text);
    }
    out
}

/// Текст прогона (для будущих экспортёров начертаний).
#[must_use]
pub fn run_text(run: &Run) -> &str {
    &run.text
}

/// Экранировать HTML-метасимволы.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Frame, Shape, Slide, SlideSize, TextBody, TextBox};

    fn presentation() -> Presentation {
        Presentation {
            size: SlideSize::default(),
            slides: vec![
                Slide {
                    name: "First".into(),
                    shapes: vec![Shape {
                        name: "t".into(),
                        frame: Frame::default(),
                        fill: None,
                        line: None,
                        kind: ShapeKind::Text(TextBox {
                            body: TextBody {
                                paragraphs: vec![
                                    Paragraph {
                                        runs: vec![Run {
                                            text: "Hello & <world>".into(),
                                            ..Run::default()
                                        }],
                                        ..Paragraph::default()
                                    },
                                    Paragraph {
                                        bullet: Some(Bullet::Char('•')),
                                        runs: vec![Run {
                                            text: "point".into(),
                                            ..Run::default()
                                        }],
                                        ..Paragraph::default()
                                    },
                                ],
                                ..TextBody::default()
                            },
                        }),
                    }],
                    ..Slide::default()
                },
                Slide {
                    name: "Second".into(),
                    ..Slide::default()
                },
            ],
            ..Presentation::default()
        }
    }

    #[test]
    fn markdown_outlines_slides_and_bullets() {
        let md = to_markdown(&presentation());
        assert!(md.starts_with("## First\n"));
        assert!(md.contains("Hello & <world>"));
        assert!(md.contains("- point"));
        assert!(md.contains("## Second"));
    }

    #[test]
    fn html_escapes_text() {
        let html = to_html(&presentation());
        assert!(html.contains("Hello &amp; &lt;world&gt;"));
        assert!(html.contains("<h1>First</h1>"));
        assert!(html.contains("<li>point</li>"));
    }

    #[test]
    fn text_keeps_plain_lines() {
        let text = to_text(&presentation());
        assert!(text.starts_with("First\n"));
        assert!(text.contains("  - point"));
        assert!(text.contains("Second\n"));
    }

    #[test]
    fn format_names_parse() {
        assert_eq!(
            TextFormat::from_name("markdown"),
            Some(TextFormat::Markdown)
        );
        assert_eq!(TextFormat::from_name("html"), Some(TextFormat::Html));
        assert_eq!(TextFormat::from_name("txt"), Some(TextFormat::Text));
        assert_eq!(TextFormat::from_name("csv"), None);
    }
}
