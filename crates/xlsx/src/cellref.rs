//! Разбор и форматирование A1-адресов ячеек.
//!
//! Нумерация столбцов в Excel — **биективная base-26**: `A = 1`, `Z = 26`,
//! `AA = 27`, …, `XFD = 16384`. Это не «base-26 с нулём», а классическая
//! ошибка на единицу, на которой спотыкаются самодельные парсеры.
//! [`MAX_COL`] и [`MAX_ROW`] повторяют жёсткие лимиты Excel (`XFD1048576`).
//!
//! Парсер принимает необязательные анкеры `$` (`$A$1`, `A$1`, `$A1`) и
//! отбрасывает их. [`CellRef::format`] всегда печатает без анкеров,
//! [`CellRef::format_anchored`] — с ними; обе обратимы к [`CellRef::parse`].

use std::fmt;
use std::fmt::Write as _;

use miette::Diagnostic;
use thiserror::Error;

/// Максимальный 0-based индекс столбца — столбец Excel `XFD`.
pub const MAX_COL: u32 = 16_383;
/// Максимальный 0-based индекс строки — строка Excel `1048576`.
pub const MAX_ROW: u32 = 1_048_575;

/// Последний допустимый столбец в 1-based биективной нумерации.
const MAX_COL_BIJECTIVE: u32 = MAX_COL + 1;
/// Последняя допустимая строка: строки Excel нумеруются с единицы.
const MAX_ROW_ONE_BASED: u32 = MAX_ROW + 1;
/// Насыщение для распознавания слишком больших номеров строк.
const ROW_SATURATION: u32 = MAX_ROW_ONE_BASED + 1;

/// Ссылка на одну ячейку. Индексы 0-based: `A1` — это
/// `CellRef { row: 0, col: 0 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    /// Номер строки, 0-based.
    pub row: u32,
    /// Номер столбца, 0-based.
    pub col: u32,
}

impl CellRef {
    /// Собрать ссылку из 0-based индексов.
    #[must_use]
    pub const fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }

    /// Разобрать `A1`, `$A$1`, `AA10`, `XFD1048576`; анкеры `$` отбрасываются.
    ///
    /// # Errors
    ///
    /// Возвращает [`ParseError`], если строка пуста, форма ссылки нарушена,
    /// номер выходит за границы `XFD`/`1048576` или остались лишние символы.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return Err(ParseError::Empty);
        }

        let mut i = 0;
        if bytes.first() == Some(&b'$') {
            i += 1;
        }

        let (col, next) = parse_column(bytes, i)?;
        i = next;

        if bytes.get(i) == Some(&b'$') {
            i += 1;
        }

        let (row, next) = parse_row(bytes, i)?;
        i = next;

        if i != bytes.len() {
            // Всё разобранное выше — ASCII, поэтому `i` стоит на границе
            // символа UTF-8 и срез не паникует.
            let ch = s[i..].chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
            return Err(ParseError::Unexpected { ch, pos: i });
        }

        Ok(Self { row, col })
    }

    /// Напечатать без анкеров: `A1`. Обратна [`CellRef::parse`].
    #[must_use]
    pub fn format(&self) -> String {
        let mut s = String::with_capacity(8);
        push_column(&mut s, self.col);
        push_row(&mut s, self.row);
        s
    }

    /// Напечатать с анкерами: `$A$1`.
    #[must_use]
    pub fn format_anchored(&self) -> String {
        let mut s = String::with_capacity(10);
        s.push('$');
        push_column(&mut s, self.col);
        s.push('$');
        push_row(&mut s, self.row);
        s
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format())
    }
}

/// Прямоугольный диапазон `first..=last` включительно.
///
/// Границы не нормализуются: `B10:A1` сохраняется как есть — вызывайте
/// [`Range::normalized`], когда нужен порядок.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// Первая ячейка, как она записана.
    pub first: CellRef,
    /// Последняя ячейка, как она записана.
    pub last: CellRef,
}

