//! Настройки печати листа: масштаб, сетка, колонтитулы и повторяемые
//! заголовки.
//!
//! Большая часть настроек лежит в самой части листа (`sheetPr`, `pageSetup`,
//! `printOptions`, `headerFooter`). Печатаемые заголовки — исключение: Excel
//! хранит их в `definedNames` книги ссылкой на имя листа, поэтому их дописывает
//! [`open`](crate::open) после разбора листа.
//!
//! Всё отсутствующее — валидный дефолт ECMA-376, а не ошибка: лист без
//! настроек печати — норма, которую Excel пишет постоянно.

use doc_converter_core::xml::XmlReader;
use quick_xml::events::Event;

use crate::cellref::{MAX_COL, MAX_ROW};
use crate::error::{Result, XlsxError};
use crate::xml::{find, is_true, resolve_reference, Attr};

/// Ориентация страницы — `orientation` в `<pageSetup>` (`ST_Orientation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// `default` или отсутствие атрибута: ориентация принтера.
    #[default]
    Default,
    /// `portrait`.
    Portrait,
    /// `landscape`.
    Landscape,
}

/// Полоса строк или столбцов: границы включительные, 0-based, как и везде в
/// модели. Файловая запись `$A:$B` — это `first: 0, last: 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Первый индекс полосы.
    pub first: u32,
    /// Последний индекс полосы, включительно.
    pub last: u32,
}

/// Настройки печати листа.
///
/// Поля заполняются из части листа, кроме
/// [`repeat_header_rows`](Self::repeat_header_rows) и
/// [`repeat_first_columns`](Self::repeat_first_columns): их источник —
/// `definedNames` книги, и дописывает их [`open`](crate::open).
// Флаги печати независимы, как атрибуты схемы: это не состояния одной машины.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintSettings {
    /// `<pageSetUpPr fitToPage="1">`: уместить лист на страницы.
    ///
    /// При этом признаке `scale` игнорируется, а работают
    /// [`fit_to_width`](Self::fit_to_width) и
    /// [`fit_to_height`](Self::fit_to_height) — семантика Excel.
    pub fit_to_page: bool,
    /// Код бумаги `paperSize` (ECMA-376, например 9 — A4); `None` — атрибут не
    /// задан, бумага принтера.
    pub paper_size: Option<u32>,
    /// Ориентация страницы.
    pub orientation: Orientation,
    /// Масштаб в процентах, `scale`.
    pub scale: u32,
    /// Число страниц по ширине, `fitToWidth` (дефолт схемы — 1).
    pub fit_to_width: u32,
    /// Число страниц по высоте, `fitToHeight` (дефолт схемы — 1).
    pub fit_to_height: u32,
    /// `gridLines` — печатать сетку.
    pub grid_lines: bool,
    /// `headings` — печатать номера строк и заголовки столбцов.
    pub headings: bool,
    /// `horizontalCentered` — центрировать содержимое по ширине.
    pub horizontal_centered: bool,
    /// `verticalCentered` — центрировать содержимое по высоте.
    pub vertical_centered: bool,
    /// Сырой текст `<oddHeader>`: коды вида `&C` не интерпретируются.
    pub odd_header: Option<String>,
    /// Сырой текст `<oddFooter>`.
    pub odd_footer: Option<String>,
    /// Повторяемые строки заголовков из `_xlnm.Print_Titles` книги (`$1:$2`).
    pub repeat_header_rows: Option<Span>,
    /// Повторяемые первые столбцы из `_xlnm.Print_Titles` книги (`$A:$B`).
    pub repeat_first_columns: Option<Span>,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            fit_to_page: false,
            paper_size: None,
            orientation: Orientation::Default,
            // Дефолты ECMA-376 (CT_PageSetup): 100% и одна страница в каждую
            // сторону.
            scale: 100,
            fit_to_width: 1,
            fit_to_height: 1,
            grid_lines: false,
            headings: false,
            horizontal_centered: false,
            vertical_centered: false,
            odd_header: None,
            odd_footer: None,
            repeat_header_rows: None,
            repeat_first_columns: None,
        }
    }
}

