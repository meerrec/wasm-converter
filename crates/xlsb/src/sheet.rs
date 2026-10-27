//! Разбор `xl/worksheets/sheetN.bin`.
//!
//! Часть листа — поток записей: `BrtBeginSheet`, прочие свойства, затем
//! `BrtBeginSheetData` и пары «`BrtRowHdr` — записи ячеек», после них
//! `BrtEndSheetData` и `BrtEndSheet`. Читатель ведёт текущую строку и отдаёт
//! ячейки сборщику `xlsx`, который сам проверяет возрастание строк и столбцов.
//!
//! Границы (`BrtBeginSheet`, `BrtEndSheet`, `BrtBeginSheetData`,
//! `BrtEndSheetData`) не проверяются: данных для модели они не несут, а их
//! пропуск не мешает разобрать корректную часть. Всё, что срез не знает,
//! пропускается так же.
//!
//! Формулы первого среза не разбираются: у записей `BrtFmlaNum` и родственных
//! значение лежит перед формулой, поэтому значение сохраняется, а
//! `Cell::formula` остаётся пустым.

use doc_converter_xlsx::{Cell, CellError, CellValue, SheetContent, WorksheetBuilder, XlsxError};

use crate::error::{Result, XlsbError};
use crate::reader::{ids, DataCursor, Record, RecordReader};

/// Записи, которые несут заголовок `Cell` и значение ячейки.
const CELL_RECORDS: [u16; 12] = [
    ids::BRT_CELL_BLANK,
    ids::BRT_CELL_RK,
    ids::BRT_CELL_ERROR,
    ids::BRT_CELL_BOOL,
    ids::BRT_CELL_REAL,
    ids::BRT_CELL_ST,
    ids::BRT_CELL_ISST,
    ids::BRT_FMLA_STRING,
    ids::BRT_FMLA_NUM,
    ids::BRT_FMLA_BOOL,
    ids::BRT_FMLA_ERROR,
    ids::BRT_CELL_R_STRING,
];

/// Разобрать часть листа `part` из её байтов.
///
/// `shared_strings` — уже прочитанная общая таблица строк: индексы
/// `BrtCellIsst` разрешаются в текст сразу, чтобы модель книги не зависела от
/// таблицы, которую `xlsx` в первом срезе не заполняет.
///
/// # Errors
/// [`XlsbError::Malformed`] — запись оборвана, ячейка встретилась до
/// `BrtRowHdr`, индекс общей строки вне таблицы или порядок ячеек нарушен.
pub(crate) fn parse(bytes: &[u8], part: &str, shared_strings: &[String]) -> Result<SheetContent> {
    let mut reader = RecordReader::new(bytes, part);
    let mut builder = WorksheetBuilder::new(part);
    let mut row: Option<u32> = None;

    while let Some(record) = reader.next()? {
        let offset = reader.last_record_offset();

        if record.id == ids::BRT_ROW_HDR {
            let mut cursor = DataCursor::new(record.data);
            row = Some(
                cursor
                    .u32()
                    .ok_or_else(|| truncated(part, offset, "BrtRowHdr"))?,
            );
            continue;
        }

        let Some(cell) = decode_cell(record, part, offset, shared_strings)? else {
            continue;
        };
        let Some(row) = row else {
            return Err(XlsbError::malformed(
                part,
                offset,
                "cell record appears before BrtRowHdr",
            ));
        };
        builder
            .push(row, cell)
            .map_err(|error| XlsbError::malformed(part, offset, reason_of(error)))?;
    }

    Ok(SheetContent {
        cells: builder.finish(),
        ..SheetContent::default()
    })
}

