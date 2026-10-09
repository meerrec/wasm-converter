//! Разбор `word/footnotes.xml` и `word/endnotes.xml` (слайс S11): сноски и их метки.
//!
//! Оба файла разбирает одна функция: различает их только вид сноски по
//! умолчанию — [`NoteKind::Footnote`] для `w:footnote` из `word/footnotes.xml`
//! и [`NoteKind::Endnote`] для `w:endnote` из `word/endnotes.xml`. Тело сноски —
//! те же блоки, что и у документа, поэтому читает его общий [`parse_blocks`], а
//! не второй блочный парсер.

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
use crate::model::{BlockItem, Footnote, NoteKind, RawRPr, Relationships};
use crate::xml::{attr_i32, attributes, capture_element, find, local_name, Attr};

/// `xml_path` предупреждений внутри `word/footnotes.xml`.
const FOOTNOTES_PATH: &str = "w:footnotes/w:footnote";

/// `xml_path` предупреждений внутри `word/endnotes.xml`.
const ENDNOTES_PATH: &str = "w:endnotes/w:endnote";

/// Имена контейнера части и его детей.
struct Container {
    /// Имя контейнера с префиксом — для сообщений об ошибках.
    name: &'static str,
    /// Путь до сноски — `xml_path` для [`parse_blocks`].
    child_path: &'static str,
}

/// Разобрать `word/footnotes.xml` (`default_kind` — [`NoteKind::Footnote`]) или
/// `word/endnotes.xml` ([`NoteKind::Endnote`]) в список сносок.
///
/// # Errors
/// [`Error::Malformed`] — в части нет контейнера `w:footnotes`/`w:endnotes` или
/// поток оборвался раньше закрывающего тега; [`Error::TooManyWarnings`] —
/// предупреждений стало больше порога.
pub(crate) fn parse(
    bytes: &[u8],
    rels: &Relationships,
    ctx: &mut ParseCtx,
    part: &str,
    default_kind: NoteKind,
) -> Result<Vec<Footnote>> {
    let mut reader = XmlReader::preserving(bytes, part);
    let Some(container) = find_container(&mut reader, part)? else {
        return Ok(Vec::new());
    };
    let mut notes = Vec::new();
    let mut seen: HashSet<i32> = HashSet::new();
    loop {
        let Some(event) = reader.next_significant()? else {
            return Err(Error::malformed(
                part,
                format!("unexpected end of input inside `{}`", container.name),
            ));
        };
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => return Ok(notes),
            _ => continue,
        };
        if !matches!(
            local_name(element.name().into_inner()),
            b"footnote" | b"endnote"
        ) {
            // У контейнера есть и служебные дети (`w:footnotePr`, `w:endnotePr`,
            // `w:extLst`): тел в них нет, и предупреждать о них значило бы шуметь
            // на валидных файлах. Поддерево всё равно дочитывается — иначе
            // `next_significant` вернул бы чужой `End`.
            skip_element(&mut reader, element, empty, ctx, part)?;
            continue;
        }
        let attrs = attributes(element, part)?;
        let Some(note_id) = annotation_id(&attrs, ctx, part, element)? else {
            skip_element(&mut reader, element, empty, ctx, part)?;
            continue;
        };
        if !seen.insert(note_id) {
            ctx.warn(
                WarningKind::InvalidAttribute,
                part,
                format!(
                    "`{}`: duplicate `w:id` {note_id}, the first note wins",
                    element_name(element)
                ),
            )?;
            skip_element(&mut reader, element, empty, ctx, part)?;
            continue;
        }
        let kind = note_kind(&attrs, default_kind);
        // ID сноски — до ID её тела: нумерация идёт в порядке обхода XML
        // (ADR-0019 §2), а `w:footnote` стоит в документе раньше своих абзацев.
        let id = ctx.id();
        let body = if empty {
            Vec::new()
        } else {
            parse_blocks(&mut reader, rels, ctx, part, container.child_path)?
        };
        notes.push(Footnote {
            id,
            note_id,
            kind,
            mark_rpr: mark_rpr(&body),
            body,
        });
    }
}

