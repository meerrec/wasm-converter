//! Разбор `xl/workbook.xml` — каталога листов книги.
//!
//! Содержимое листов лежит в отдельных частях, а `workbook.xml` ссылается на
//! них через relationships (`r:id`), поэтому разбору нужна карта
//! [`RelMap`] из `xl/_rels/workbook.xml.rels`.

use doc_converter_core::rels::RelMap;
use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesStart, Event};

use crate::error::{Result, XlsxError};
use crate::model::{SheetState, WorksheetMeta};
use crate::print_settings::{parse_print_titles, read_element_text, PrintTitles};
use crate::xml::{attributes, find, is_true, Attr};

/// Окончание `Type` связи, ведущей на обычный лист.
const WORKSHEET_REL: &str = "/worksheet";

/// Имя `<definedName>` с печатаемыми заголовками.
const PRINT_TITLES_NAME: &str = "_xlnm.Print_Titles";

/// То, что `xl/workbook.xml` сообщает о книге.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkbookMeta {
    /// Листы в порядке из файла.
    pub sheets: Vec<WorksheetMeta>,
    /// `<workbookPr date1904="1"/>`: даты считаются от 1904-01-01.
    pub date1904: bool,
    /// Печатаемые заголовки листов из `_xlnm.Print_Titles` в `definedNames`.
    pub print_titles: Vec<PrintTitles>,
}

impl WorkbookMeta {
    /// Разобрать `xl/workbook.xml`, разрешив `r:id` через `rels`.
    ///
    /// Листы, чья связь ведёт не на `worksheet` (chartsheet, dialogsheet,
    /// macrosheet), пропускаются: движок их не рисует, и в модели листов они
    /// не занимают место. Неизвестное значение `state` считается видимым —
    /// из-за одного испорченного атрибута книга открываться не должна.
    ///
    /// # Errors
    ///
    /// [`XlsxError::Core`] — XML не разбирается или `r:id` листа нет в карте
    /// связей; [`XlsxError::Malformed`] — у `<sheet>` нет имени, связи или
    /// цель связи внешняя.
    pub fn parse(bytes: &[u8], rels: &RelMap, part: impl Into<String>) -> Result<Self> {
        let part = part.into();
        // Пробелы значимы: значения `definedName` — это ссылки с именами
        // листов, где пробел — часть имени.
        let mut reader = XmlReader::preserving(bytes, part.clone());
        let mut meta = Self::default();

        while let Some(event) = reader.next_significant()? {
            // `<definedName>` несёт текст, поэтому читается до развилки на
            // Start/Empty: у остальных событий здесь нет работы.
            if let Event::Start(element) = &event {
                if element.local_name().as_ref() == b"definedName" {
                    read_defined_name(&mut meta, element, &mut reader, &part)?;
                    continue;
                }
            }
            let (Event::Start(element) | Event::Empty(element)) = event else {
                continue;
            };
            match element.local_name().as_ref() {
                b"workbookPr" => {
                    let attrs = attributes(&element, &part)?;
                    meta.date1904 = find(&attrs, "date1904").is_some_and(is_true);
                }
                b"sheet" => {
                    if let Some(sheet) = sheet(&attributes(&element, &part)?, rels, &part)? {
                        meta.sheets.push(sheet);
                    }
                }
                _ => {}
            }
        }

        Ok(meta)
    }
}

/// Прочитать `<definedName>`, если это печатаемые заголовки листа.
///
/// Заголовки лежат в книге, а не в части листа, потому что ссылаются на имя
/// листа. Остальные определённые имена (`_xlnm.Print_Area`, пользовательские)
/// пропускаются вместе с текстом: внешний цикл на текст не смотрит.
fn read_defined_name(
    meta: &mut WorkbookMeta,
    element: &BytesStart<'_>,
    reader: &mut XmlReader<'_>,
    part: &str,
) -> Result<()> {
    let attrs = attributes(element, part)?;
    if find(&attrs, "name") != Some(PRINT_TITLES_NAME) {
        return Ok(());
    }
    let value = read_element_text(reader, part, "definedName")?;
    meta.print_titles.extend(parse_print_titles(&value));
    Ok(())
}

