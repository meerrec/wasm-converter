//! Разбор `word/comments.xml` (слайс S11): комментарии и их тела.
//!
//! Тело комментария — те же блоки, что и у документа, поэтому читает его общий
//! [`parse_blocks`], а не второй блочный парсер. Автор, инициалы и дата лежат
//! атрибутами `w:comment` и переносятся в модель как есть.

// Вызывающего у парсера ещё нет: его подключит `parse.rs` (слайс S12). До тех
// пор `dead_code` срабатывал бы на каждом элементе модуля — как в `document.rs`.
#![allow(dead_code)]

use std::collections::HashSet;

use doc_converter_core::xml::XmlReader;
use doc_converter_core::WarningKind;
use quick_xml::events::{BytesStart, Event};

use crate::context::ParseCtx;
use crate::document::parse_blocks;
use crate::error::{Error, Result};
use crate::model::{Comment, Relationships};
use crate::xml::{attr_i32, attributes, capture_element, find, local_name, Attr};

/// Имя контейнера части.
const CONTAINER: &str = "w:comments";

/// `xml_path` предупреждений внутри части.
const COMMENT_PATH: &str = "w:comments/w:comment";

/// Разобрать `word/comments.xml` в список комментариев.
///
/// # Errors
/// [`Error::Malformed`] — в части нет контейнера `w:comments` или поток оборвался
/// раньше закрывающего тега; [`Error::TooManyWarnings`] — предупреждений стало
/// больше порога.
pub(crate) fn parse(
    bytes: &[u8],
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<Vec<Comment>> {
    let mut reader = XmlReader::preserving(bytes, part);
    if !open_container(&mut reader, part)? {
        return Ok(Vec::new());
    }
    let mut comments = Vec::new();
    let mut seen: HashSet<i32> = HashSet::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                format!("unexpected end of input inside `{CONTAINER}`"),
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(comments),
            _ => continue,
        };
        if local_name(element.name().into_inner()) != b"comment" {
            // У контейнера есть и служебные дети (`w:commentExtensible`,
            // `w:extLst`): тел в них нет, и предупреждать о них значило бы
            // шуметь на валидных файлах. Поддерево всё равно дочитывается —
            // иначе `next_significant` вернул бы чужой `End`.
            skip_element(&mut reader, element, empty, ctx, part)?;
            continue;
        }
        let attrs = attributes(element, part)?;
        let Some(comment_id) = annotation_id(&attrs, ctx, part, element)? else {
            skip_element(&mut reader, element, empty, ctx, part)?;
            continue;
        };
        if !seen.insert(comment_id) {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!(
                    "`{}`: duplicate `w:id` {comment_id}, the first comment wins",
                    element_name(element)
                ),
            )?;
            skip_element(&mut reader, element, empty, ctx, part)?;
            continue;
        }
        // ID комментария — до ID его тела: нумерация идёт в порядке обхода XML
        // (ADR-0019 §2), а `w:comment` стоит в документе раньше своих абзацев.
        let id = ctx.id();
        let body = if empty {
            Vec::new()
        } else {
            parse_blocks(&mut reader, rels, ctx, part, COMMENT_PATH)?
        };
        comments.push(Comment {
            id,
            comment_id,
            author: find(&attrs, "author").map(str::to_owned),
            initials: find(&attrs, "initials").map(str::to_owned),
            date: find(&attrs, "date").map(str::to_owned),
            body,
        });
    }
}

/// Найти корневой контейнер `w:comments`.
///
/// `Ok(false)` — контейнер пуст (`<w:comments/>`): комментариев в нём нет.
///
/// # Errors
/// [`Error::Malformed`] — первый элемент не `w:comments` или часть пуста.
fn open_container(reader: &mut XmlReader<'_>, part: &str) -> Result<bool> {
    while let Some(event) = reader.next_significant()? {
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            _ => continue,
        };
        if local_name(element.name().into_inner()) != b"comments" {
            return Err(Error::malformed(
                part,
                format!(
                    "expected `w:comments`, found `{}`",
                    String::from_utf8_lossy(element.name().into_inner())
                ),
            ));
        }
        return Ok(!empty);
    }
    Err(Error::malformed(part, "`w:comments` is missing"))
}

