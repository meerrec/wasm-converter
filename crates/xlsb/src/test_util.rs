//! Синтетические записи BIFF12 и ZIP-пакеты для тестов.
//!
//! Байты собираются вручную по MS-XLSB §2.1.4 и разделам записей: так тест
//! проверяет разбор, а не пару «кодировщик — декодер» из библиотеки.

use std::io::{Cursor, Write};

use crate::reader::ids;

/// Собрать запись: тип, размер по правилам MS-XLSB, данные.
pub(crate) fn record(id: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 6);
    encode_type(id, &mut out);
    encode_size(data.len(), &mut out);
    out.extend_from_slice(data);
    out
}

/// Тип записи: один байт до 0x80, иначе два по семь значащих бит.
fn encode_type(id: u16, out: &mut Vec<u8>) {
    if id < 0x80 {
        out.push(u8::try_from(id).expect("one-byte record type"));
    } else {
        let low = u8::try_from(id & 0x7F).expect("seven bits");
        let high = u8::try_from(id >> 7).expect("record type fits two bytes");
        out.push(0x80 | low);
        out.push(high);
    }
}

/// Размер записи: группы по семь бит, старший бит — продолжение.
fn encode_size(size: usize, out: &mut Vec<u8>) {
    let mut value = size;
    loop {
        let mut byte = u8::try_from(value & 0x7F).expect("seven bits");
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// `XLWideString`: длина в символах UTF-16 и сами символы.
pub(crate) fn wide_string(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = Vec::with_capacity(4 + units.len() * 2);
    out.extend_from_slice(
        &u32::try_from(units.len())
            .expect("short string")
            .to_le_bytes(),
    );
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

/// `XLNullableWideString`: `None` — длина `0xFFFFFFFF`.
pub(crate) fn nullable_wide_string(text: Option<&str>) -> Vec<u8> {
    match text {
        Some(text) => wide_string(text),
        None => u32::MAX.to_le_bytes().to_vec(),
    }
}

/// Структура `Cell`: столбец, 24-битный индекс формата, байт флагов.
pub(crate) fn cell(col: u32, style: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&col.to_le_bytes());
    out.extend_from_slice(&style.to_le_bytes()[..3]);
    out.push(0);
    out
}

/// `BrtCellBlank`: заголовок ячейки без значения.
pub(crate) fn cell_blank(col: u32) -> Vec<u8> {
    record(ids::BRT_CELL_BLANK, &cell(col, 0))
}

/// `BrtCellReal`: число `f64`.
pub(crate) fn cell_real(col: u32, value: f64) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.extend_from_slice(&value.to_le_bytes());
    record(ids::BRT_CELL_REAL, &data)
}

/// `BrtCellRk`: упакованное число `RkNumber`.
pub(crate) fn cell_rk(col: u32, raw: u32) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.extend_from_slice(&raw.to_le_bytes());
    record(ids::BRT_CELL_RK, &data)
}

/// `BrtCellBool`: логическое значение одним байтом.
pub(crate) fn cell_bool(col: u32, value: bool) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.push(u8::from(value));
    record(ids::BRT_CELL_BOOL, &data)
}

/// `BrtCellError`: код ошибки одним байтом.
pub(crate) fn cell_error(col: u32, code: u8) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.push(code);
    record(ids::BRT_CELL_ERROR, &data)
}

/// `BrtCellSt`: строка прямо в ячейке.
pub(crate) fn cell_st(col: u32, text: &str) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.extend(wide_string(text));
    record(ids::BRT_CELL_ST, &data)
}

/// `BrtCellIsst`: индекс в общей таблице строк.
pub(crate) fn cell_isst(col: u32, index: u32) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.extend_from_slice(&index.to_le_bytes());
    record(ids::BRT_CELL_ISST, &data)
}