impl Range {
    /// Разобрать `A1:B10`. Двоеточие обязательно: одиночная ссылка — не
    /// диапазон.
    ///
    /// # Errors
    ///
    /// Возвращает [`ParseError`], если разделитель `:` отсутствует или любая из
    /// границ не разбирается как [`CellRef`].
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let (a, b) = s.split_once(':').ok_or(ParseError::RangeMissingColon)?;
        Ok(Self {
            first: CellRef::parse(a)?,
            last: CellRef::parse(b)?,
        })
    }

    /// Разобрать `A1:B10` или одиночную ячейку `A1`.
    ///
    /// Так записаны ссылки в `ref` у `<dimension>`, `<mergeCell>` и
    /// `<hyperlink>`: диапазон из одной ячейки кодируется без двоеточия, и это
    /// законно, а не порча файла.
    ///
    /// # Errors
    ///
    /// Возвращает [`ParseError`], если строка — не диапазон и не одиночная
    /// ссылка.
    pub fn parse_ref(s: &str) -> Result<Self, ParseError> {
        if let Ok(range) = Self::parse(s) {
            return Ok(range);
        }
        let cell = CellRef::parse(s)?;
        Ok(Self {
            first: cell,
            last: cell,
        })
    }

    /// Поменять границы местами, если нужно, чтобы `first <= last`
    /// покомпонентно.
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            first: CellRef::new(
                self.first.row.min(self.last.row),
                self.first.col.min(self.last.col),
            ),
            last: CellRef::new(
                self.first.row.max(self.last.row),
                self.first.col.max(self.last.col),
            ),
        }
    }

    /// Проверить, попадает ли ячейка в диапазон; границы нормализуются.
    #[must_use]
    pub fn contains(&self, cell: CellRef) -> bool {
        let n = self.normalized();
        cell.row >= n.first.row
            && cell.row <= n.last.row
            && cell.col >= n.first.col
            && cell.col <= n.last.col
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.first, self.last)
    }
}

/// Ошибка разбора A1-ссылки.
#[derive(Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum ParseError {
    /// Строка пуста.
    #[error("empty cell reference")]
    Empty,

    /// На месте столбца нет заглавных латинских букв.
    #[error("expected a column letter at position {0}")]
    ExpectedColumn(usize),

    /// На месте строки нет цифр.
    #[error("expected a row digit at position {0}")]
    ExpectedRow(usize),

    /// Столбец за пределами `XFD`.
    #[error("column `{0}` is beyond the XFD limit")]
    ColumnOverflow(String),

    /// Строка за пределами `1048576`.
    #[error("row `{0}` is beyond the 1048576 limit")]
    RowOverflow(String),

    /// Строка `0` — строки в Excel нумеруются с единицы.
    #[error("rows are numbered from 1, so 0 is not a valid row")]
    RowZero,

    /// В записи диапазона нет `:`.
    #[error("range is missing the `:` separator")]
    RangeMissingColon,

    /// Лишний символ после корректной ссылки.
    #[error("unexpected character `{ch}` at position {pos}")]
    Unexpected {
        /// Встреченный символ.
        ch: char,
        /// Позиция в байтах от начала строки.
        pos: usize,
    },
}

/// Разобрать буквы столбца начиная с `start`.
///
/// Возвращает 0-based индекс столбца и позицию за последней буквой.
fn parse_column(bytes: &[u8], start: usize) -> Result<(u32, usize), ParseError> {
    let mut value: u32 = 0; // 1-based биективный номер
    let mut i = start;
    while let Some(&b) = bytes.get(i) {
        if !b.is_ascii_uppercase() {
            break;
        }
        value = value
            .checked_mul(26)
            .and_then(|v| v.checked_add(u32::from(b - b'A') + 1))
            .ok_or_else(|| ParseError::ColumnOverflow(column_letters(value)))?;
        if value > MAX_COL_BIJECTIVE {
            return Err(ParseError::ColumnOverflow(column_letters(value)));
        }
        i += 1;
    }
    if i == start {
        return Err(ParseError::ExpectedColumn(start));
    }
    Ok((value - 1, i))
}

