//! Чтение записей BIFF12.
//!
//! Заголовок записи описан в MS-XLSB §2.1.4. Тип занимает один или два байта:
//! второй нужен тогда и только тогда, когда старший бит первого выставлен, а
//! значение складывается из семи младших бит обоих байт. Размер — от одного до
//! четырёх байт, по семь значащих бит в каждом, младшая группа в первом байте;
//! старший бит четвёртого байта спецификация велит игнорировать.
//!
//! ```
//! use doc_converter_xlsb::reader::RecordReader;
//!
//! // BrtRowHdr (0) с четырьмя байтами данных и BrtEndSheetData (146).
//! let bytes = [0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x92, 0x01, 0x00];
//! let mut reader = RecordReader::new(&bytes, "xl/worksheets/sheet1.bin");
//!
//! let row = reader.next().unwrap().unwrap();
//! assert_eq!((row.id, row.data), (0, &[0, 0, 0, 0][..]));
//!
//! let end = reader.next().unwrap().unwrap();
//! assert_eq!((end.id, end.data), (146, &[][..]));
//! assert!(reader.next().unwrap().is_none());
//! ```

use crate::error::{Result, XlsbError};

/// Идентификаторы записей BIFF12, которые понимает этот срез.
///
/// Значения — из Record Enumeration (MS-XLSB §2.3). Имена совпадают с
/// названиями записей спецификации, чтобы поиск по ним находил нужный раздел.
pub mod ids {
    /// `BrtRowHdr` (0) — начало строки листа.
    pub const BRT_ROW_HDR: u16 = 0;
    /// `BrtCellBlank` (1) — ячейка без значения.
    pub const BRT_CELL_BLANK: u16 = 1;
    /// `BrtCellRk` (2) — ячейка с `RkNumber`.
    pub const BRT_CELL_RK: u16 = 2;
    /// `BrtCellError` (3) — ячейка с кодом ошибки.
    pub const BRT_CELL_ERROR: u16 = 3;
    /// `BrtCellBool` (4) — логическая ячейка.
    pub const BRT_CELL_BOOL: u16 = 4;
    /// `BrtCellReal` (5) — ячейка с числом `f64`.
    pub const BRT_CELL_REAL: u16 = 5;
    /// `BrtCellSt` (6) — ячейка со строкой прямо в записи.
    pub const BRT_CELL_ST: u16 = 6;
    /// `BrtCellIsst` (7) — ячейка с индексом в общей таблице строк.
    pub const BRT_CELL_ISST: u16 = 7;
    /// `BrtFmlaString` (8) — формула со строковым значением.
    pub const BRT_FMLA_STRING: u16 = 8;
    /// `BrtFmlaNum` (9) — формула с числовым значением.
    pub const BRT_FMLA_NUM: u16 = 9;
    /// `BrtFmlaBool` (10) — формула с логическим значением.
    pub const BRT_FMLA_BOOL: u16 = 10;
    /// `BrtFmlaError` (11) — формула со значением-ошибкой.
    pub const BRT_FMLA_ERROR: u16 = 11;
    /// `BrtSSTItem` (19) — элемент общей таблицы строк.
    pub const BRT_SST_ITEM: u16 = 19;
    /// `BrtCellRString` (62) — ячейка с rich-строкой.
    pub const BRT_CELL_R_STRING: u16 = 62;
    /// `BrtBeginSheet` (129) — начало части листа.
    pub const BRT_BEGIN_SHEET: u16 = 129;
    /// `BrtEndSheet` (130) — конец части листа.
    pub const BRT_END_SHEET: u16 = 130;
    /// `BrtBeginBook` (131) — начало части книги.
    pub const BRT_BEGIN_BOOK: u16 = 131;
    /// `BrtEndBook` (132) — конец части книги.
    pub const BRT_END_BOOK: u16 = 132;
    /// `BrtBeginBundleShs` (143) — начало каталога листов.
    pub const BRT_BEGIN_BUNDLE_SHS: u16 = 143;
    /// `BrtEndBundleShs` (144) — конец каталога листов.
    pub const BRT_END_BUNDLE_SHS: u16 = 144;
    /// `BrtBeginSheetData` (145) — начало данных листа.
    pub const BRT_BEGIN_SHEET_DATA: u16 = 145;
    /// `BrtEndSheetData` (146) — конец данных листа.
    pub const BRT_END_SHEET_DATA: u16 = 146;
    /// `BrtWbProp` (153) — свойства книги, в том числе `f1904`.
    pub const BRT_WB_PROP: u16 = 153;
    /// `BrtBundleSh` (156) — лист в каталоге книги.
    pub const BRT_BUNDLE_SH: u16 = 156;
    /// `BrtBeginSst` (159) — начало общей таблицы строк.
    pub const BRT_BEGIN_SST: u16 = 159;
    /// `BrtEndSst` (160) — конец общей таблицы строк.
    pub const BRT_END_SST: u16 = 160;
}

