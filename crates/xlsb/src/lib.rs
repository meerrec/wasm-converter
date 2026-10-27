//! XLSB (BIFF12): чтение бинарных частей OOXML-пакета в модель [`Workbook`].
//!
//! XLSB — это OOXML-пакет (ZIP), у которого части `xl/*.bin` записаны
//! записями BIFF12 вместо XML. Крейт доводит книгу до модели
//! `doc-converter-xlsx`, чтобы просмотр и экспорт работали без отдельной ветки
//! на формат.
//!
//! Первый срез читает каталог листов (`BrtBundleSh`), систему дат
//! (`BrtWbProp`), общую таблицу строк (`BrtSSTItem`) и значения ячеек.
//! Стили, тема, формулы, объединения, гиперссылки и настройки печати пока не
//! разбираются: см. `styles` и TODO в нём.
//!
//! Лист связывается с частью по `strRelID` из
//! `xl/_rels/workbook.bin.rels`. Если части связей в пакете нет (так бывает
//! в синтетических и очень простых пакетах), первый срез пробует соглашение
//! `xl/worksheets/sheet{iTabID}.bin`; при наличии связей `Target` важнее
//! соглашения.
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod error;
pub mod plugin;
pub mod reader;

mod sheet;
mod strings;
mod styles;
#[cfg(test)]
mod test_util;
mod workbook;

use doc_converter_core::rels::{rels_part, RelMap};
use doc_converter_core::{Archive, ZipLimits};
use doc_converter_xlsx::{SharedStrings, Sheet, StyleTable, Theme, Workbook, WorksheetMeta};

use crate::error::{Result, XlsbError};
use crate::workbook::BundleSheet;

/// Часть с каталогом листов.
pub const WORKBOOK_PART: &str = "xl/workbook.bin";

/// Общая таблица строк. В книге без строк этой части нет вовсе, и это норма.
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.bin";

/// Таблица стилей.
pub const STYLES_PART: &str = "xl/styles.bin";

/// Тема книги. Первый срез её не читает и отдаёт `Theme::default()`.
pub const THEME_PART: &str = "xl/theme/theme1.bin";

/// Отношение листа: `…/relationships/worksheet`.
///
/// Листы остальных типов (chart sheet, dialog sheet, macro sheet, лист-модуль)
/// в модель `xlsx` не выражаются и дают [`XlsbError::Unsupported`].
const WORKSHEET_REL: &str = "/worksheet";

/// Открыть XLSB из сырых байт.
///
/// Читает каталог листов, общую таблицу строк, стили и содержимое каждого
/// листа. Строки общей таблицы встраиваются в ячейки (`CellValue::InlineString`),
/// поэтому [`Workbook::shared_strings`] остаётся пустой: конструктора таблицы
/// из готовых строк в `xlsx` нет.
///
/// # Errors
/// [`XlsbError::Core`] — байты не ZIP-архив, в пакете нет `xl/workbook.bin`
/// или нарушены лимиты архива;
/// [`XlsbError::Malformed`] — запись BIFF12 оборвана, часть листа не найдена
/// или данные листа противоречат модели `xlsx`;
/// [`XlsbError::Unsupported`] — лист-модуль или лист не-рабочего типа;
/// [`XlsbError::Limit`] — запись превысила [`reader::MAX_RECORD_BYTES`].
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes, ZipLimits::default())?;

    let workbook::Catalog {
        sheets: entries,
        date1904,
    } = workbook::parse(&archive.part(WORKBOOK_PART)?, WORKBOOK_PART)?;

    let shared_strings = if archive.contains(SHARED_STRINGS_PART) {
        strings::parse(&archive.part(SHARED_STRINGS_PART)?, SHARED_STRINGS_PART)?
    } else {
        Vec::new()
    };

    let styles = if archive.contains(STYLES_PART) {
        styles::parse(&archive.part(STYLES_PART)?, STYLES_PART)?
    } else {
        StyleTable::default()
    };

    let rels = read_rels(&mut archive)?;
    let mut sheets = Vec::with_capacity(entries.len());
    for entry in entries {
        let part = sheet_part(&archive, &entry, rels.as_ref())?;
        let content = sheet::parse(&archive.part(&part)?, &part, &shared_strings)?;
        sheets.push(Sheet::new(
            WorksheetMeta {
                name: entry.name,
                part,
                state: entry.state,
            },
            content,
        ));
    }

    Ok(Workbook::new(
        sheets,
        SharedStrings::default(),
        styles,
        Theme::default(),
        date1904,
    ))
}