/// Печатаемые заголовки одного листа из `_xlnm.Print_Titles`.
///
/// Значение `definedName` перечисляет ссылки на лист через запятую: строки
/// шапки (`$1:$2`) и первые столбцы (`$A:$B`). Ссылок каждого вида не больше
/// одной, но при дубликатах полосы объединяются, а не теряются.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintTitles {
    /// Имя листа, как оно записано в ссылке.
    pub sheet: String,
    /// Полоса строк, если была ссылка на строки.
    pub rows: Option<Span>,
    /// Полоса столбцов, если была ссылка на столбцы.
    pub cols: Option<Span>,
}

/// Прочитать `<pageSetUpPr>`: пока только признак `fitToPage`.
pub(crate) fn read_page_setup_pr(print: &mut PrintSettings, attrs: &[Attr<'_>]) {
    if let Some(value) = find(attrs, "fitToPage") {
        print.fit_to_page = is_true(value);
    }
}

/// Прочитать `<pageSetup>`: бумага, ориентация, масштаб и размеры в страницах.
///
/// Незнакомая ориентация — не порча файла: остаётся ориентация принтера.
pub(crate) fn read_page_setup(print: &mut PrintSettings, attrs: &[Attr<'_>]) {
    if let Some(paper) = unsigned(attrs, "paperSize") {
        print.paper_size = Some(paper);
    }
    if let Some(orientation) = find(attrs, "orientation") {
        print.orientation = match orientation {
            "portrait" => Orientation::Portrait,
            "landscape" => Orientation::Landscape,
            _ => Orientation::Default,
        };
    }
    if let Some(scale) = unsigned(attrs, "scale") {
        print.scale = scale;
    }
    if let Some(width) = unsigned(attrs, "fitToWidth") {
        print.fit_to_width = width;
    }
    if let Some(height) = unsigned(attrs, "fitToHeight") {
        print.fit_to_height = height;
    }
}

/// Прочитать `<printOptions>`: сетка, заголовки строк и столбцов,
/// центрирование. Отсутствующий атрибут ничего не меняет — атрибуты
/// независимы.
pub(crate) fn read_print_options(print: &mut PrintSettings, attrs: &[Attr<'_>]) {
    for (name, field) in [
        ("gridLines", &mut print.grid_lines),
        ("headings", &mut print.headings),
        ("horizontalCentered", &mut print.horizontal_centered),
        ("verticalCentered", &mut print.vertical_centered),
    ] {
        if let Some(value) = find(attrs, name) {
            *field = is_true(value);
        }
    }
}

/// Беззнаковое число из атрибута; отсутствующее или неразбираемое значение —
/// `None` (неизвестный атрибут не должен ронять разбор).
fn unsigned(attrs: &[Attr<'_>], name: &str) -> Option<u32> {
    find(attrs, name).and_then(|value| value.trim().parse().ok())
}

/// Дочитать прямой текст элемента до парного закрывающего тега.
///
/// Так записаны `<oddHeader>`/`<oddFooter>` и значения `<definedName>`: текст
/// лежит прямо в элементе, а не в `<t>`, как в строках ячеек. Ссылки на
/// сущности разворачиваются — `&amp;C` в колонтитуле обязан стать `&C`, иначе
/// коды форматирования поедут.
///
/// # Errors
///
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] — файл
/// оборвался внутри элемента или испорчена ссылка на сущность.
pub(crate) fn read_element_text(
    reader: &mut XmlReader<'_>,
    part: &str,
    tag: &str,
) -> Result<String> {
    let mut text = String::new();
    while let Some(event) = reader.next_significant()? {
        match event {
            Event::Text(chunk) => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| XlsxError::malformed(part, format!("bad text: {e}")))?;
                text.push_str(&decoded);
            }
            // Ссылки на сущности quick-xml отдаёт отдельным событием.
            Event::GeneralRef(reference) => text.push_str(&resolve_reference(&reference, part)?),
            Event::CData(chunk) => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| XlsxError::malformed(part, format!("bad CDATA: {e}")))?;
                text.push_str(&decoded);
            }
            Event::End(end) if end.local_name().as_ref() == tag.as_bytes() => return Ok(text),
            _ => {}
        }
    }
    Err(XlsxError::malformed(
        part,
        format!("unexpected end of file inside <{tag}>"),
    ))
}