/// `BrtCellRString`: `RichStr` без прогонов форматирования.
pub(crate) fn cell_r_string(col: u32, text: &str) -> Vec<u8> {
    let mut data = cell(col, 0);
    data.push(0); // fRichStr = 0, fExtStr = 0
    data.extend(wide_string(text));
    record(ids::BRT_CELL_R_STRING, &data)
}

/// Описание листа для [`workbook_bin`].
pub(crate) struct SheetSpec<'a> {
    /// Имя листа.
    pub(crate) name: &'a str,
    /// `strRelID`; `None` — NULL-строка.
    pub(crate) rel_id: Option<&'a str>,
    /// `iTabID`.
    pub(crate) tab_id: u32,
    /// `hsState`: 0 — видимый, 1 — скрытый, 2 — очень скрытый.
    pub(crate) state: u32,
}

/// Собрать `xl/workbook.bin` с каталогом листов.
pub(crate) fn workbook_bin(sheets: &[SheetSpec<'_>], date1904: bool) -> Vec<u8> {
    let mut out = record(ids::BRT_BEGIN_BOOK, &[]);

    let mut props = Vec::new();
    props.extend_from_slice(&u32::from(date1904).to_le_bytes());
    props.extend_from_slice(&0_u32.to_le_bytes()); // dwThemeVersion
    props.extend(wide_string("Book")); // CodeName
    out.extend(record(ids::BRT_WB_PROP, &props));

    out.extend(record(ids::BRT_BEGIN_BUNDLE_SHS, &[]));
    for sheet in sheets {
        let mut data = Vec::new();
        data.extend_from_slice(&sheet.state.to_le_bytes());
        data.extend_from_slice(&sheet.tab_id.to_le_bytes());
        data.extend(nullable_wide_string(sheet.rel_id));
        data.extend(wide_string(sheet.name));
        out.extend(record(ids::BRT_BUNDLE_SH, &data));
    }
    out.extend(record(ids::BRT_END_BUNDLE_SHS, &[]));
    out.extend(record(ids::BRT_END_BOOK, &[]));
    out
}

/// Собрать `xl/sharedStrings.bin` из строк без прогонов форматирования.
pub(crate) fn sst_bin(strings: &[&str]) -> Vec<u8> {
    let count = u32::try_from(strings.len()).expect("short table");
    let mut data = Vec::new();
    data.extend_from_slice(&count.to_le_bytes()); // cstTotal
    data.extend_from_slice(&count.to_le_bytes()); // cstUnique
    let mut out = record(ids::BRT_BEGIN_SST, &data);
    for text in strings {
        let mut item = vec![0_u8]; // fRichStr = 0, fExtStr = 0
        item.extend(wide_string(text));
        out.extend(record(ids::BRT_SST_ITEM, &item));
    }
    out
}

/// Собрать `xl/worksheets/sheetN.bin` из готовых записей ячеек одной строки.
///
/// `BrtRowHdr` содержит только `rw`: остальные поля (высота, флаги, диапазоны
/// столбцов) читателю первого среза не нужны.
pub(crate) fn sheet_bin(row: u32, cells: &[Vec<u8>]) -> Vec<u8> {
    let mut out = record(ids::BRT_BEGIN_SHEET, &[]);
    out.extend(record(ids::BRT_BEGIN_SHEET_DATA, &[]));
    out.extend(record(ids::BRT_ROW_HDR, &row.to_le_bytes()));
    for cell in cells {
        out.extend_from_slice(cell);
    }
    out.extend(record(ids::BRT_END_SHEET_DATA, &[]));
    out.extend(record(ids::BRT_END_SHEET, &[]));
    out
}

/// ZIP-пакет из пар «имя части — байты».
///
/// Сжатие `Stored`: пакет должен быть детерминированным, а скорость теста не
/// зависит от компрессора.
pub(crate) fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
        for (name, body) in entries {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file(*name, options).expect("start file");
            writer.write_all(body).expect("write part");
        }
        writer.finish().expect("finish zip");
    }
    buffer
}
