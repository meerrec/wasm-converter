//! Разбор классических примечаний листа (`xl/comments*.xml`).
//!
//! Примечания лежат не в части листа, а в отдельной части пакета, на которую
//! лист ссылается связью типа `…/relationships/comments`. Имя части зависит от
//! порядка добавления (`comments1.xml`, `comments2.xml`), поэтому часть
//! находится по связи, а не по имени; лист без примечаний — валидный случай.
//!
//! Разбираются только классические примечания. Современные threaded comments
//! (`xl/threadedComments/*.xml`, тип связи `…/threadedComment`) в модель не
//! входят: у них другая структура (ответы, идентификаторы людей, даты) и
//! отдельный вид аннотаций в PDF.

use doc_converter_core::xml::XmlReader;
use quick_xml::events::Event;

use crate::cellref::CellRef;
use crate::error::{Result, XlsxError};
use crate::strings::read_item_text;
use crate::xml::{attributes, find, resolve_reference, Attr};

/// Примечание, привязанное к ячейке.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Ячейка, на которой лежит примечание (`ref`).
    pub cell: CellRef,
    /// Автор по индексу `authorId`. `None` — списка авторов в части нет или
    /// индекс в него не попадает: придумывать имя в этом случае нечестно.
    pub author: Option<String>,
    /// Текст примечания: все runs склеены в одну строку, как их видит читатель.
    pub text: String,
}

/// Разобрать часть `xl/comments*.xml`.
///
/// Переносы строк внутри `<t>` значимы, а `xml:space` quick-xml не смотрит,
/// поэтому читатель — [`XmlReader::preserving`].
///
/// # Errors
///
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] — у
/// `<comment>` нет `ref` или он не разбирается, либо файл оборвался внутри
/// элемента.
pub fn parse(bytes: &[u8], part: impl Into<String>) -> Result<Vec<Comment>> {
    let part = part.into();
    let mut reader = XmlReader::preserving(bytes, part.clone());
    let mut authors: Vec<String> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();

    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(start) => match start.local_name().as_ref() {
                b"author" => authors.push(read_author(&mut reader, &part)?),
                b"comment" => {
                    let attrs = attributes(&start, &part)?;
                    pending.push(read_comment(&mut reader, &attrs, &part)?);
                }
                _ => {}
            },
            Event::Empty(empty) => match empty.local_name().as_ref() {
                // `<author/>` — автор без имени, `<comment/>` — примечание без текста.
                b"author" => authors.push(String::new()),
                b"comment" => {
                    let attrs = attributes(&empty, &part)?;
                    pending.push(Pending::new(&attrs, String::new(), &part)?);
                }
                _ => {}
            },
            _ => {}
        }
    }

    Ok(pending
        .into_iter()
        .map(|comment| comment.resolve(&authors))
        .collect())
}

/// Примечание до разрешения `authorId`.
///
/// Список авторов идёт в файле до `commentList`, но полагаться на порядок
/// значит ломаться на частях, которые его не соблюдают, поэтому автор
/// подставляется после прохода, когда известны все авторы.
struct Pending {
    cell: CellRef,
    author_id: Option<usize>,
    text: String,
}

