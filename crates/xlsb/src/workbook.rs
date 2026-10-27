//! Разбор `xl/workbook.bin`: каталог листов и свойства книги.
//!
//! Каталог — это записи `BrtBundleSh` между `BrtBeginBundleShs` и
//! `BrtEndBundleShs`. Каждая несёт имя листа, его видимость, `iTabID` и
//! `strRelID` — ссылку на связь книги, по которой находится часть листа.
//! Связь разрешает [`crate::open`]: здесь остаётся только каталог.
//!
//! Неизвестные записи пропускаются: часть книги содержит ещё десятки записей
//! (окна, вычисления, защиты), и первая из них не должна валить открытие.

use doc_converter_xlsx::SheetState;

use crate::error::{Result, XlsbError};
use crate::reader::{ids, DataCursor, NullableWideString, RecordReader};

/// Лист в каталоге книги.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundleSheet {
    /// Имя листа, как его показывает Excel.
    pub(crate) name: String,
    /// `strRelID`: ключ в `xl/_rels/workbook.bin.rels`; `None` — лист-модуль.
    pub(crate) rel_id: Option<String>,
    /// `iTabID`: по нему первый срез ищет часть, если связей нет.
    pub(crate) tab_id: u32,
    /// Видимость листа.
    pub(crate) state: SheetState,
    /// Смещение записи `BrtBundleSh` — для ошибок разрешения части.
    pub(crate) offset: usize,
}

/// Каталог книги.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Catalog {
    /// Листы в порядке `BrtBundleSh`.
    pub(crate) sheets: Vec<BundleSheet>,
    /// Даты отсчитываются от 1904-01-01 (`f1904`).
    pub(crate) date1904: bool,
}

/// Разобрать часть книги `part` из её байтов.
///
/// # Errors
/// [`XlsbError::Malformed`] — запись оборвана, `BrtBeginBook` отсутствует или
/// `BrtBundleSh` короче своих полей.
pub(crate) fn parse(bytes: &[u8], part: &str) -> Result<Catalog> {
    let mut reader = RecordReader::new(bytes, part);
    let mut catalog = Catalog::default();
    let mut seen_book = false;

    while let Some(record) = reader.next()? {
        let offset = reader.last_record_offset();
        match record.id {
            ids::BRT_BEGIN_BOOK => seen_book = true,
            ids::BRT_WB_PROP => {
                let mut cursor = DataCursor::new(record.data);
                let flags = cursor
                    .u32()
                    .ok_or_else(|| XlsbError::malformed(part, offset, "BrtWbProp is truncated"))?;
                // Бит A (f1904) — младший бит поля флагов; остальные биты
                // первого среза не касаются.
                catalog.date1904 = flags & 0x01 != 0;
            }
            ids::BRT_BUNDLE_SH => {
                catalog
                    .sheets
                    .push(read_bundle_sheet(record.data, part, offset)?);
            }
            _ => {}
        }
    }

    // Без BrtBeginBook это не часть книги, а что-то другое: пустая книга
    // из случайного файла выглядела бы как успешное открытие.
    if !seen_book {
        return Err(XlsbError::malformed(
            part,
            0,
            "workbook part has no BrtBeginBook record",
        ));
    }
    Ok(catalog)
}

/// Прочитать `BrtBundleSh`: состояние, `iTabID`, `strRelID`, имя.
fn read_bundle_sheet(data: &[u8], part: &str, offset: usize) -> Result<BundleSheet> {
    let mut cursor = DataCursor::new(data);
    let truncated = || XlsbError::malformed(part, offset, "BrtBundleSh is truncated");

    let state_raw = cursor.u32().ok_or_else(truncated)?;
    let tab_id = cursor.u32().ok_or_else(truncated)?;
    let rel_id = match cursor.nullable_wide_string().ok_or_else(truncated)? {
        NullableWideString::Null => None,
        NullableWideString::Text(text) => Some(text),
    };
    let name = cursor.wide_string().ok_or_else(truncated)?;

    Ok(BundleSheet {
        name,
        rel_id,
        tab_id,
        state: sheet_state(state_raw),
        offset,
    })
}