/// Разобрать цифры строки начиная с `start`.
///
/// Возвращает 0-based индекс строки и позицию за последней цифрой.
fn parse_row(bytes: &[u8], start: usize) -> Result<(u32, usize), ParseError> {
    let mut value: u32 = 0; // 1-based номер строки
    let mut i = start;
    while let Some(&b) = bytes.get(i) {
        if !b.is_ascii_digit() {
            break;
        }
        // Насыщение вместо переполнения: всё, что больше `1048576`, заведомо
        // невалидно, а точное число нужно только для текста ошибки.
        value = (value * 10 + u32::from(b - b'0')).min(ROW_SATURATION);
        i += 1;
    }
    if i == start {
        return Err(ParseError::ExpectedRow(start));
    }
    if value == 0 {
        return Err(ParseError::RowZero);
    }
    if value > MAX_ROW_ONE_BASED {
        return Err(ParseError::RowOverflow(
            String::from_utf8_lossy(&bytes[start..i]).into_owned(),
        ));
    }
    Ok((value - 1, i))
}

/// Имя столбца по 0-based индексу: `0 → A`, `16383 → XFD`.
///
/// Так подписан столбец в заголовке листа.
#[must_use]
pub fn column_name(col: u32) -> String {
    column_letters(col + 1)
}

/// Номер строки по 0-based индексу: `0 → "1"`.
#[must_use]
pub fn row_name(row: u32) -> String {
    let mut out = String::with_capacity(7);
    push_row(&mut out, row);
    out
}

/// Дописать буквы столбца (0-based индекс) в `out`.
fn push_column(out: &mut String, col: u32) {
    push_column_letters(out, col + 1);
}

/// Дописать 1-based биективный номер столбца буквами (`1 → A`, `16384 → XFD`).
///
/// Ноль даёт пустую строку — на этом пути не бывает underflow.
fn push_column_letters(out: &mut String, mut n: u32) {
    // При `n <= 16384` получается максимум три буквы, восьми байт хватает
    // с запасом; вызывающие не выходят за `MAX_COL_BIJECTIVE`.
    let mut buf = [0u8; 8];
    let mut i = buf.len();
    while n > 0 {
        let rem = (n - 1) % 26;
        i -= 1;
        // `rem < 26`, поэтому приведение к `u8` без потерь.
        buf[i] = b'A' + u8::try_from(rem).unwrap_or_default();
        n = (n - 1) / 26;
    }
    out.push_str(&String::from_utf8_lossy(&buf[i..]));
}

/// Напечатать 1-based биективный номер столбца буквами.
fn column_letters(n: u32) -> String {
    let mut s = String::with_capacity(3);
    push_column_letters(&mut s, n);
    s
}