impl Pending {
    /// Собрать примечание из атрибутов `<comment>`.
    ///
    /// # Errors
    ///
    /// [`XlsxError::Malformed`] — нет атрибута `ref` или он не разбирается.
    fn new(attrs: &[Attr<'_>], text: String, part: &str) -> Result<Self> {
        let Some(raw) = find(attrs, "ref") else {
            return Err(XlsxError::malformed(part, "<comment> without ref"));
        };
        let cell = CellRef::parse(raw.trim()).map_err(|e| {
            XlsxError::malformed(part, format!("comment ref `{raw}` is not a cell: {e}"))
        })?;
        Ok(Self {
            cell,
            author_id: find(attrs, "authorId").and_then(|raw| raw.trim().parse().ok()),
            text,
        })
    }

    fn resolve(self, authors: &[String]) -> Comment {
        Comment {
            cell: self.cell,
            author: self.author_id.and_then(|id| authors.get(id)).cloned(),
            text: self.text,
        }
    }
}

/// Дочитать `<comment>` до парного закрывающего тега.
///
/// Вызывается, когда читатель стоит сразу за открывающим тегом. Текст лежит
/// в `<text>` — тот же контейнер `CT_Rst`, что у `<si>` общей таблицы строк,
/// поэтому его читает [`read_item_text`]. Содержимое `<commentPr>` (якорь
/// выноски, форма) в модель не идёт: геометрия примечанию не нужна.
///
/// # Errors
///
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] — файл
/// оборвался внутри `<comment>`.
fn read_comment(reader: &mut XmlReader<'_>, attrs: &[Attr<'_>], part: &str) -> Result<Pending> {
    // Глубина вложенности внутри `<comment>`: на нуле закрывающий тег наш.
    let mut depth = 0usize;
    let mut text = String::new();

    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Start(start) => {
                if depth == 0 && start.local_name().as_ref() == b"text" {
                    // Дочитывает `</text>` сам и возвращает нас на уровень `<comment>`.
                    text = read_item_text(reader, part, "text")?;
                } else {
                    depth += 1;
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    return Pending::new(attrs, text, part);
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    Err(XlsxError::malformed(
        part,
        "unexpected end of file inside <comment>",
    ))
}

/// Дочитать текст `<author>` до парного закрывающего тега.
///
/// # Errors
///
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] — файл
/// оборвался внутри `<author>` или текст не декодируется.
fn read_author(reader: &mut XmlReader<'_>, part: &str) -> Result<String> {
    let mut text = String::new();
    // У `<author>` вложенных элементов не бывает, но глубина всё равно
    // считается: иначе испорченный файл увёл бы разбор в чужой тег.
    let mut depth = 0usize;

    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Text(chunk) if depth == 0 => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| XlsxError::malformed(part, format!("bad text: {e}")))?;
                text.push_str(&decoded);
            }
            // Ссылки на сущности quick-xml отдаёт отдельным событием.
            Event::GeneralRef(reference) if depth == 0 => {
                text.push_str(&resolve_reference(&reference, part)?);
            }
            Event::CData(chunk) if depth == 0 => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| XlsxError::malformed(part, format!("bad CDATA: {e}")))?;
                text.push_str(&decoded);
            }
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => return Ok(text),
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }

    Err(XlsxError::malformed(
        part,
        "unexpected end of file inside <author>",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PART: &str = "xl/comments1.xml";

    fn parse(xml: &str) -> Vec<Comment> {
        super::parse(xml.as_bytes(), PART).unwrap()
    }

    const COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <authors>
    <author>Ирек</author>
    <author>ООО &amp; Ко</author>
  </authors>
  <commentList>
    <comment ref="B2" authorId="0" shapeId="0">
      <text>
        <r><rPr><b/><sz val="9"/><color rgb="FF0000"/><rFont val="Tahoma"/></rPr><t>Первая строка</t></r>
        <r><rPr><sz val="9"/></rPr><t xml:space="preserve">
вторая строка</t></r>
      </text>
      <commentPr><shapeId>0</shapeId></commentPr>
    </comment>
    <comment ref="C3" authorId="1"><text><t>Ответ</t></text></comment>
    <comment ref="D4"><text><t>Без автора</t></text></comment>
  </commentList>
</comments>"#;

    #[test]
    fn reads_authors_cells_and_rich_text() {
        let comments = parse(COMMENTS);
        assert_eq!(comments.len(), 3);

        let first = &comments[0];
        assert_eq!(first.cell, CellRef::new(1, 1));
        assert_eq!(first.author.as_deref(), Some("Ирек"));
        // Runs склеены, перенос и хвостовой пробел сохранены, `<commentPr>`
        // в текст не попал.
        assert_eq!(first.text, "Первая строка\nвторая строка");

        // Сущность в имени автора разворачивается.
        let second = &comments[1];
        assert_eq!(second.cell, CellRef::new(2, 2));
        assert_eq!(second.author.as_deref(), Some("ООО & Ко"));
        assert_eq!(second.text, "Ответ");

        // `authorId` нет — автор не выдумывается.
        let third = &comments[2];
        assert_eq!(third.cell, CellRef::new(3, 3));
        assert_eq!(third.author, None);
        assert_eq!(third.text, "Без автора");
    }

    #[test]
    fn empty_document_has_no_comments() {
        let comments = parse(
            r#"<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                 <authors/><commentList/>
               </comments>"#,
        );
        assert!(comments.is_empty());
    }

    #[test]
    fn comment_without_text_or_unknown_author_id() {
        let comments = parse(
            r#"<comments>
                 <authors><author/></authors>
                 <commentList>
                   <comment ref="A1" authorId="0"/>
                   <comment ref="A2" authorId="7"><text><t>x</t></text></comment>
                 </commentList>
               </comments>"#,
        );

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].cell, CellRef::new(0, 0));
        assert_eq!(comments[0].author.as_deref(), Some(""));
        assert_eq!(comments[0].text, "");
        // Индекс за пределами списка авторов — автор неизвестен.
        assert_eq!(comments[1].author, None);
        assert_eq!(comments[1].text, "x");
    }

    #[test]
    fn missing_ref_is_an_error() {
        let err = super::parse(
            r#"<comments><commentList><comment authorId="0"/></commentList></comments>"#.as_bytes(),
            PART,
        )
        .unwrap_err();
        assert!(matches!(err, XlsxError::Malformed { .. }));
    }

    #[test]
    fn bad_ref_is_an_error() {
        let err = super::parse(
            r#"<comments><commentList><comment ref="B2:C3"/></commentList></comments>"#.as_bytes(),
            PART,
        )
        .unwrap_err();
        assert!(matches!(err, XlsxError::Malformed { .. }));
    }

    #[test]
    fn truncated_comment_is_an_error() {
        let err = super::parse(
            r#"<comments><commentList><comment ref="A1"><text><t>x"#.as_bytes(),
            PART,
        )
        .unwrap_err();
        assert!(matches!(err, XlsxError::Malformed { .. }));
    }
}