/// `ST_SheetState`: 0 — видимый, 1 — скрытый, 2 — очень скрытый.
///
/// Неизвестное значение трактуется как видимое: прятать лист из-за одного
/// незнакомого числа — хуже, чем показать его.
const fn sheet_state(raw: u32) -> SheetState {
    match raw {
        1 => SheetState::Hidden,
        2 => SheetState::VeryHidden,
        _ => SheetState::Visible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{nullable_wide_string, record, wide_string, workbook_bin, SheetSpec};

    fn parse_ok(bytes: &[u8]) -> Catalog {
        parse(bytes, "xl/workbook.bin").expect("catalog")
    }

    /// Каталог несёт имя, `relId`, `iTabID` и видимость, а `BrtWbProp` —
    /// систему дат.
    #[test]
    fn reads_sheet_catalog_and_date_system() {
        let bytes = workbook_bin(
            &[
                SheetSpec {
                    name: "Data",
                    rel_id: Some("rId1"),
                    tab_id: 1,
                    state: 0,
                },
                SheetSpec {
                    name: "Hidden",
                    rel_id: Some("rId2"),
                    tab_id: 2,
                    state: 1,
                },
            ],
            true,
        );

        let catalog = parse_ok(&bytes);

        assert!(catalog.date1904);
        assert_eq!(catalog.sheets.len(), 2);
        assert_eq!(catalog.sheets[0].name, "Data");
        assert_eq!(catalog.sheets[0].rel_id.as_deref(), Some("rId1"));
        assert_eq!(catalog.sheets[0].tab_id, 1);
        assert_eq!(catalog.sheets[0].state, SheetState::Visible);
        assert_eq!(catalog.sheets[1].state, SheetState::Hidden);
    }

    #[test]
    fn unknown_state_is_visible() {
        assert_eq!(sheet_state(0), SheetState::Visible);
        assert_eq!(sheet_state(7), SheetState::Visible);
        assert_eq!(sheet_state(2), SheetState::VeryHidden);
    }

    /// `strRelID` может быть NULL: так записан лист-модуль. Каталог это
    /// сохраняет, а не выдумывает связь.
    #[test]
    fn null_rel_id_is_kept_as_none() {
        let mut data = Vec::new();
        data.extend_from_slice(&2_u32.to_le_bytes()); // VERYHIDDEN
        data.extend_from_slice(&9_u32.to_le_bytes());
        data.extend(nullable_wide_string(None));
        data.extend(wide_string("Module1"));
        let mut bytes = record(ids::BRT_BEGIN_BOOK, &[]);
        bytes.extend(record(ids::BRT_BUNDLE_SH, &data));

        let catalog = parse_ok(&bytes);

        assert_eq!(catalog.sheets[0].rel_id, None);
        assert_eq!(catalog.sheets[0].state, SheetState::VeryHidden);
    }

    #[test]
    fn missing_begin_book_is_malformed() {
        let bytes = record(ids::BRT_END_BOOK, &[]);
        let err = parse(&bytes, "xl/workbook.bin").expect_err("no BrtBeginBook");

        assert!(matches!(
            err,
            XlsbError::Malformed { offset: 0, ref reason, .. }
                if reason.contains("BrtBeginBook")
        ));
    }

    #[test]
    fn truncated_bundle_sheet_reports_its_offset() {
        let begin = record(ids::BRT_BEGIN_BOOK, &[]);
        let broken = record(ids::BRT_BUNDLE_SH, &[0, 0, 0, 0, 1, 0]);
        let mut bytes = begin;
        let offset = bytes.len();
        bytes.extend(broken);

        let err = parse(&bytes, "xl/workbook.bin").expect_err("truncated sheet");

        assert!(matches!(
            err,
            XlsbError::Malformed { offset: at, .. } if at == offset
        ));
    }

    #[test]
    fn unknown_records_are_ignored() {
        // BrtCalcProp (157) и BrtBookView (158): в первом срезе не разбираются.
        const BRT_CALC_PROP: u16 = 157;
        const BRT_BOOK_VIEW: u16 = 158;

        let mut bytes = record(ids::BRT_BEGIN_BOOK, &[]);
        bytes.extend(record(BRT_CALC_PROP, &[0xAB; 12]));
        bytes.extend(record(BRT_BOOK_VIEW, &[0xCD; 7]));

        let catalog = parse_ok(&bytes);
        assert!(catalog.sheets.is_empty());
    }
}