/// Предел на данные одной записи.
///
/// Спецификация допускает до `2^28 - 1` байт, но запись такого размера — это
/// либо повреждение, либо попытка съесть память: настоящие книги дробят лист
/// на записи строк и ячеек, которые на порядки меньше. Предел проверяется до
/// выделения памяти, поэтому служит защитой, а не только диагностикой.
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

/// Одна запись BIFF12: тип и байты данных без заголовка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    /// Тип записи из Record Enumeration.
    pub id: u16,
    /// Данные записи; длина равна объявленному размеру.
    pub data: &'a [u8],
}

/// Читатель записей BIFF12 по буферу части пакета.
///
/// Имя части хранится внутри, чтобы ошибка сразу называла файл: вызывающий
/// передаёт его один раз в [`RecordReader::new`], а не в каждый [`next`].
///
/// [`next`]: RecordReader::next
#[derive(Debug, Clone)]
pub struct RecordReader<'a> {
    bytes: &'a [u8],
    part: String,
    /// Начало следующей записи.
    offset: usize,
    /// Начало записи, которую вернул последний успешный [`next`].
    ///
    /// [`next`]: RecordReader::next
    record_start: usize,
}

impl<'a> RecordReader<'a> {
    /// Начать чтение части `part` из её распакованных байтов.
    #[must_use]
    pub fn new(bytes: &'a [u8], part: impl Into<String>) -> Self {
        Self {
            bytes,
            part: part.into(),
            offset: 0,
            record_start: 0,
        }
    }

    /// Смещение следующей записи — то есть размер уже прочитанного.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Смещение записи, которую вернул последний [`next`].
    ///
    /// [`next`]: RecordReader::next
    #[must_use]
    pub const fn last_record_offset(&self) -> usize {
        self.record_start
    }

    /// Прочитать следующую запись; `None` — часть закончилась.
    ///
    /// # Errors
    /// [`XlsbError::Malformed`] — заголовок или данные записи оборваны;
    /// [`XlsbError::Limit`] — объявленный размер больше [`MAX_RECORD_BYTES`].
    ///
    // Имя из терминологии BIFF12 и из постановки задачи; `Iterator` не
    // подходит: обрыв части — не элемент потока, а ошибка всего разбора, и
    // растворять её в `Item` значило бы заставлять каждого вызывающего
    // разворачивать `Result` внутри цикла.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Record<'a>>> {
        let start = self.offset;
        let bytes = self.bytes;
        if start == bytes.len() {
            return Ok(None);
        }

        let mut pos = start;
        let first = *bytes
            .get(pos)
            .ok_or_else(|| self.malformed(start, "record type is missing"))?;
        pos += 1;
        let id = if first & 0x80 == 0 {
            u16::from(first)
        } else {
            let second = *bytes
                .get(pos)
                .ok_or_else(|| self.malformed(start, "two-byte record type is truncated"))?;
            pos += 1;
            u16::from(first & 0x7F) | (u16::from(second & 0x7F) << 7)
        };

        let mut size: usize = 0;
        for shift in [0_u32, 7, 14, 21] {
            let byte = *bytes
                .get(pos)
                .ok_or_else(|| self.malformed(start, "record size is truncated"))?;
            pos += 1;
            size |= usize::from(byte & 0x7F) << shift;
            // Старший бит четвёртого байта спецификация велит игнорировать:
            // после четырёх байт продолжения нет.
            if byte & 0x80 == 0 || shift == 21 {
                break;
            }
        }

        if size > MAX_RECORD_BYTES {
            return Err(XlsbError::Limit(format!(
                "record {id} in `{}` declares {size} bytes of data, the limit is {MAX_RECORD_BYTES}",
                self.part
            )));
        }