/// Разобрать значение `_xlnm.Print_Titles` в заголовки по листам.
///
/// Значение — список ссылок через запятую: `'Лист 1'!$A:$B,'Лист 1'!$1:$2`.
/// Имя листа заключается в кавычки, если содержит пробелы или спецсимволы;
/// внутри кавычек `''` — это одна кавычка. Всё, что не похоже на полосу строк
/// или столбцов, пропускается: чужая ссылка не повод не открыть книгу.
pub(crate) fn parse_print_titles(value: &str) -> Vec<PrintTitles> {
    let mut titles: Vec<PrintTitles> = Vec::new();
    for item in split_references(value) {
        let Some((sheet, range)) = split_sheet(item) else {
            continue;
        };
        let Some((axis, span)) = span_of(range) else {
            continue;
        };
        let index = if let Some(index) = titles.iter().position(|titles| titles.sheet == sheet) {
            index
        } else {
            titles.push(PrintTitles {
                sheet,
                rows: None,
                cols: None,
            });
            titles.len() - 1
        };
        let entry = &mut titles[index];
        match axis {
            Axis::Rows => entry.rows = Some(widen(entry.rows, span)),
            Axis::Cols => entry.cols = Some(widen(entry.cols, span)),
        }
    }
    titles
}

/// Ось ссылки печатаемых заголовков.
enum Axis {
    /// `$1:$2` — строки.
    Rows,
    /// `$A:$B` — столбцы.
    Cols,
}

/// Разбить значение по запятым, не тронув их внутри кавычек имени листа.
fn split_references(value: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    let bytes = value.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'\'' => quoted = !quoted,
            b',' if !quoted => {
                items.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    items.push(&value[start..]);
    items
}

/// Отделить имя листа от ссылки: `'Лист 1'!$1:$2` или `Лист1!$1:$2`.
///
/// Внутри кавычек удвоенная кавычка (`''`) — часть имени.
fn split_sheet(item: &str) -> Option<(String, &str)> {
    let item = item.trim();
    let Some(quoted) = item.strip_prefix('\'') else {
        let (name, range) = item.split_once('!')?;
        if name.is_empty() {
            return None;
        }
        return Some((name.to_owned(), range));
    };
    let bytes = quoted.as_bytes();
    let mut name = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if bytes.get(index + 1) == Some(&b'\'') {
                name.push('\'');
                index += 2;
                continue;
            }
            let range = quoted.get(index + 1..)?.strip_prefix('!')?;
            if name.is_empty() {
                return None;
            }
            return Some((name, range));
        }
        // Имя может быть не-ASCII (кириллица), поэтому копируем посимвольно.
        let ch = quoted[index..].chars().next()?;
        name.push(ch);
        index += ch.len_utf8();
    }
    None
}

/// Разобрать ссылку `$1:$2` (строки) или `$A:$B` (столбцы).
///
/// Границы в файле 1-based, в модели — 0-based; `$` необязателен. Ссылка вида
/// `$A$1:$B$2` заголовками не бывает и пропускается, как и выход за лимиты
/// Excel.
fn span_of(range: &str) -> Option<(Axis, Span)> {
    let (first, last) = range.trim().split_once(':')?;
    let first = first.trim().trim_start_matches('$');
    let last = last.trim().trim_start_matches('$');
    if let (Ok(first), Ok(last)) = (first.parse::<u32>(), last.parse::<u32>()) {
        let first = first.checked_sub(1)?;
        let last = last.checked_sub(1)?;
        if first > last || last > MAX_ROW {
            return None;
        }
        return Some((Axis::Rows, Span { first, last }));
    }
    let first = column(first)?;
    let last = column(last)?;
    if first > last {
        return None;
    }
    Some((Axis::Cols, Span { first, last }))
}

