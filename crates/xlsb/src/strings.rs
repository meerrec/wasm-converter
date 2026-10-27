//! Разбор `xl/sharedStrings.bin` — общей таблицы строк книги.
//!
//! Таблица — это `BrtBeginSst` и последовательность `BrtSSTItem`. Каждый
//! элемент начинается с байта флагов `RichStr`: `fRichStr` и `fExtStr`
//! сообщают о прогонах форматирования и фонетических подсказках, которые идут
//! после текста. Первый срез берёт только текст: модель `xlsx` хранит строку
//! целиком, без украшений внутри ячейки.

use crate::error::{Result, XlsbError};
use crate::reader::{ids, DataCursor, RecordReader};

/// Прочитать общую таблицу строк из байтов части `part`.
///
/// # Errors
/// [`XlsbError::Malformed`] — запись оборвана или `BrtSSTItem` короче своих
/// полей.
pub(crate) fn parse(bytes: &[u8], part: &str) -> Result<Vec<String>> {
    let mut reader = RecordReader::new(bytes, part);
    let mut items = Vec::new();

    while let Some(record) = reader.next()? {
        let offset = reader.last_record_offset();
        match record.id {
            // Заголовок нужен только чтобы проверить форму записи: счётчики
            // `cstTotal`/`cstUnique` — подсказка, а не источник истины, и
            // доверять им при выделении памяти нельзя.
            ids::BRT_BEGIN_SST => {
                let mut cursor = DataCursor::new(record.data);
                let _total = cursor
                    .u32()
                    .ok_or_else(|| truncated(part, offset, "BrtBeginSst"))?;
                cursor
                    .u32()
                    .ok_or_else(|| truncated(part, offset, "BrtBeginSst"))?;
            }
            ids::BRT_SST_ITEM => {
                let mut cursor = DataCursor::new(record.data);
                // Байт флагов RichStr: fRichStr/fExtStr.
                cursor
                    .u8()
                    .ok_or_else(|| truncated(part, offset, "BrtSSTItem"))?;
                let text = cursor
                    .wide_string()
                    .ok_or_else(|| truncated(part, offset, "BrtSSTItem"))?;
                items.push(text);
            }
            _ => {}
        }
    }

    Ok(items)
}

/// Ошибка «запись короче своих полей».
fn truncated(part: &str, offset: usize, record: &str) -> XlsbError {
    XlsbError::malformed(part, offset, format!("{record} is truncated"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{record, sst_bin, wide_string};

    const PART: &str = "xl/sharedStrings.bin";

    #[test]
    fn reads_strings_in_order() {
        let items = parse(&sst_bin(&["first", "", "третий"]), PART).expect("table");

        assert_eq!(items, vec!["first", "", "третий"]);
    }

    #[test]
    fn no_begin_sst_is_an_empty_table() {
        let items = parse(&[], PART).expect("empty table");

        assert!(items.is_empty());
    }

    /// `fRichStr` и `fExtStr` первого среза не касаются: текст всё равно
    /// читается, а прогоны форматирования остаются в записи.
    #[test]
    fn ignores_rich_run_flags() {
        let mut item = vec![0b0000_0011_u8];
        item.extend(wide_string("styled"));
        let mut data = Vec::new();
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        let mut bytes = record(ids::BRT_BEGIN_SST, &data);
        bytes.extend(record(ids::BRT_SST_ITEM, &item));

        let items = parse(&bytes, PART).expect("table");

        assert_eq!(items, vec!["styled"]);
    }

    #[test]
    fn truncated_item_reports_its_offset() {
        let mut data = Vec::new();
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        let mut bytes = record(ids::BRT_BEGIN_SST, &data);
        let offset = bytes.len();
        // Байт флагов и объявленная длина 4 без самих символов.
        let mut item = vec![0_u8];
        item.extend_from_slice(&4_u32.to_le_bytes());
        bytes.extend(record(ids::BRT_SST_ITEM, &item));

        let err = parse(&bytes, PART).expect_err("truncated item");

        assert!(matches!(
            err,
            XlsbError::Malformed { offset: at, ref reason, .. }
                if at == offset && reason.contains("BrtSSTItem")
        ));
    }
}