/// Разобрать запись ячейки; `None` — это не запись ячейки.
fn decode_cell(
    record: Record<'_>,
    part: &str,
    offset: usize,
    shared_strings: &[String],
) -> Result<Option<Cell>> {
    if !CELL_RECORDS.contains(&record.id) {
        return Ok(None);
    }

    let mut cursor = DataCursor::new(record.data);
    let col = cursor
        .u32()
        .ok_or_else(|| truncated(part, offset, "cell header"))?;
    let style_low = cursor
        .u8()
        .ok_or_else(|| truncated(part, offset, "cell header"))?;
    let style_mid = cursor
        .u8()
        .ok_or_else(|| truncated(part, offset, "cell header"))?;
    let style_high = cursor
        .u8()
        .ok_or_else(|| truncated(part, offset, "cell header"))?;
    // Четвёртый байт заголовка — fPhShow и резерв; модели он не нужен.
    cursor
        .u8()
        .ok_or_else(|| truncated(part, offset, "cell header"))?;
    let style = u32::from(style_low) | (u32::from(style_mid) << 8) | (u32::from(style_high) << 16);

    let value = match record.id {
        ids::BRT_CELL_BLANK => CellValue::Empty,

        // fInt (бит 1) — знаковое целое, fx100 (бит 0) — число умножено на 100.
        ids::BRT_CELL_RK => CellValue::Number(decode_rk(
            cursor
                .u32()
                .ok_or_else(|| truncated(part, offset, "BrtCellRk"))?,
        )),

        ids::BRT_CELL_REAL | ids::BRT_FMLA_NUM => CellValue::Number(
            cursor
                .f64()
                .ok_or_else(|| truncated(part, offset, "BrtCellReal"))?,
        ),

        ids::BRT_CELL_BOOL | ids::BRT_FMLA_BOOL => CellValue::Bool(
            cursor
                .u8()
                .ok_or_else(|| truncated(part, offset, "BrtCellBool"))?
                != 0,
        ),

        ids::BRT_CELL_ERROR | ids::BRT_FMLA_ERROR => {
            let code = cursor
                .u8()
                .ok_or_else(|| truncated(part, offset, "BrtCellError"))?;
            let error = decode_error(code).ok_or_else(|| {
                XlsbError::malformed(part, offset, format!("unknown cell error code {code:#04X}"))
            })?;
            CellValue::Error(error)
        }

        ids::BRT_CELL_ST | ids::BRT_FMLA_STRING => CellValue::InlineString(
            cursor
                .wide_string()
                .ok_or_else(|| truncated(part, offset, "BrtCellSt"))?
                .into(),
        ),

        // RichStr: байт флагов fRichStr/fExtStr, затем строка. Прогоны
        // форматирования и фонетика в модель не попадают.
        ids::BRT_CELL_R_STRING => {
            cursor
                .u8()
                .ok_or_else(|| truncated(part, offset, "BrtCellRString"))?;
            CellValue::InlineString(
                cursor
                    .wide_string()
                    .ok_or_else(|| truncated(part, offset, "BrtCellRString"))?
                    .into(),
            )
        }

        ids::BRT_CELL_ISST => {
            let index = cursor
                .u32()
                .ok_or_else(|| truncated(part, offset, "BrtCellIsst"))?;
            let text = usize::try_from(index)
                .ok()
                .and_then(|index| shared_strings.get(index))
                .ok_or_else(|| {
                    XlsbError::malformed(
                        part,
                        offset,
                        format!(
                            "shared string index {index} is out of range ({} items)",
                            shared_strings.len()
                        ),
                    )
                })?;
            // `xlsx::SharedStrings` в первом срезе остаётся пустой: строка
            // встраивается в ячейку, и текст не теряется.
            CellValue::InlineString(text.as_str().into())
        }

        // CELL_RECORDS и этот match держатся вместе; сюда попадает только их
        // рассинхронизация.
        _ => return Ok(None),
    };

    Ok(Some(Cell::new(col, style, value)))
}

/// Декодировать `RkNumber`.
///
/// `fx100` (бит 0) означает, что сохранённое значение умножено на 100;
/// `fInt` (бит 1) — что 30 старших бит поля — знаковое целое, иначе это
/// старшие 30 бит `f64`, а младшие 34 бита нулевые.
fn decode_rk(raw: u32) -> f64 {
    let value = if raw & 0x02 != 0 {
        // `i32` из байтов — reinterpret тех же битов, без потери знака.
        f64::from(i32::from_le_bytes(raw.to_le_bytes()) >> 2)
    } else {
        f64::from_bits(u64::from(raw & 0xFFFF_FFFC) << 32)
    };
    if raw & 0x01 != 0 {
        value / 100.0
    } else {
        value
    }
}