/// Номер столбца 0-based из букв: `A` → 0, `XFD` → 16383.
fn column(letters: &str) -> Option<u32> {
    let mut value: u32 = 0;
    if letters.is_empty() {
        return None;
    }
    for byte in letters.bytes() {
        if !byte.is_ascii_uppercase() {
            return None;
        }
        value = value
            .checked_mul(26)?
            .checked_add(u32::from(byte - b'A') + 1)?;
        if value > MAX_COL + 1 {
            return None;
        }
    }
    Some(value - 1)
}

/// Слить полосы: ссылок одного вида в значении не больше одной, но при
/// дубликатах честнее взять объединение, чем потерять строки.
fn widen(existing: Option<Span>, span: Span) -> Span {
    match existing {
        Some(current) => Span {
            first: current.first.min(span.first),
            last: current.last.max(span.last),
        },
        None => span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_schema() {
        let print = PrintSettings::default();
        assert!(!print.fit_to_page);
        assert_eq!(print.paper_size, None);
        assert_eq!(print.orientation, Orientation::Default);
        assert_eq!(print.scale, 100);
        assert_eq!(print.fit_to_width, 1);
        assert_eq!(print.fit_to_height, 1);
        assert!(print.odd_header.is_none() && print.odd_footer.is_none());
        assert!(print.repeat_header_rows.is_none() && print.repeat_first_columns.is_none());
    }

    #[test]
    fn reads_rows_and_columns_of_a_quoted_sheet() {
        let titles = parse_print_titles("'Лист1'!$A:$B,'Лист1'!$1:$2");
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].sheet, "Лист1");
        assert_eq!(titles[0].cols, Some(Span { first: 0, last: 1 }));
        assert_eq!(titles[0].rows, Some(Span { first: 0, last: 1 }));
    }

    #[test]
    fn reads_an_unquoted_sheet_name() {
        let titles = parse_print_titles("Sheet1!$2:$3");
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].sheet, "Sheet1");
        assert_eq!(titles[0].rows, Some(Span { first: 1, last: 2 }));
        assert_eq!(titles[0].cols, None);
    }

    #[test]
    fn doubled_quotes_and_commas_stay_in_the_name() {
        let titles = parse_print_titles("'It''s, ok'!$A:$A");
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].sheet, "It's, ok");
        assert_eq!(titles[0].cols, Some(Span { first: 0, last: 0 }));
    }

    #[test]
    fn sheets_are_kept_apart() {
        let titles = parse_print_titles("'A'!$1:$1, B!$C:$D");
        assert_eq!(titles.len(), 2);
        assert_eq!(titles[0].sheet, "A");
        assert_eq!(titles[0].rows, Some(Span { first: 0, last: 0 }));
        assert_eq!(titles[1].sheet, "B");
        assert_eq!(titles[1].cols, Some(Span { first: 2, last: 3 }));
    }

    #[test]
    fn duplicate_ranges_widen_the_span() {
        let titles = parse_print_titles("'Лист1'!$1:$1,'Лист1'!$3:$4");
        assert_eq!(titles[0].rows, Some(Span { first: 0, last: 3 }));
    }

    #[test]
    fn garbage_is_skipped() {
        for value in [
            "",
            "Sheet1",
            "Sheet1!",
            "Sheet1!$A$1:$B$2",
            "'Unclosed!$A:$A",
            "''!$A:$A",
            "'A'!$B:$A",
            "'A'!$2:$1",
            "'A'!$1:$1048577",
        ] {
            assert!(
                parse_print_titles(value).is_empty(),
                "значение {value:?} должно быть пропущено"
            );
        }
    }
}