/// Дочитать пропускаемый элемент до его `End`.
///
/// `capture_element` возвращает XML поддерева, но здесь он нужен ради побочного
/// эффекта: без него `next_significant` вернул бы вызывающему чужой `End`.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn skip_element(
    reader: &mut XmlReader<'_>,
    element: &BytesStart<'_>,
    empty: bool,
    ctx: &mut ParseCtx,
    part: &str,
) -> Result<()> {
    if !empty {
        capture_element(reader, element, ctx, part)?;
    }
    Ok(())
}

/// Число из `w:id`; `None` — комментарий пропускается.
///
/// Невалидное значение [`attr_i32`] уже отметил предупреждением, а вот
/// отсутствие атрибута — отдельный случай: без `w:id` комментарий не
/// привязывается к тексту, и терять его молча нельзя.
///
/// # Errors
/// То же, что у [`parse_blocks`].
fn annotation_id(
    attrs: &[Attr<'_>],
    ctx: &mut ParseCtx,
    part: &str,
    element: &BytesStart<'_>,
) -> Result<Option<i32>> {
    if let Some(id) = attr_i32(attrs, "id", ctx, part)? {
        return Ok(Some(id));
    }
    if find(attrs, "id").is_none() {
        ctx.warn(
            WarningKind::InvalidAttribute,
            part,
            format!("`{}` has no `w:id`, skipped", element_name(element)),
        )?;
    }
    Ok(None)
}

/// Имя элемента с префиксом — для сообщений предупреждений.
fn element_name(element: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(element.name().into_inner()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockItem, Inline, Paragraph, RunContent};

    const PART: &str = "word/comments.xml";

    /// Каталог фикстур со сносками и комментариями.
    fn fixtures_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx/notes")
    }

    /// Часть `part` из фикстуры `name.docx` вместе с её сайкаром.
    fn fixture(name: &str, part: &str) -> (Vec<u8>, serde_json::Value) {
        let path = fixtures_root().join(format!("{name}.docx"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut archive = doc_converter_core::Archive::new(bytes)
            .unwrap_or_else(|e| panic!("{name}: пакет не открывается: {e}"));
        let xml = archive
            .read(part)
            .unwrap_or_else(|e| panic!("{name}: `{part}`: {e}"));
        let path = fixtures_root().join(format!("{name}.json"));
        let sidecar =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let sidecar = serde_json::from_str(&sidecar).unwrap_or_else(|e| panic!("{name}: {e}"));
        (xml, sidecar)
    }

    /// Разобрать часть.
    fn parse_comments(xml: &[u8], part: &str) -> (Vec<Comment>, ParseCtx) {
        let mut ctx = ParseCtx::new();
        let comments = parse(xml, &Relationships::default(), &mut ctx, part)
            .unwrap_or_else(|e| panic!("{part}: {e}"));
        (comments, ctx)
    }

    /// Плоский текст абзаца: сайкары сверяют именно его.
    fn paragraph_text(paragraph: &Paragraph) -> String {
        let mut text = String::new();
        for inline in &paragraph.runs {
            let Inline::Run(run) = inline else { continue };
            for content in &run.content {
                if let RunContent::Text(chunk) = content {
                    text.push_str(chunk);
                }
            }
        }
        text.trim().to_owned()
    }

    /// Тексты абзацев тела — по одному на абзац, в порядке следования.
    fn paragraph_texts(body: &[BlockItem]) -> Vec<String> {
        body.iter()
            .filter_map(|item| match item {
                BlockItem::Paragraph(paragraph) => Some(paragraph_text(paragraph)),
                _ => None,
            })
            .collect()
    }

    /// ID, выданные как атрибут, а не разбором тела.
    fn invalid_attribute_ids(ctx: &ParseCtx) -> Vec<&str> {
        ctx.warnings()
            .iter()
            .filter(|warning| warning.kind == WarningKind::InvalidAttribute)
            .map(|warning| warning.message.as_str())
            .collect()
    }

    /// Комментарий с автором, инициалами и датой — сверка с сайкаром.
    #[test]
    fn a_comment_matches_its_sidecar() {
        let (xml, sidecar) = fixture("comments_basic", PART);
        let (comments, ctx) = parse_comments(&xml, PART);

        assert_eq!(comments.len(), 1);
        let expected = sidecar["content"]["comments"]
            .as_array()
            .expect("сайкар: `content.comments`");
        assert_eq!(comments.len(), expected.len(), "число комментариев");
        for (comment, want) in comments.iter().zip(expected) {
            assert_eq!(
                i64::from(comment.comment_id),
                want["id"].as_i64().expect("сайкар: `id`"),
            );
            assert_eq!(
                comment.author.as_deref(),
                Some(want["author"].as_str().expect("сайкар: `author`")),
            );
            assert_eq!(
                comment.date.as_deref(),
                Some(want["date"].as_str().expect("сайкар: `date`")),
            );
            assert_eq!(
                paragraph_texts(&comment.body),
                vec![want["text"].as_str().expect("сайкар: `text`").to_owned()],
            );
        }
        assert_eq!(comments[0].initials.as_deref(), Some("FA"));
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// Три комментария фикстуры: свои id, авторы и тексты у каждого.
    #[test]
    fn several_comments_match_their_sidecar() {
        let (xml, sidecar) = fixture("comments_multiple", PART);
        let (comments, ctx) = parse_comments(&xml, PART);

        let expected = sidecar["content"]["comments"]
            .as_array()
            .expect("сайкар: `content.comments`");
        assert_eq!(comments.len(), expected.len());
        assert_eq!(
            comments
                .iter()
                .map(|comment| comment.comment_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3],
        );
        for (comment, want) in comments.iter().zip(expected) {
            assert_eq!(
                paragraph_texts(&comment.body),
                vec![want["text"].as_str().expect("сайкар: `text`").to_owned()],
            );
        }
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// Тело комментария идёт через `parse_blocks`: абзацев может быть несколько.
    #[test]
    fn a_body_with_several_paragraphs_is_parsed_as_blocks() {
        let xml = br#"<w:comments><w:comment w:id="1">
            <w:p><w:r><w:t>First.</w:t></w:r></w:p>
            <w:p><w:r><w:t>Second.</w:t></w:r></w:p>
        </w:comment></w:comments>"#;
        let (comments, _) = parse_comments(xml, PART);

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].body.len(), 2);
        assert_eq!(
            paragraph_texts(&comments[0].body),
            vec!["First.", "Second."]
        );
    }

    /// Атрибутов может и не быть: тогда поля пусты, а не выдуманы.
    #[test]
    fn a_comment_without_the_optional_attributes_has_empty_fields() {
        let xml = br#"<w:comments><w:comment w:id="4">
            <w:p><w:r><w:t>No metadata.</w:t></w:r></w:p>
        </w:comment></w:comments>"#;
        let (comments, ctx) = parse_comments(xml, PART);

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].comment_id, 4);
        assert_eq!(comments[0].author, None);
        assert_eq!(comments[0].initials, None);
        assert_eq!(comments[0].date, None);
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// ID комментария выдаётся раньше ID его тела: нумерация идёт по XML.
    #[test]
    fn a_comment_id_precedes_the_ids_of_its_body() {
        let (xml, _) = fixture("comments_basic", PART);
        let (comments, _) = parse_comments(&xml, PART);

        let BlockItem::Paragraph(paragraph) = &comments[0].body[0] else {
            panic!("первый блок комментария — абзац");
        };
        assert!(
            comments[0].id < paragraph.id,
            "{:?} < {:?}",
            comments[0].id,
            paragraph.id
        );
    }

    /// Дубликат `w:id`: предупреждение, первый побеждает.
    #[test]
    fn a_duplicate_id_is_warned_and_the_first_comment_wins() {
        let xml = br#"<w:comments>
            <w:comment w:id="7"><w:p><w:r><w:t>First.</w:t></w:r></w:p></w:comment>
            <w:comment w:id="7"><w:p><w:r><w:t>Second.</w:t></w:r></w:p></w:comment>
        </w:comments>"#;
        let (comments, ctx) = parse_comments(xml, PART);

        assert_eq!(comments.len(), 1);
        assert_eq!(paragraph_texts(&comments[0].body), vec!["First."]);
        let warnings = ctx.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::InvalidAttribute);
        assert!(
            warnings[0].message.contains("duplicate"),
            "{}",
            warnings[0].message
        );
    }

    /// Невалидный `w:id`: предупреждение, комментарий пропускается.
    #[test]
    fn an_invalid_id_is_warned_and_the_comment_is_skipped() {
        let xml = br#"<w:comments>
            <w:comment w:id="not a number"><w:p><w:r><w:t>Lost.</w:t></w:r></w:p></w:comment>
            <w:comment><w:p><w:r><w:t>Also lost.</w:t></w:r></w:p></w:comment>
            <w:comment w:id="2"><w:p><w:r><w:t>Kept.</w:t></w:r></w:p></w:comment>
        </w:comments>"#;
        let (comments, ctx) = parse_comments(xml, PART);

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].comment_id, 2);
        assert_eq!(paragraph_texts(&comments[0].body), vec!["Kept."]);
        let ids = invalid_attribute_ids(&ctx);
        assert_eq!(ids.len(), 2, "{:?}", ctx.warnings());
        assert!(ids[0].contains("id"), "{}", ids[0]);
        assert!(ids[1].contains("w:id"), "{}", ids[1]);
    }

    /// Пустой контейнер — пустой вектор, а не ошибка.
    #[test]
    fn an_empty_container_gives_no_comments() {
        for xml in [&b"<w:comments/>"[..], b"<w:comments></w:comments>"] {
            let mut ctx = ParseCtx::new();
            let comments = parse(xml, &Relationships::default(), &mut ctx, PART)
                .expect("пустой контейнер разбирается");
            assert!(comments.is_empty(), "{}", String::from_utf8_lossy(xml));
            assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
        }
    }

    /// Незнакомый ребёнок контейнера пропускается, а не теряет поток.
    #[test]
    fn an_unknown_child_is_skipped_without_losing_the_next_comment() {
        let xml = br#"<w:comments>
            <w:commentExtensible><w:extLst/></w:commentExtensible>
            <w:comment w:id="1"><w:p><w:r><w:t>After.</w:t></w:r></w:p></w:comment>
        </w:comments>"#;
        let (comments, ctx) = parse_comments(xml, PART);

        assert_eq!(comments.len(), 1);
        assert_eq!(paragraph_texts(&comments[0].body), vec!["After."]);
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    /// Битый XML — фатальная ошибка, а не частичный результат.
    #[test]
    fn broken_xml_is_fatal() {
        // Поток оборвался внутри комментария.
        let err = parse(
            b"<w:comments><w:comment w:id=\"1\"><w:p>",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
        )
        .expect_err("обрыв потока не разбирается");
        assert!(matches!(err, Error::Malformed { .. }), "{err}");

        // Не закрыт сам тег — на этом спотыкается quick-xml.
        parse(
            b"<w:comments",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
        )
        .expect_err("незакрытый тег не разбирается");
    }

    /// Часть без контейнера — ошибка: разбирать нечего.
    #[test]
    fn a_part_without_a_container_is_malformed() {
        let err = parse(
            b"<w:footnotes/>",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
        )
        .expect_err("без `w:comments` разбирать нечего");
        assert!(err.to_string().contains("w:comments"), "{err}");
    }
}