/// Код ошибки ячейки (`BErr`): `0x00` — `#NULL!`, `0x07` — `#DIV/0!`,
/// `0x0F` — `#VALUE!`, `0x17` — `#REF!`, `0x1D` — `#NAME?`, `0x24` — `#NUM!`,
/// `0x2A` — `#N/A`. Неизвестный код — не ошибка формы записи, а значение,
/// которого нет в модели `xlsx`, поэтому вызывающий делает из него
/// [`XlsbError::Malformed`].
const fn decode_error(code: u8) -> Option<CellError> {
    match code {
        0x00 => Some(CellError::Null),
        0x07 => Some(CellError::Div0),
        0x0F => Some(CellError::Value),
        0x17 => Some(CellError::Ref),
        0x1D => Some(CellError::Name),
        0x24 => Some(CellError::Num),
        0x2A => Some(CellError::Na),
        _ => None,
    }
}

/// Ошибка «запись короче своих полей».
fn truncated(part: &str, offset: usize, record: &str) -> XlsbError {
    XlsbError::malformed(part, offset, format!("{record} is truncated"))
}

/// Причина из ошибки сборщика: часть и смещение уже известны.
fn reason_of(error: XlsxError) -> String {
    match error {
        XlsxError::Malformed { reason, .. } => reason,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use doc_converter_xlsx::CellRef;

    use super::*;
    use crate::test_util::{
        cell_blank, cell_bool, cell_error, cell_isst, cell_r_string, cell_real, cell_rk, cell_st,
        record, sheet_bin,
    };

    const PART: &str = "xl/worksheets/sheet1.bin";

    fn parse_ok(bytes: &[u8], strings: &[String]) -> SheetContent {
        parse(bytes, PART, strings).expect("sheet content")
    }

    fn value(content: &SheetContent, col: u32) -> &CellValue {
        &content
            .cells
            .cell(CellRef::new(0, col))
            .expect("cell")
            .value
    }

    #[test]
    fn reads_number_string_and_bool_cells() {
        let bytes = sheet_bin(
            0,
            &[cell_real(0, 42.5), cell_st(1, "hello"), cell_bool(2, true)],
        );

        let content = parse_ok(&bytes, &[]);

        assert_eq!(content.cells.cell_count(), 3);
        assert_eq!(value(&content, 0), &CellValue::Number(42.5));
        assert_eq!(value(&content, 1), &CellValue::InlineString("hello".into()));
        assert_eq!(value(&content, 2), &CellValue::Bool(true));
    }

    #[test]
    fn reads_blank_cell() {
        let content = parse_ok(&sheet_bin(0, &[cell_blank(3)]), &[]);

        assert_eq!(value(&content, 3), &CellValue::Empty);
    }

    /// Три варианта `RkNumber`: целое, целое со сдвигом на 100 и число
    /// с плавающей точкой в старших 30 битах.
    #[test]
    fn decodes_rk_variants() {
        let integer = (42_u32 << 2) | 0x02;
        let scaled = integer | 0x01;
        // Старшие 32 бита f64 для 1.5 = 0x3FF8_0000, младшие два бита — флаги.
        let float = 0x3FF8_0000_u32;
        let bytes = sheet_bin(
            0,
            &[cell_rk(0, integer), cell_rk(1, scaled), cell_rk(2, float)],
        );

        let content = parse_ok(&bytes, &[]);

        assert_eq!(value(&content, 0), &CellValue::Number(42.0));
        assert_eq!(value(&content, 1), &CellValue::Number(0.42));
        assert_eq!(value(&content, 2), &CellValue::Number(1.5));
    }

    #[test]
    fn decodes_error_cells() {
        let bytes = sheet_bin(0, &[cell_error(0, 0x07), cell_error(1, 0x2A)]);

        let content = parse_ok(&bytes, &[]);

        assert_eq!(value(&content, 0), &CellValue::Error(CellError::Div0));
        assert_eq!(value(&content, 1), &CellValue::Error(CellError::Na));
    }

    #[test]
    fn unknown_error_code_is_malformed() {
        let err = parse(&sheet_bin(0, &[cell_error(0, 0x55)]), PART, &[])
            .expect_err("unknown error code");

        assert!(matches!(
            err,
            XlsbError::Malformed { ref reason, .. } if reason.contains("0x55")
        ));
    }

    /// `BrtCellIsst` разрешается в текст сразу: таблица строк в модель не
    /// попадает.
    #[test]
    fn resolves_shared_string_index() {
        let bytes = sheet_bin(0, &[cell_isst(0, 1)]);
        let strings = vec!["first".to_owned(), "second".to_owned()];

        let content = parse_ok(&bytes, &strings);

        assert_eq!(
            value(&content, 0),
            &CellValue::InlineString("second".into())
        );
    }

    #[test]
    fn shared_string_index_out_of_range_is_malformed() {
        let err = parse(
            &sheet_bin(0, &[cell_isst(0, 5)]),
            PART,
            &["only".to_owned()],
        )
        .expect_err("index out of range");

        assert!(matches!(
            err,
            XlsbError::Malformed { ref reason, .. } if reason.contains("index 5")
        ));
    }

    #[test]
    fn reads_rich_string_cell() {
        let content = parse_ok(&sheet_bin(0, &[cell_r_string(0, "rich")]), &[]);

        assert_eq!(value(&content, 0), &CellValue::InlineString("rich".into()));
    }

    /// У `BrtFmlaNum` значение идёт перед формулой: оно сохраняется, а сама
    /// формула первого среза не разбирается.
    #[test]
    fn keeps_formula_value_without_formula() {
        let mut data = crate::test_util::cell(0, 0);
        data.extend_from_slice(&7.0_f64.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes()); // grbit
        data.extend_from_slice(&0_u32.to_le_bytes()); // размер формулы
        let bytes = sheet_bin(0, &[record(ids::BRT_FMLA_NUM, &data)]);

        let content = parse_ok(&bytes, &[]);

        assert_eq!(value(&content, 0), &CellValue::Number(7.0));
        assert_eq!(
            content
                .cells
                .cell(CellRef::new(0, 0))
                .expect("cell")
                .formula,
            None
        );
    }

    #[test]
    fn cell_before_row_header_is_malformed() {
        let mut bytes = record(ids::BRT_BEGIN_SHEET, &[]);
        bytes.extend(cell_real(0, 1.0));

        let err = parse(&bytes, PART, &[]).expect_err("cell without row");

        assert!(matches!(
            err,
            XlsbError::Malformed { ref reason, .. } if reason.contains("BrtRowHdr")
        ));
    }

    /// Порядок строк проверяет сборщик `xlsx`: ошибка должна дойти с частью
    /// и смещением записи, а не потеряться.
    #[test]
    fn descending_rows_report_the_offending_record_offset() {
        let mut bytes = record(ids::BRT_BEGIN_SHEET, &[]);
        bytes.extend(record(ids::BRT_ROW_HDR, &1_u32.to_le_bytes()));
        bytes.extend(cell_real(0, 1.0));
        let offset = bytes.len();
        bytes.extend(record(ids::BRT_ROW_HDR, &0_u32.to_le_bytes()));
        bytes.extend(cell_real(0, 2.0));

        let err = parse(&bytes, PART, &[]).expect_err("rows go backwards");

        match err {
            XlsbError::Malformed {
                part,
                offset: at,
                reason,
            } => {
                assert_eq!(part, PART);
                assert_eq!(at, offset + 6, "offset must be the cell record");
                assert!(reason.contains("ascending order"), "{reason}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn unknown_records_are_ignored() {
        let mut bytes = record(ids::BRT_BEGIN_SHEET, &[]);
        bytes.extend(record(147, &[0xAB; 9])); // BrtWsProp
        bytes.extend(record(ids::BRT_BEGIN_SHEET_DATA, &[]));
        bytes.extend(record(ids::BRT_ROW_HDR, &0_u32.to_le_bytes()));
        bytes.extend(cell_real(0, 3.0));
        bytes.extend(record(148, &[0xCD; 16])); // BrtWsDim
        bytes.extend(record(ids::BRT_END_SHEET_DATA, &[]));

        let content = parse_ok(&bytes, &[]);
        assert_eq!(content.cells.cell_count(), 1);
    }

    #[test]
    fn truncated_row_header_is_malformed() {
        let mut bytes = record(ids::BRT_BEGIN_SHEET, &[]);
        let offset = bytes.len();
        bytes.extend(record(ids::BRT_ROW_HDR, &[0x01, 0x00]));

        let err = parse(&bytes, PART, &[]).expect_err("truncated row");

        assert!(matches!(
            err,
            XlsbError::Malformed { offset: at, ref reason, .. }
                if at == offset && reason.contains("BrtRowHdr")
        ));
    }
}