/// Дописать номер строки (0-based индекс) в `out`.
fn push_row(out: &mut String, row: u32) {
    // Запись в `String` не может упасть.
    let _ = write!(out, "{}", row + 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_parse() {
        assert_eq!(CellRef::parse("A1").unwrap(), CellRef::new(0, 0));
        assert_eq!(CellRef::parse("B1").unwrap(), CellRef::new(0, 1));
        assert_eq!(CellRef::parse("A2").unwrap(), CellRef::new(1, 0));
        assert_eq!(CellRef::parse("Z1").unwrap(), CellRef::new(0, 25));
        assert_eq!(CellRef::parse("AA1").unwrap(), CellRef::new(0, 26));
        assert_eq!(CellRef::parse("AZ1").unwrap(), CellRef::new(0, 51));
        assert_eq!(CellRef::parse("BA1").unwrap(), CellRef::new(0, 52));
    }

    #[test]
    fn anchors_are_stripped() {
        for s in ["$A$1", "A$1", "$A1"] {
            assert_eq!(CellRef::parse(s).unwrap(), CellRef::new(0, 0), "{s}");
        }
    }

    #[test]
    fn boundaries() {
        assert_eq!(
            CellRef::parse("XFD1048576").unwrap(),
            CellRef::new(MAX_ROW, MAX_COL)
        );

        assert!(matches!(
            CellRef::parse("XFE1"),
            Err(ParseError::ColumnOverflow(_))
        ));
        assert!(matches!(
            CellRef::parse("A1048577"),
            Err(ParseError::RowOverflow(_))
        ));
        assert!(matches!(CellRef::parse("A0"), Err(ParseError::RowZero)));
    }

    #[test]
    fn overflow_reports_original_text() {
        assert_eq!(
            CellRef::parse("XFE1").unwrap_err(),
            ParseError::ColumnOverflow("XFE".into())
        );
        assert_eq!(
            CellRef::parse("A9999999").unwrap_err(),
            ParseError::RowOverflow("9999999".into())
        );
        // Насыщение не даёт переполниться на заведомо длинном числе.
        assert!(matches!(
            CellRef::parse("A99999999999999999999"),
            Err(ParseError::RowOverflow(_))
        ));
    }

    #[test]
    fn round_trip_fixed() {
        for s in ["A1", "Z1", "AA1", "AZ100", "BA52", "XFD1048576"] {
            let r = CellRef::parse(s).unwrap();
            assert_eq!(r.format(), s, "round-trip сломался на {s}");
            assert_eq!(CellRef::parse(&r.format_anchored()).unwrap(), r);
        }
    }

    #[test]
    fn parse_errors() {
        assert!(matches!(CellRef::parse(""), Err(ParseError::Empty)));
        assert!(matches!(
            CellRef::parse("1A"),
            Err(ParseError::ExpectedColumn(0))
        ));
        assert!(matches!(
            CellRef::parse("A"),
            Err(ParseError::ExpectedRow(1))
        ));
        assert!(matches!(
            CellRef::parse("$"),
            Err(ParseError::ExpectedColumn(1))
        ));
        assert!(matches!(
            CellRef::parse("A1B"),
            Err(ParseError::Unexpected { ch: 'B', pos: 2 })
        ));
        assert!(matches!(
            CellRef::parse("A1:"),
            Err(ParseError::Unexpected { ch: ':', pos: 2 })
        ));
        // Строчные буквы спецификацией не предусмотрены.
        assert!(matches!(
            CellRef::parse("a1"),
            Err(ParseError::ExpectedColumn(0))
        ));
    }

    #[test]
    fn range_parse_and_normalize() {
        let r = Range::parse("B10:A1").unwrap();
        assert_eq!(r.first, CellRef::new(9, 1));
        assert_eq!(r.last, CellRef::new(0, 0));

        let n = r.normalized();
        assert_eq!(n.first, CellRef::new(0, 0));
        assert_eq!(n.last, CellRef::new(9, 1));
        assert!(n.contains(CellRef::new(5, 0)));
        assert!(!n.contains(CellRef::new(10, 0)));
        assert_eq!(r.to_string(), "B10:A1");
    }

    #[test]
    fn range_errors() {
        assert!(matches!(
            Range::parse("A1"),
            Err(ParseError::RangeMissingColon)
        ));
        assert!(matches!(Range::parse("A1:"), Err(ParseError::Empty)));
        assert!(matches!(Range::parse(":B2"), Err(ParseError::Empty)));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn round_trip_any_valid(row in 0u32..=MAX_ROW, col in 0u32..=MAX_COL) {
            let r = CellRef::new(row, col);
            prop_assert_eq!(CellRef::parse(&r.format()).unwrap(), r);
            prop_assert_eq!(CellRef::parse(&r.format_anchored()).unwrap(), r);
        }

        #[test]
        fn parse_never_panics(s in ".{0,32}") {
            let _ = CellRef::parse(&s);
            let _ = Range::parse(&s);
        }

        #[test]
        fn format_is_canonical(row in 0u32..=MAX_ROW, col in 0u32..=MAX_COL) {
            let s = CellRef::new(row, col).format();
            prop_assert_eq!(CellRef::parse(&s).unwrap().format(), s);
        }
    }
}