/// Собрать [`WorksheetMeta`] из атрибутов `<sheet>`.
///
/// `Ok(None)` — лист пропущен: связь ведёт не на лист (например, на диаграмму).
fn sheet(attrs: &[Attr<'_>], rels: &RelMap, part: &str) -> Result<Option<WorksheetMeta>> {
    let name = find(attrs, "name")
        .ok_or_else(|| XlsxError::malformed(part, "<sheet> without a name"))?
        .to_owned();
    let rel_id = find(attrs, "id").ok_or_else(|| {
        XlsxError::malformed(part, format!("<sheet name=\"{name}\"> without r:id"))
    })?;

    let rel = rels.get(rel_id).ok_or_else(|| {
        XlsxError::from(doc_converter_core::Error::UnresolvedRel(rel_id.to_owned()))
    })?;

    if !rel.rel_type.ends_with(WORKSHEET_REL) {
        return Ok(None);
    }

    let target = rel.part(part).ok_or_else(|| {
        XlsxError::malformed(
            part,
            format!("<sheet name=\"{name}\"> points outside the package"),
        )
    })?;

    Ok(Some(WorksheetMeta {
        name,
        part: target,
        state: find(attrs, "state").map_or(SheetState::Visible, state_of),
    }))
}

/// Видимость листа; неизвестное значение трактуем как видимый лист.
fn state_of(value: &str) -> SheetState {
    match value {
        "hidden" => SheetState::Hidden,
        "veryHidden" => SheetState::VeryHidden,
        _ => SheetState::Visible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print_settings::Span;

    const PART: &str = "xl/workbook.xml";

    fn rels(xml: &str) -> RelMap {
        RelMap::parse(xml.as_bytes()).unwrap()
    }

    fn workbook(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
               <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                         xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
                 {body}
               </workbook>"#
        )
    }

    fn rels_with_sheets() -> RelMap {
        rels(
            r#"<?xml version="1.0"?>
               <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                 <Relationship Id="rId1" Type="http://x/relationships/worksheet" Target="worksheets/sheet1.xml"/>
                 <Relationship Id="rId2" Type="http://x/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/>
                 <Relationship Id="rId3" Type="http://x/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
               </Relationships>"#,
        )
    }

    fn parse(body: &str) -> Result<WorkbookMeta> {
        WorkbookMeta::parse(workbook(body).as_bytes(), &rels_with_sheets(), PART)
    }

    #[test]
    fn collects_sheets_in_file_order() {
        let meta = parse(
            r#"<sheets>
                 <sheet name="Данные" sheetId="1" r:id="rId1"/>
                 <sheet name="Скрытый" sheetId="2" state="hidden" r:id="rId2"/>
               </sheets>"#,
        )
        .unwrap();

        assert_eq!(meta.sheets.len(), 2);
        assert_eq!(meta.sheets[0].name, "Данные");
        assert_eq!(meta.sheets[0].part, "xl/worksheets/sheet1.xml");
        assert_eq!(meta.sheets[0].state, SheetState::Visible);
        assert_eq!(meta.sheets[1].name, "Скрытый");
        // Абсолютная цель разрешается от корня пакета.
        assert_eq!(meta.sheets[1].part, "xl/worksheets/sheet2.xml");
        assert_eq!(meta.sheets[1].state, SheetState::Hidden);
    }

    #[test]
    fn skips_non_worksheet_parts() {
        let meta = parse(
            r#"<sheets>
                 <sheet name="Данные" sheetId="1" r:id="rId1"/>
                 <sheet name="Диаграмма" sheetId="3" r:id="rId3"/>
               </sheets>"#,
        )
        .unwrap();

        assert_eq!(meta.sheets.len(), 1);
        assert_eq!(meta.sheets[0].name, "Данные");
    }

    #[test]
    fn reads_date_system() {
        assert!(parse(r#"<workbookPr date1904="1"/>"#).unwrap().date1904);
        assert!(parse(r#"<workbookPr date1904="true"/>"#).unwrap().date1904);
        assert!(!parse(r#"<workbookPr date1904="0"/>"#).unwrap().date1904);
        assert!(!parse(r"<workbookPr/>").unwrap().date1904);
        assert!(!parse("").unwrap().date1904);
        // Атрибут есть, но не про даты.
        assert!(!parse(r#"<workbookPr codeName="X"/>"#).unwrap().date1904);
    }

    #[test]
    fn unknown_state_is_visible() {
        let meta =
            parse(r#"<sheets><sheet name="A" sheetId="1" state="weird" r:id="rId1"/></sheets>"#)
                .unwrap();

        assert_eq!(meta.sheets[0].state, SheetState::Visible);
    }

    #[test]
    fn very_hidden_state_is_kept() {
        let meta = parse(
            r#"<sheets><sheet name="A" sheetId="1" state="veryHidden" r:id="rId1"/></sheets>"#,
        )
        .unwrap();

        assert_eq!(meta.sheets[0].state, SheetState::VeryHidden);
    }

    #[test]
    fn unescapes_entities_in_sheet_names() {
        let meta = parse(
            r#"<sheets><sheet name="Доходы &amp; расходы" sheetId="1" r:id="rId1"/></sheets>"#,
        )
        .unwrap();

        assert_eq!(meta.sheets[0].name, "Доходы & расходы");
    }

    #[test]
    fn empty_workbook_has_no_sheets() {
        let meta = parse("<sheets/>").unwrap();

        assert!(meta.sheets.is_empty());
        assert!(!meta.date1904);
    }

    #[test]
    fn sheet_without_name_is_malformed() {
        let err = parse(r#"<sheets><sheet sheetId="1" r:id="rId1"/></sheets>"#).unwrap_err();

        assert!(matches!(err, XlsxError::Malformed { .. }));
        assert!(err.to_string().contains("without a name"));
    }

    #[test]
    fn sheet_without_rel_is_malformed() {
        let err = parse(r#"<sheets><sheet name="A" sheetId="1"/></sheets>"#).unwrap_err();

        assert!(err.to_string().contains("without r:id"));
    }

    #[test]
    fn dangling_rel_id_is_unresolved() {
        let err =
            parse(r#"<sheets><sheet name="A" sheetId="1" r:id="rId99"/></sheets>"#).unwrap_err();

        assert!(matches!(err, XlsxError::Core(_)));
        assert!(err.to_string().contains("rId99"));
    }

    #[test]
    fn external_target_is_malformed() {
        let rels = rels(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                 <Relationship Id="rId1" Type="http://x/relationships/worksheet"
                               Target="https://example.com/sheet.xml" TargetMode="External"/>
               </Relationships>"#,
        );
        let err = WorkbookMeta::parse(
            workbook(r#"<sheets><sheet name="A" sheetId="1" r:id="rId1"/></sheets>"#).as_bytes(),
            &rels,
            PART,
        )
        .unwrap_err();

        assert!(err.to_string().contains("outside the package"));
    }

    #[test]
    fn reads_print_titles_from_defined_names() {
        let meta = parse(
            r#"<sheets>
                 <sheet name="Лист1" sheetId="1" r:id="rId1"/>
                 <sheet name="Лист2" sheetId="2" r:id="rId2"/>
               </sheets>
               <definedNames>
                 <definedName name="_xlnm.Print_Area" localSheetId="0">'Лист1'!$A$1:$B$2</definedName>
                 <definedName name="_xlnm.Print_Titles" localSheetId="1">'Лист2'!$A:$B,'Лист2'!$1:$2</definedName>
               </definedNames>"#,
        )
        .unwrap();

        assert_eq!(meta.print_titles.len(), 1);
        assert_eq!(meta.print_titles[0].sheet, "Лист2");
        assert_eq!(meta.print_titles[0].rows, Some(Span { first: 0, last: 1 }));
        assert_eq!(meta.print_titles[0].cols, Some(Span { first: 0, last: 1 }));
    }

    #[test]
    fn print_titles_unescape_entities_in_sheet_names() {
        let meta = parse(
            r#"<definedNames>
                 <definedName name="_xlnm.Print_Titles">'Доходы &amp; расходы'!$1:$1</definedName>
               </definedNames>"#,
        )
        .unwrap();

        assert_eq!(meta.print_titles[0].sheet, "Доходы & расходы");
    }

    #[test]
    fn defined_names_without_print_titles_are_ignored() {
        let meta = parse(
            r#"<definedNames>
                 <definedName name="Total">Лист1!$D$10</definedName>
                 <definedName name="_xlnm.Print_Area">'Лист1'!$A$1:$B$2</definedName>
               </definedNames>"#,
        )
        .unwrap();

        assert!(meta.print_titles.is_empty());
    }

    #[test]
    fn workbook_without_defined_names_has_no_print_titles() {
        let meta =
            parse(r#"<sheets><sheet name="Лист1" sheetId="1" r:id="rId1"/></sheets>"#).unwrap();

        assert!(meta.print_titles.is_empty());
    }
}