        let end = pos
            .checked_add(size)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| {
                self.malformed(
                    start,
                    format!(
                        "record {id} declares {size} bytes of data, but only {} are left",
                        bytes.len() - pos
                    ),
                )
            })?;

        self.offset = end;
        self.record_start = start;
        Ok(Some(Record {
            id,
            data: &bytes[pos..end],
        }))
    }

    /// Ошибка с частью и смещением записи.
    fn malformed(&self, offset: usize, reason: impl Into<String>) -> XlsbError {
        XlsbError::malformed(self.part.clone(), offset, reason)
    }
}

/// Значение `XLNullableWideString`.
///
/// Отдельный тип, а не `Option<Option<String>>`: NULL и пустая строка — разные
/// состояния `BrtBundleSh.strRelID`, и различать их должен тип, а не
/// комментарий.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NullableWideString {
    /// Длина `0xFFFFFFFF` — строки нет.
    Null,
    /// Строка, возможно пустая.
    Text(String),
}

/// Курсор по данным одной записи.
///
/// Все методы возвращают `None`, если данных не хватило: как назвать ошибку,
/// решает вызывающий — только он знает, какое поле читает.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DataCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> DataCursor<'a> {
    /// Встать на начало данных записи.
    pub(crate) const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Прочитать один байт.
    pub(crate) fn u8(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }

    /// Прочитать `u32` little-endian.
    pub(crate) fn u32(&mut self) -> Option<u32> {
        let end = self.pos.checked_add(4)?;
        let bytes: [u8; 4] = self.data.get(self.pos..end)?.try_into().ok()?;
        self.pos = end;
        Some(u32::from_le_bytes(bytes))
    }

    /// Прочитать `f64` little-endian.
    pub(crate) fn f64(&mut self) -> Option<f64> {
        let end = self.pos.checked_add(8)?;
        let bytes: [u8; 8] = self.data.get(self.pos..end)?.try_into().ok()?;
        self.pos = end;
        Some(f64::from_le_bytes(bytes))
    }

    /// Прочитать `XLWideString`: длину в символах и сами символы UTF-16LE.
    pub(crate) fn wide_string(&mut self) -> Option<String> {
        let chars = self.u32()?;
        self.take_utf16(chars)
    }

    /// Прочитать `XLNullableWideString`; `0xFFFFFFFF` — строки нет.
    pub(crate) fn nullable_wide_string(&mut self) -> Option<NullableWideString> {
        let chars = self.u32()?;
        if chars == u32::MAX {
            return Some(NullableWideString::Null);
        }
        Some(NullableWideString::Text(self.take_utf16(chars)?))
    }

    /// Прочитать `chars` символов UTF-16LE, начиная с текущей позиции.
    fn take_utf16(&mut self, chars: u32) -> Option<String> {
        let len = usize::try_from(chars).ok()?.checked_mul(2)?;
        let end = self.pos.checked_add(len)?;
        let bytes = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(decode_utf16(bytes))
    }
}