/// Связи книги, если часть связей есть.
fn read_rels(archive: &mut Archive) -> Result<Option<RelMap>> {
    let part = rels_part(WORKBOOK_PART);
    if !archive.contains(&part) {
        return Ok(None);
    }
    Ok(Some(RelMap::parse(&archive.part(&part)?)?))
}

/// Часть листа: по связи `strRelID`, иначе по соглашению `sheet{iTabID}.bin`.
fn sheet_part(archive: &Archive, entry: &BundleSheet, rels: Option<&RelMap>) -> Result<String> {
    let Some(rel_id) = &entry.rel_id else {
        // NULL `strRelID` у очень скрытого листа — признак листа-модуля:
        // части листа у него нет вовсе.
        return Err(XlsbError::Unsupported(format!(
            "sheet `{}` has no strRelID: module sheets are not supported",
            entry.name
        )));
    };

    if let Some(rel) = rels.and_then(|rels| rels.get(rel_id)) {
        if !rel.rel_type.ends_with(WORKSHEET_REL) {
            return Err(XlsbError::Unsupported(format!(
                "sheet `{}` has relationship type `{}`, only worksheets are supported",
                entry.name, rel.rel_type
            )));
        }
        let Some(part) = rel.part(WORKBOOK_PART) else {
            return Err(XlsbError::Unsupported(format!(
                "sheet `{}` has an external relationship `{rel_id}`",
                entry.name
            )));
        };
        if archive.contains(&part) {
            return Ok(part);
        }
        return Err(XlsbError::malformed(
            WORKBOOK_PART,
            entry.offset,
            format!("sheet `{}` points to the missing part `{part}`", entry.name),
        ));
    }

    // Связей нет или в них нет такого `relId`: первый срез пробует
    // соглашение имени по `iTabID`.
    let fallback = format!("xl/worksheets/sheet{}.bin", entry.tab_id);
    if archive.contains(&fallback) {
        return Ok(fallback);
    }
    Err(XlsbError::malformed(
        WORKBOOK_PART,
        entry.offset,
        format!(
            "sheet `{}` has no part: relationship `{rel_id}` is missing and `{fallback}` is absent",
            entry.name
        ),
    ))
}

#[cfg(test)]
mod tests {
    use doc_converter_xlsx::{CellRef, CellValue, SheetState};

    use super::*;
    use crate::reader::ids;
    use crate::test_util::{
        cell_bool, cell_isst, cell_real, cell_st, record, sheet_bin, sst_bin, workbook_bin, zip,
        SheetSpec,
    };

    /// Книга без связей: лист ищется по `iTabID`.
    fn minimal_book(date1904: bool) -> Vec<u8> {
        let workbook = workbook_bin(
            &[SheetSpec {
                name: "Sheet1",
                rel_id: Some("rId1"),
                tab_id: 1,
                state: 0,
            }],
            date1904,
        );
        let sheet = sheet_bin(
            0,
            &[cell_real(0, 1.5), cell_st(1, "text"), cell_bool(2, true)],
        );
        zip(&[
            (WORKBOOK_PART, &workbook),
            ("xl/worksheets/sheet1.bin", &sheet),
        ])
    }

    fn value(book: &Workbook, col: u32) -> &CellValue {
        &book.sheets()[0]
            .cells
            .cell(CellRef::new(0, col))
            .expect("cell")
            .value
    }

    #[test]
    fn opens_a_minimal_book() {
        let book = open(minimal_book(false)).expect("book");

        assert_eq!(book.sheet_count(), 1);
        assert!(!book.date1904());
        let sheet = &book.sheets()[0];
        assert_eq!(sheet.meta.name, "Sheet1");
        assert_eq!(sheet.meta.part, "xl/worksheets/sheet1.bin");
        assert_eq!(sheet.meta.state, SheetState::Visible);
        assert_eq!(value(&book, 0), &CellValue::Number(1.5));
        assert_eq!(value(&book, 1), &CellValue::InlineString("text".into()));
        assert_eq!(value(&book, 2), &CellValue::Bool(true));
    }

    #[test]
    fn reads_the_date_system() {
        let book = open(minimal_book(true)).expect("book");

        assert!(book.date1904());
    }

    /// Строки общей таблицы встраиваются в ячейки: модель `xlsx` своей таблицы
    /// не получает, но текст ячейки не теряется.
    #[test]
    fn inlines_shared_strings() {
        let workbook = workbook_bin(
            &[SheetSpec {
                name: "Sheet1",
                rel_id: Some("rId1"),
                tab_id: 1,
                state: 0,
            }],
            false,
        );
        let sheet = sheet_bin(0, &[cell_isst(0, 1)]);
        let strings = sst_bin(&["first", "second"]);
        let bytes = zip(&[
            (WORKBOOK_PART, &workbook),
            ("xl/worksheets/sheet1.bin", &sheet),
            (SHARED_STRINGS_PART, &strings),
        ]);

        let book = open(bytes).expect("book");

        assert!(book.shared_strings().is_empty());
        assert_eq!(value(&book, 0), &CellValue::InlineString("second".into()));
    }

