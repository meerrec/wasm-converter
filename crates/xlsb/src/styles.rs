//! Разбор `xl/styles.bin` — таблицы стилей книги.
//!
//! Первый срез стили не применяет: ячейка сохраняет `iStyleRef` из своего
//! заголовка, а таблица возвращается пустой, поэтому `xlsx` разрешает любой
//! индекс в формат по умолчанию. Книга при этом открывается и рисуется, а
//! разбор самой объёмной части пакета остаётся следующему срезу.
//!
//! TODO(следующий срез): `BrtBeginStyleSheet`/`BrtEndStyleSheet`, `BrtFmt`,
//! `BrtBeginFonts`/`BrtFont`, `BrtBeginFills`/`BrtFill`,
//! `BrtBeginBorders`/`BrtBorder` и `BrtBeginCellXFs`/`BrtXF` — после них
//! `Cell::style` начнёт указывать на настоящие форматы, а `StyleTable`
//! перестанет быть пустой.

use doc_converter_xlsx::StyleTable;

use crate::error::Result;
use crate::reader::RecordReader;

/// Прочитать таблицу стилей из байтов части `part`.
///
/// Записи проходятся читателем, но не разбираются: обрыв записи —
/// единственное, что эта функция сейчас проверяет.
///
/// # Errors
/// [`crate::XlsbError::Malformed`] — заголовок или данные записи оборваны;
/// [`crate::XlsbError::Limit`] — запись больше [`crate::reader::MAX_RECORD_BYTES`].
pub(crate) fn parse(bytes: &[u8], part: &str) -> Result<StyleTable> {
    let mut reader = RecordReader::new(bytes, part);
    while reader.next()?.is_some() {}
    Ok(StyleTable::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{record, wide_string};

    const PART: &str = "xl/styles.bin";

    #[test]
    fn reads_the_part_and_returns_default_styles() {
        let mut bytes = record(278, &[]); // BrtBeginStyleSheet
        let mut format = 164_u16.to_le_bytes().to_vec();
        format.extend(wide_string("0.00"));
        bytes.extend(record(44, &format)); // BrtFmt
        bytes.extend(record(279, &[])); // BrtEndStyleSheet

        let styles = parse(&bytes, PART).expect("styles");

        assert!(styles.is_empty());
    }

    #[test]
    fn truncated_part_is_malformed() {
        let bytes = [0x05, 0x08, 0x00];

        let err = parse(&bytes, PART).expect_err("truncated record");

        assert!(matches!(
            err,
            crate::XlsbError::Malformed { ref part, .. } if part == PART
        ));
    }
}