/// Декодировать UTF-16LE в строку.
///
/// Одиночные суррогаты заменяются на `U+FFFD`: книга с таким дефектом должна
/// открыться, а не пропасть целиком.
fn decode_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Запись с заголовком из явных байт: так проверяется кодирование длины,
    /// а не пара «кодировщик — декодер» из `test_util`.
    fn raw(header: &[u8], size: usize) -> Vec<u8> {
        let mut bytes = header.to_vec();
        bytes.extend(std::iter::repeat_n(0x5A, size));
        bytes
    }

    fn read_one(bytes: &[u8]) -> Record<'_> {
        let mut reader = RecordReader::new(bytes, "xl/worksheets/sheet1.bin");
        let record = reader.next().expect("record").expect("some record");
        assert_eq!(reader.next().expect("end"), None);
        record
    }

    #[test]
    fn empty_input_has_no_records() {
        let mut reader = RecordReader::new(&[], "xl/workbook.bin");
        assert_eq!(reader.next().expect("end"), None);
        assert_eq!(reader.offset(), 0);
    }

    #[test]
    fn reads_one_byte_record_type() {
        // BrtCellReal (5), размер 0.
        let record = read_one(&[0x05, 0x00]);
        assert_eq!(record.id, ids::BRT_CELL_REAL);
        assert!(record.data.is_empty());
    }

    #[test]
    fn reads_two_byte_record_type() {
        // BrtBundleSh (156): 0x9C = 0x80 | 28, старшие семь бит — 1.
        let record = read_one(&[0x9C, 0x01, 0x00]);
        assert_eq!(record.id, ids::BRT_BUNDLE_SH);
    }

    #[test]
    fn reads_one_byte_size() {
        let bytes = raw(&[0x05, 0x7F], 127);
        let record = read_one(&bytes);
        assert_eq!(record.data.len(), 127);
        assert_eq!(record.data[126], 0x5A);
    }

    #[test]
    fn reads_two_byte_size() {
        // 200 = 72 | 1 << 7 — пример из MS-XLSB §2.1.4.
        let bytes = raw(&[0x05, 0xC8, 0x01], 200);
        let record = read_one(&bytes);
        assert_eq!(record.data.len(), 200);
    }

    #[test]
    fn reads_three_byte_size() {
        // 16384 = 1 << 14.
        let bytes = raw(&[0x05, 0x80, 0x80, 0x01], 16_384);
        let record = read_one(&bytes);
        assert_eq!(record.data.len(), 16_384);
    }

    #[test]
    fn reads_four_byte_size() {
        // 2^21 = 2 097 152: четыре байта, все с битом продолжения.
        let bytes = raw(&[0x05, 0x80, 0x80, 0x80, 0x01], 1 << 21);
        let record = read_one(&bytes);
        assert_eq!(record.data.len(), 1 << 21);
    }

    #[test]
    fn ignores_the_high_bit_of_the_fourth_size_byte() {
        // Четвёртый байт 0x81: бит продолжения выставлен, но пятый байт
        // размером уже не считается — данные начинаются сразу за ним.
        let bytes = raw(&[0x05, 0x80, 0x80, 0x80, 0x81], 1 << 21);
        let record = read_one(&bytes);
        assert_eq!(record.data.len(), 1 << 21);
    }

    #[test]
    fn truncated_record_type_is_malformed() {
        let mut reader = RecordReader::new(&[0x9C], "xl/workbook.bin");
        let err = reader.next().expect_err("truncated type");

        assert!(matches!(
            err,
            XlsbError::Malformed {
                offset: 0,
                ref reason,
                ..
            } if reason.contains("record type")
        ));
    }

    #[test]
    fn truncated_record_size_is_malformed() {
        let mut reader = RecordReader::new(&[0x05], "xl/workbook.bin");
        let err = reader.next().expect_err("truncated size");

        assert!(matches!(
            err,
            XlsbError::Malformed {
                offset: 0,
                ref reason,
                ..
            } if reason.contains("size")
        ));
    }

    #[test]
    fn truncated_record_data_is_malformed_at_the_record_start() {
        // BrtCellReal (5), объявлено 4 байта, есть 2.
        let mut reader = RecordReader::new(&[0x05, 0x04, 0xAA, 0xBB], "xl/workbook.bin");
        let err = reader.next().expect_err("truncated data");

        match err {
            XlsbError::Malformed {
                part,
                offset,
                reason,
            } => {
                assert_eq!(part, "xl/workbook.bin");
                assert_eq!(offset, 0);
                assert!(reason.contains("only 2 are left"), "{reason}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn oversized_record_is_a_limit_error() {
        // 2^24 = 16 МиБ ровно; +1 байт уже за пределом.
        let mut reader = RecordReader::new(&[0x05, 0x81, 0x80, 0x80, 0x08], "xl/workbook.bin");
        let err = reader.next().expect_err("too large");

        assert!(matches!(err, XlsbError::Limit(ref reason) if reason.contains("16777217")));
    }

    #[test]
    fn wide_string_decodes_surrogate_pairs() {
        let mut data = 2_u32.to_le_bytes().to_vec();
        data.extend_from_slice(&0xD83D_u16.to_le_bytes());
        data.extend_from_slice(&0xDE00_u16.to_le_bytes());

        let mut cursor = DataCursor::new(&data);
        assert_eq!(cursor.wide_string().as_deref(), Some("😀"));
    }

    #[test]
    fn nullable_wide_string_reads_null() {
        let data = u32::MAX.to_le_bytes();
        let mut cursor = DataCursor::new(&data);

        assert_eq!(
            cursor.nullable_wide_string(),
            Some(NullableWideString::Null)
        );
        assert_eq!(cursor.u8(), None);
    }

    #[test]
    fn wide_string_declared_longer_than_data_is_none() {
        let data = 3_u32.to_le_bytes();
        let mut cursor = DataCursor::new(&data);

        assert_eq!(cursor.wide_string(), None);
    }
}