    /// Связь книги важнее соглашения имени: лист берётся из `Target`, а
    /// одноимённый `sheet1.bin` игнорируется.
    #[test]
    fn follows_workbook_relationships() {
        let workbook = workbook_bin(
            &[SheetSpec {
                name: "Data",
                rel_id: Some("rId1"),
                tab_id: 1,
                state: 0,
            }],
            false,
        );
        let rels = br#"<?xml version="1.0" encoding="UTF-8"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1"
                              Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                              Target="worksheets/data.bin"/>
            </Relationships>"#
            .to_vec();
        let data = sheet_bin(0, &[cell_real(0, 7.0)]);
        let decoy = sheet_bin(0, &[cell_real(0, 1.0)]);
        let bytes = zip(&[
            (WORKBOOK_PART, &workbook),
            ("xl/_rels/workbook.bin.rels", &rels),
            ("xl/worksheets/data.bin", &data),
            ("xl/worksheets/sheet1.bin", &decoy),
        ]);

        let book = open(bytes).expect("book");

        assert_eq!(book.sheets()[0].meta.part, "xl/worksheets/data.bin");
        assert_eq!(value(&book, 0), &CellValue::Number(7.0));
    }

    #[test]
    fn rejects_a_non_zip_payload() {
        let err = open(vec![0x50, 0x4B, 0x00]).expect_err("not a zip");

        assert!(matches!(err, XlsbError::Core(_)));
    }

    #[test]
    fn missing_workbook_part_is_a_core_error() {
        let bytes = zip(&[("xl/worksheets/sheet1.bin", b"")]);

        let err = open(bytes).expect_err("no workbook part");

        match err {
            XlsbError::Core(doc_converter_core::Error::MissingPart(part)) => {
                assert_eq!(part, WORKBOOK_PART);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn truncated_workbook_record_reports_part_and_offset() {
        let mut workbook = record(ids::BRT_BEGIN_BOOK, &[]);
        let offset = workbook.len();
        // BrtBundleSh (156), объявлено 64 байта, данных нет.
        workbook.extend([0x9C, 0x01, 0x40]);
        let bytes = zip(&[(WORKBOOK_PART, &workbook)]);

        let err = open(bytes).expect_err("truncated record");

        match err {
            XlsbError::Malformed {
                part,
                offset: at,
                reason,
            } => {
                assert_eq!(part, WORKBOOK_PART);
                assert_eq!(at, offset);
                assert!(reason.contains("64 bytes"), "{reason}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn module_sheet_is_unsupported() {
        let workbook = workbook_bin(
            &[SheetSpec {
                name: "Module1",
                rel_id: None,
                tab_id: 1,
                state: 2,
            }],
            false,
        );
        let bytes = zip(&[(WORKBOOK_PART, &workbook)]);

        let err = open(bytes).expect_err("module sheet");

        assert!(matches!(err, XlsbError::Unsupported(ref reason) if reason.contains("strRelID")));
    }

    #[test]
    fn missing_sheet_part_is_malformed() {
        let workbook = workbook_bin(
            &[SheetSpec {
                name: "Sheet1",
                rel_id: Some("rId1"),
                tab_id: 1,
                state: 0,
            }],
            false,
        );
        let bytes = zip(&[(WORKBOOK_PART, &workbook)]);

        let err = open(bytes).expect_err("no sheet part");

        assert!(matches!(
            err,
            XlsbError::Malformed { ref part, ref reason, .. }
                if part == WORKBOOK_PART && reason.contains("sheet1.bin")
        ));
    }

    #[test]
    fn chart_sheet_relationship_is_unsupported() {
        let workbook = workbook_bin(
            &[SheetSpec {
                name: "Chart1",
                rel_id: Some("rId1"),
                tab_id: 1,
                state: 0,
            }],
            false,
        );
        let rels = br#"<?xml version="1.0" encoding="UTF-8"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1"
                              Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet"
                              Target="chartsheets/sheet1.bin"/>
            </Relationships>"#
            .to_vec();
        let bytes = zip(&[
            (WORKBOOK_PART, &workbook),
            ("xl/_rels/workbook.bin.rels", &rels),
        ]);

        let err = open(bytes).expect_err("chart sheet");

        assert!(matches!(err, XlsbError::Unsupported(ref reason) if reason.contains("chartsheet")));
    }
}