/// Найти корневой контейнер части: `w:footnotes` или `w:endnotes`.
///
/// `Ok(None)` — контейнер пуст (`<w:footnotes/>`): сносок в нём нет.
///
/// # Errors
/// [`Error::Malformed`] — первый элемент не контейнер сносок или часть пуста.
fn find_container(reader: &mut XmlReader<'_>, part: &str) -> Result<Option<Container>> {
    while let Some(event) = reader.next_significant()? {
        let (element, empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            _ => continue,
        };
        let container = match local_name(element.name().into_inner()) {
            b"footnotes" => Container {
                name: "w:footnotes",
                child_path: FOOTNOTES_PATH,
            },
            b"endnotes" => Container {
                name: "w:endnotes",
                child_path: ENDNOTES_PATH,
            },
            other => {
                return Err(Error::malformed(
                    part,
                    format!(
                        "expected `w:footnotes` or `w:endnotes`, found `{}`",
                        String::from_utf8_lossy(other)
                    ),
                ));
            }
        };
        return Ok(if empty { None } else { Some(container) });
    }
    Err(Error::malformed(
        part,
        "`w:footnotes` or `w:endnotes` is missing",
    ))
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

/// Число из `w:id`; `None` — сноска пропускается.
///
/// Невалидное значение [`attr_i32`] уже отметил предупреждением, а вот
/// отсутствие атрибута — отдельный случай: без `w:id` сноска не привязывается к
/// тексту, и терять её молча нельзя.
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

/// Вид сноски по `@w:type`; неизвестное значение — вид по умолчанию.
fn note_kind(attrs: &[Attr<'_>], default_kind: NoteKind) -> NoteKind {
    match find(attrs, "type").map(str::trim) {
        Some("separator") => NoteKind::Separator,
        Some("continuationSeparator") => NoteKind::ContinuationSeparator,
        Some("continuationNotice") => NoteKind::ContinuationNotice,
        _ => default_kind,
    }
}

/// Свойства знака сноски: `w:rPr` внутри `w:pPr` первого абзаца тела.
///
/// Знак выноски (`w:footnoteRef`) — run первого абзаца, но начертание Word
/// пишет в свойствах знака абзаца, а не run'а. `parse_blocks` уже разобрал
/// `w:pPr/w:rPr` в `Paragraph::mark_rpr` — второй раз XML не читается.
fn mark_rpr(body: &[BlockItem]) -> RawRPr {
    body.iter()
        .find_map(|item| match item {
            BlockItem::Paragraph(paragraph) => Some((*paragraph.mark_rpr).clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Имя элемента с префиксом — для сообщений предупреждений.
fn element_name(element: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(element.name().into_inner()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Inline, Paragraph, RunContent, Toggle};

    const PART: &str = "word/footnotes.xml";

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

    /// Разобрать часть с видом сноски по умолчанию.
    fn parse_notes(xml: &[u8], part: &str, kind: NoteKind) -> (Vec<Footnote>, ParseCtx) {
        let mut ctx = ParseCtx::new();
        let notes = parse(xml, &Relationships::default(), &mut ctx, part, kind)
            .unwrap_or_else(|e| panic!("{part}: {e}"));
        (notes, ctx)
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

    /// Сноска с текстом: число, вид и тело сверяются с сайкаром.
    #[test]
    fn a_footnote_with_text_matches_its_sidecar() {
        let (xml, sidecar) = fixture("footnote_basic", PART);
        let (notes, ctx) = parse_notes(&xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].note_id, 1);
        assert_eq!(notes[0].kind, NoteKind::Footnote);
        let expected = sidecar["content"]["footnotes"]
            .as_array()
            .expect("сайкар: `content.footnotes`");
        assert_eq!(notes.len(), expected.len(), "число сносок");
        for (note, want) in notes.iter().zip(expected) {
            assert_eq!(
                i64::from(note.note_id),
                want["id"].as_i64().expect("сайкар: `id`"),
            );
            assert_eq!(
                paragraph_texts(&note.body),
                vec![want["text"].as_str().expect("сайкар: `text`").to_owned()],
            );
        }
        // `w:footnoteRef` пока не разобран и приезжает `UnknownElement` — важно
        // только, что сам разбор сносок ничего не забраковал.
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// Служебные части различаются по `@w:type`, а не по `w:id`.
    #[test]
    fn separators_from_the_fixture_get_their_kinds() {
        let (xml, sidecar) = fixture("footnote_separator", PART);
        let (notes, ctx) = parse_notes(&xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 3);
        assert_eq!(
            notes.iter().map(|note| note.note_id).collect::<Vec<_>>(),
            vec![-1, 0, 1],
        );
        assert_eq!(
            notes.iter().map(|note| note.kind).collect::<Vec<_>>(),
            vec![
                NoteKind::Separator,
                NoteKind::ContinuationSeparator,
                NoteKind::Footnote
            ],
        );
        let separators = sidecar["content"]["separators"]
            .as_array()
            .expect("сайкар: `content.separators`");
        assert_eq!(separators.len(), 2);
        for (note, want) in notes.iter().zip(separators) {
            assert_eq!(
                i64::from(note.note_id),
                want["id"].as_i64().expect("сайкар: `id`"),
            );
        }
        let footnotes = sidecar["content"]["footnotes"]
            .as_array()
            .expect("сайкар: `content.footnotes`");
        assert_eq!(
            paragraph_texts(&notes[2].body),
            vec![footnotes[0]["text"]
                .as_str()
                .expect("сайкар: `text`")
                .to_owned()],
        );
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// Концевые сноски — тот же разбор, но другой вид по умолчанию.
    #[test]
    fn endnotes_get_the_endnote_kind_by_default() {
        let xml = br#"<w:endnotes>
            <w:endnote w:id="2"><w:p><w:r><w:t>Endnote.</w:t></w:r></w:p></w:endnote>
            <w:endnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:endnote>
        </w:endnotes>"#;
        let (notes, ctx) = parse_notes(xml, "word/endnotes.xml", NoteKind::Endnote);

        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].kind, NoteKind::Endnote);
        assert_eq!(notes[0].note_id, 2);
        assert_eq!(paragraph_texts(&notes[0].body), vec!["Endnote."]);
        assert_eq!(notes[1].kind, NoteKind::Separator);
        assert_eq!(notes[1].note_id, -1);
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// Сноска из ячейки таблицы: разбор не зависит от места вызова.
    #[test]
    fn a_footnote_from_a_table_cell_fixture_parses() {
        let (xml, sidecar) = fixture("footnote_in_table", PART);
        let (notes, ctx) = parse_notes(&xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 1);
        assert_eq!(
            paragraph_texts(&notes[0].body),
            vec![sidecar["content"]["footnotes"][0]["text"]
                .as_str()
                .expect("сайкар: `text`")
                .to_owned()],
        );
        assert!(
            invalid_attribute_ids(&ctx).is_empty(),
            "{:?}",
            ctx.warnings()
        );
    }

    /// Тело сноски идёт через `parse_blocks`: несколько абзацев и таблица.
    #[test]
    fn a_body_with_several_paragraphs_and_a_table_is_parsed_as_blocks() {
        let xml = br#"<w:footnotes><w:footnote w:id="1">
            <w:p><w:pPr><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:t>First.</w:t></w:r></w:p>
            <w:p><w:r><w:t>Second.</w:t></w:r></w:p>
            <w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>
              <w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>
              <w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>
                <w:p><w:r><w:t>Cell.</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
        </w:footnote></w:footnotes>"#;
        let (notes, _) = parse_notes(xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 1);
        // Третий блок — таблица: до S7b она приезжает `Unknown`, после — `Table`;
        // тесту важен состав тела, а не вариант блока.
        assert_eq!(notes[0].body.len(), 3);
        assert_eq!(paragraph_texts(&notes[0].body), vec!["First.", "Second."]);
        // Знак сноски: начертание берётся из `w:pPr/w:rPr` первого абзаца.
        assert_eq!(notes[0].mark_rpr.b, Some(Toggle::On));
    }

    /// ID сноски выдаётся раньше ID её тела: нумерация идёт в порядке обхода XML.
    #[test]
    fn a_note_id_precedes_the_ids_of_its_body() {
        let (xml, _) = fixture("footnote_basic", PART);
        let (notes, _) = parse_notes(&xml, PART, NoteKind::Footnote);

        let BlockItem::Paragraph(paragraph) = &notes[0].body[0] else {
            panic!("первый блок сноски — абзац");
        };
        assert!(
            notes[0].id < paragraph.id,
            "{:?} < {:?}",
            notes[0].id,
            paragraph.id
        );
    }

    /// Дубликат `w:id`: предупреждение, первый побеждает.
    #[test]
    fn a_duplicate_id_is_warned_and_the_first_note_wins() {
        let xml = br#"<w:footnotes>
            <w:footnote w:id="7"><w:p><w:r><w:t>First.</w:t></w:r></w:p></w:footnote>
            <w:footnote w:id="7"><w:p><w:r><w:t>Second.</w:t></w:r></w:p></w:footnote>
        </w:footnotes>"#;
        let (notes, ctx) = parse_notes(xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 1);
        assert_eq!(paragraph_texts(&notes[0].body), vec!["First."]);
        let warnings = ctx.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::InvalidAttribute);
        assert!(
            warnings[0].message.contains("duplicate"),
            "{}",
            warnings[0].message
        );
    }

    /// Невалидный `w:id`: предупреждение, сноска пропускается.
    #[test]
    fn an_invalid_id_is_warned_and_the_note_is_skipped() {
        let xml = br#"<w:footnotes>
            <w:footnote w:id="not a number"><w:p><w:r><w:t>Lost.</w:t></w:r></w:p></w:footnote>
            <w:footnote><w:p><w:r><w:t>Also lost.</w:t></w:r></w:p></w:footnote>
            <w:footnote w:id="2"><w:p><w:r><w:t>Kept.</w:t></w:r></w:p></w:footnote>
        </w:footnotes>"#;
        let (notes, ctx) = parse_notes(xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].note_id, 2);
        assert_eq!(paragraph_texts(&notes[0].body), vec!["Kept."]);
        let ids = invalid_attribute_ids(&ctx);
        assert_eq!(ids.len(), 2, "{:?}", ctx.warnings());
        assert!(ids[0].contains("id"), "{}", ids[0]);
        assert!(ids[1].contains("w:id"), "{}", ids[1]);
    }

    /// Пустой контейнер — пустой вектор, а не ошибка.
    #[test]
    fn an_empty_container_gives_no_notes() {
        for xml in [
            &b"<w:footnotes/>"[..],
            b"<w:footnotes></w:footnotes>",
            b"<w:endnotes></w:endnotes>",
        ] {
            let mut ctx = ParseCtx::new();
            let notes = parse(
                xml,
                &Relationships::default(),
                &mut ctx,
                PART,
                NoteKind::Footnote,
            )
            .expect("пустой контейнер разбирается");
            assert!(notes.is_empty(), "{}", String::from_utf8_lossy(xml));
            assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
        }
    }

    /// Незнакомый ребёнок контейнера пропускается, а не теряет поток.
    #[test]
    fn an_unknown_child_is_skipped_without_losing_the_next_note() {
        let xml = br#"<w:footnotes>
            <w:footnotePr><w:numFmt w:val="decimal"/></w:footnotePr>
            <w:something><w:nested/></w:something>
            <w:footnote w:id="1"><w:p><w:r><w:t>After.</w:t></w:r></w:p></w:footnote>
        </w:footnotes>"#;
        let (notes, ctx) = parse_notes(xml, PART, NoteKind::Footnote);

        assert_eq!(notes.len(), 1);
        assert_eq!(paragraph_texts(&notes[0].body), vec!["After."]);
        assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    }

    /// Битый XML — фатальная ошибка, а не частичный результат.
    #[test]
    fn broken_xml_is_fatal() {
        // Поток оборвался внутри сноски.
        let err = parse(
            b"<w:footnotes><w:footnote w:id=\"1\"><w:p>",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
            NoteKind::Footnote,
        )
        .expect_err("обрыв потока не разбирается");
        assert!(matches!(err, Error::Malformed { .. }), "{err}");

        // Не закрыт сам тег — на этом спотыкается quick-xml.
        parse(
            b"<w:footnotes",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
            NoteKind::Footnote,
        )
        .expect_err("незакрытый тег не разбирается");
    }

    /// Часть без контейнера — ошибка: разбирать нечего.
    #[test]
    fn a_part_without_a_container_is_malformed() {
        let err = parse(
            b"<w:document/>",
            &Relationships::default(),
            &mut ParseCtx::new(),
            PART,
            NoteKind::Footnote,
        )
        .expect_err("без контейнера сносок разбирать нечего");
        assert!(err.to_string().contains("w:footnotes"), "{err}");
    }
}
