//! Стили ODF: числовые форматы, ширины столбцов и высоты строк.
//!
//! Первый срез берёт из стилей только то, без чего теряется смысл значения:
//! `number:*-style` превращается в код формата Excel (`0.00`, `0.00%`,
//! `DD"."MM"."YYYY`), `style:data-style-name` связывает его со стилем ячейки, а
//! `style:column-width`/`style:row-height` — с геометрией листа.
//!
//! Шрифт, заливка, рамки, выравнивание, скрытые строки и таблицы
//! (`style:table-cell-properties`, `style:text-properties`,
//! `style:table-properties`) пока не переносятся: у `xlsx::CellFormat` есть
//! ссылки на шрифты, заливки и рамки, а собирать их без цвета темы и стилей
//! абзацев — значит показать неверный вид. Это следующий срез; значения ячеек
//! от этого не страдают.
//!
//! `style:page-layout` (размер страницы и поля, `PrintSettings`) тоже
//! отложен: в ODF его выбирает стиль таблицы, а не лист, и связывать их надо
//! вместе с остальными свойствами таблицы.

use std::collections::BTreeMap;

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_xlsx::layout::MAX_DIGIT_WIDTH;
use doc_converter_xlsx::model::{CellFormat, StyleTable};
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};

use crate::error::{OdfError, Result};
use crate::xml::{find, is_true, map_core, resolve_reference};

/// Сколько знаков после запятой и целых разрядов максимум учитывается.
///
/// Число из атрибута приходит из недоверенного файла; `"0".repeat(u32::MAX)`
/// съел бы память ещё до проверки лимитов архива.
const MAX_DIGITS: u32 = 32;

/// Первый `numFmtId` пользовательского формата (ECMA-376: 0–163 встроены).
const FIRST_CUSTOM_FORMAT_ID: u32 = 164;

/// Накопитель стилей из `styles.xml` и `content.xml`.
#[derive(Debug, Default)]
pub(crate) struct StyleBook {
    /// `style:name` числового стиля → код формата Excel.
    number_codes: BTreeMap<String, String>,
    /// Стиль ячейки (`style:family="table-cell"`) → имя числового стиля.
    cell_data_styles: BTreeMap<String, String>,
    /// Стиль столбца → ширина в «символах» Excel.
    column_widths: BTreeMap<String, f32>,
    /// Стиль строки → высота в пунктах.
    row_heights: BTreeMap<String, f32>,
}

impl StyleBook {
    /// Собрать таблицу стилей и разрешение имён в индексы.
    ///
    /// Формат `0` — формат по умолчанию, как в OOXML: ячейка без числового
    /// стиля ссылается на него. Пользовательские коды получают идентификаторы
    /// с 164, чтобы не перекрыть встроенные.
    pub(crate) fn freeze(&self) -> CellStyles {
        let mut formats = vec![CellFormat::default()];
        let mut number_formats = BTreeMap::new();
        let mut cells = BTreeMap::new();
        let mut next_id = FIRST_CUSTOM_FORMAT_ID;

        for (name, data_style) in &self.cell_data_styles {
            let Some(code) = self.number_codes.get(data_style) else {
                continue;
            };
            let Ok(index) = u32::try_from(formats.len()) else {
                break;
            };
            formats.push(CellFormat {
                num_fmt: next_id,
                ..CellFormat::default()
            });
            number_formats.insert(next_id, code.clone());
            cells.insert(name.clone(), index);
            next_id = next_id.saturating_add(1);
        }

        CellStyles {
            table: StyleTable::new(formats, Vec::new(), Vec::new(), Vec::new(), number_formats),
            cells,
            columns: self.column_widths.clone(),
            rows: self.row_heights.clone(),
        }
    }
}

/// Разрешённые стили: таблица форматов и имена, превращённые в индексы.
#[derive(Debug, Default)]
pub(crate) struct CellStyles {
    table: StyleTable,
    cells: BTreeMap<String, u32>,
    columns: BTreeMap<String, f32>,
    rows: BTreeMap<String, f32>,
}

impl CellStyles {
    /// Индекс формата ячейки по имени её стиля; неизвестный стиль — 0.
    #[must_use]
    pub(crate) fn index_of(&self, name: Option<&str>) -> u32 {
        name.and_then(|name| self.cells.get(name).copied())
            .unwrap_or(0)
    }

    /// Ширина столбца в «символах» Excel по имени стиля.
    #[must_use]
    pub(crate) fn column_width(&self, name: Option<&str>) -> Option<f32> {
        self.columns.get(name?).copied()
    }

    /// Высота строки в пунктах по имени стиля.
    #[must_use]
    pub(crate) fn row_height(&self, name: Option<&str>) -> Option<f32> {
        self.rows.get(name?).copied()
    }

    /// Таблица стилей книги.
    #[must_use]
    pub(crate) fn table(&self) -> &StyleTable {
        &self.table
    }
}

/// Собрать стили из одной части пакета.
///
/// Функция вызывается и для `styles.xml`, и для `content.xml`: в первом лежат
/// именованные стили, во втором — автоматические (`office:automatic-styles`),
/// на которые ссылаются ячейки. Повторное объявление имени перекрывает
/// предыдущее, поэтому `content.xml` разбирается вторым.
///
/// # Errors
/// [`OdfError::Core`] — XML не разбирается; [`OdfError::Malformed`] — атрибут
/// элемента или текст не читаются.
pub(crate) fn collect(bytes: &[u8], part: &str, book: &mut StyleBook) -> Result<()> {
    let mut reader = XmlReader::preserving(bytes, part.to_owned());
    let mut parser = Parser {
        part,
        book,
        number: None,
        style: None,
        capture: None,
    };

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }
    Ok(())
}

/// Вид числового стиля: от него зависит, что дописать в конец кода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberKind {
    /// `number:number-style` — просто число.
    Number,
    /// `number:percentage-style` — значение умножается на 100 токеном `%`.
    Percentage,
    /// `number:date-style` — дата.
    Date,
    /// `number:time-style` — время или длительность.
    Time,
    /// `number:currency-style` — число с символом валюты.
    Currency,
}

/// Открытый числовой стиль.
#[derive(Debug)]
struct NumberStyle {
    /// Имя элемента, которым стиль закрывается.
    element: &'static str,
    kind: NumberKind,
    /// `style:name`, под которым стиль будут искать стили ячеек.
    name: String,
    /// Код формата Excel, собираемый по ходу разбора.
    code: String,
}

/// Открытый стиль строки или столбца: размер лежит во вложенном свойстве.
#[derive(Debug, Clone)]
struct StyleDef {
    name: String,
    kind: StyleKind,
}

/// Семейство стиля, у которого размер задан вложенным элементом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StyleKind {
    /// `style:family="table-column"`.
    Column,
    /// `style:family="table-row"`.
    Row,
}

/// Что за текст сейчас читается: литерал формата или символ валюты.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    /// `number:text` — литерал; в процентном формате `%` пропускается.
    Text,
    /// `number:currency-symbol` — символ валюты перед числом.
    Currency,
}

/// Накопитель текста вложенного элемента.
#[derive(Debug)]
struct Capture {
    element: &'static str,
    kind: CaptureKind,
    text: String,
}

/// Состояние одного прохода по части стилей.
struct Parser<'a> {
    part: &'a str,
    book: &'a mut StyleBook,
    number: Option<NumberStyle>,
    style: Option<StyleDef>,
    capture: Option<Capture>,
}

impl Parser<'_> {
    fn handle(&mut self, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.start(&element, true),
            // Пустой элемент значит то же, что открытый и сразу закрытый.
            Event::Empty(element) => self.start(&element, false),
            Event::End(element) => {
                self.end(element.local_name().as_ref());
                Ok(())
            }
            Event::Text(text) => self.text(&text),
            Event::GeneralRef(reference) => self.reference(&reference),
            _ => Ok(()),
        }
    }

    /// Открыть или обработать элемент.
    fn start(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let name = element.local_name();
        let name = name.as_ref();

        if let Some((style_element, kind)) = number_style_kind(name) {
            // Вложенный числовой стиль невозможен; второй открывать некуда.
            if self.number.is_some() {
                return Ok(());
            }
            let attrs = attrs(element, self.part)?;
            let Some(style_name) = find(&attrs, "name") else {
                return Ok(());
            };
            self.number = Some(NumberStyle {
                element: style_element,
                kind,
                name: style_name.to_owned(),
                code: String::new(),
            });
            if !open {
                self.finish_number();
            }
            return Ok(());
        }

        if name == b"style" {
            self.start_style(element)?;
            return Ok(());
        }

        if let Some(number) = self.number.as_mut() {
            match name {
                b"text" | b"currency-symbol" => {
                    // Пустой элемент текста ничего не несёт, но захват
                    // пришлось бы закрывать событием End, которого не будет.
                    if !open {
                        return Ok(());
                    }
                    self.capture = Some(Capture {
                        element: if name == b"text" {
                            "text"
                        } else {
                            "currency-symbol"
                        },
                        kind: if name == b"text" {
                            CaptureKind::Text
                        } else {
                            CaptureKind::Currency
                        },
                        text: String::new(),
                    });
                }
                _ => {
                    let attrs = attrs(element, self.part)?;
                    number.push(name, &attrs);
                }
            }
            return Ok(());
        }

        if let Some(style) = self.style.clone() {
            let attrs = attrs(element, self.part)?;
            let raw = match (style.kind, name) {
                (StyleKind::Column, b"table-column-properties") => {
                    find(&attrs, "column-width").and_then(column_width_chars)
                }
                (StyleKind::Row, b"table-row-properties") => {
                    find(&attrs, "row-height").and_then(row_height_points)
                }
                _ => None,
            };
            if let Some(value) = raw {
                match style.kind {
                    StyleKind::Column => {
                        self.book.column_widths.insert(style.name, value);
                    }
                    StyleKind::Row => {
                        self.book.row_heights.insert(style.name, value);
                    }
                }
            }
        }
        Ok(())
    }

    /// Разобрать `style:style`: числовой стиль ячейки или размер строки/столбца.
    fn start_style(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        match find(&attrs, "family") {
            Some("table-cell") => {
                if let (Some(name), Some(data)) =
                    (find(&attrs, "name"), find(&attrs, "data-style-name"))
                {
                    self.book
                        .cell_data_styles
                        .insert(name.to_owned(), data.to_owned());
                }
            }
            Some("table-column") => {
                if let Some(name) = find(&attrs, "name") {
                    self.style = Some(StyleDef {
                        name: name.to_owned(),
                        kind: StyleKind::Column,
                    });
                }
            }
            Some("table-row") => {
                if let Some(name) = find(&attrs, "name") {
                    self.style = Some(StyleDef {
                        name: name.to_owned(),
                        kind: StyleKind::Row,
                    });
                }
            }
            // Остальные семейства (таблица, абзац, текст) — следующий срез.
            _ => {}
        }
        Ok(())
    }

    /// Закрыть элемент: снять захват текста, стиль или числовой формат.
    fn end(&mut self, name: &[u8]) {
        if self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.element.as_bytes() == name)
        {
            if let Some(capture) = self.capture.take() {
                self.apply_capture(&capture);
            }
        }

        if self
            .number
            .as_ref()
            .is_some_and(|number| number.element.as_bytes() == name)
        {
            self.finish_number();
        }

        if name == b"style" {
            self.style = None;
        }
    }

    /// Записать захваченный текст в код формата.
    fn apply_capture(&mut self, capture: &Capture) {
        let Some(number) = self.number.as_mut() else {
            return;
        };
        match capture.kind {
            // В процентном формате `%` — это не литерал, а токен умножения на
            // 100; его дописывает `finish_number`.
            CaptureKind::Text
                if number.kind == NumberKind::Percentage && capture.text.trim() == "%" => {}
            CaptureKind::Text | CaptureKind::Currency => {
                // Переводы строк и табуляция — форматирование XML, а не
                // литерал формата; пробелы значимы и остаются.
                let text = capture
                    .text
                    .trim_matches(|ch: char| matches!(ch, '\n' | '\r' | '\t'));
                push_literal(&mut number.code, text);
            }
        }
    }

    /// Закрыть числовой стиль и запомнить его код.
    fn finish_number(&mut self) {
        let Some(mut number) = self.number.take() else {
            return;
        };
        if number.kind == NumberKind::Percentage {
            number.code.push('%');
        }
        if !number.code.is_empty() {
            self.book.number_codes.insert(number.name, number.code);
        }
    }

    /// Текст текстового узла: он важен только внутри захвата.
    fn text(&mut self, text: &BytesText<'_>) -> Result<()> {
        if self.capture.is_none() {
            return Ok(());
        }
        let decoded = text.decode().map_err(|error| {
            OdfError::malformed(self.part, format!("text is not UTF-8: {error}"))
        })?;
        if let Some(capture) = self.capture.as_mut() {
            capture.text.push_str(&decoded);
        }
        Ok(())
    }

    /// Ссылка на сущность внутри захваченного текста.
    fn reference(&mut self, reference: &BytesRef<'_>) -> Result<()> {
        if self.capture.is_none() {
            return Ok(());
        }
        let decoded = resolve_reference(reference, self.part)?;
        if let Some(capture) = self.capture.as_mut() {
            capture.text.push_str(&decoded);
        }
        Ok(())
    }
}

/// Атрибуты элемента с именем части в ошибке.
fn attrs<'a>(element: &'a BytesStart<'a>, part: &str) -> Result<Vec<Attr<'a>>> {
    let attrs =
        doc_converter_core::xml::attributes(element).map_err(|error| map_core(part, error))?;
    Ok(attrs.into_vec())
}

impl NumberStyle {
    /// Дописать в код разобранный дочерний элемент.
    fn push(&mut self, name: &[u8], attrs: &[Attr<'_>]) {
        match name {
            b"number" => self.push_number(attrs),
            b"scientific-number" => self.push_scientific(attrs),
            b"fraction" => self.code.push_str("# ?/?"),
            b"day" => self.push_letters(if style_is_long(attrs) { "DD" } else { "D" }),
            b"month" => {
                let long = style_is_long(attrs);
                let textual = find(attrs, "textual").is_some_and(is_true);
                self.push_letters(match (textual, long) {
                    (true, true) => "MMMM",
                    (true, false) => "MMM",
                    (false, true) => "MM",
                    (false, false) => "M",
                });
            }
            b"year" => self.push_letters(if style_is_long(attrs) { "YYYY" } else { "YY" }),
            b"day-of-week" => {
                self.push_letters(if style_is_long(attrs) { "DDDD" } else { "DDD" });
            }
            b"week-of-year" => self.push_letters("WW"),
            b"hours" => self.push_letters(if style_is_long(attrs) { "HH" } else { "H" }),
            b"minutes" => self.push_letters(if style_is_long(attrs) { "MM" } else { "M" }),
            b"seconds" => {
                self.push_letters(if style_is_long(attrs) { "SS" } else { "S" });
                if let Some(decimals) = attr_u32(attrs, "decimal-places") {
                    let decimals = decimals.min(MAX_DIGITS);
                    if decimals > 0 {
                        self.code.push('.');
                        self.code
                            .extend(std::iter::repeat_n('0', decimals as usize));
                    }
                }
            }
            b"am-pm" => self.push_letters("AM/PM"),
            // Квартал, эра и встроенный текст Excel-кодом не выражаются;
            // потерять их вид лучше, чем сломать формат целиком.
            _ => {}
        }
    }

    /// Числовой шаблон `number:number`.
    fn push_number(&mut self, attrs: &[Attr<'_>]) {
        let min_integer = attr_u32(attrs, "min-integer-digits")
            .unwrap_or(1)
            .clamp(1, MAX_DIGITS);
        let decimals = attr_u32(attrs, "decimal-places")
            .unwrap_or(0)
            .min(MAX_DIGITS);
        let grouping = find(attrs, "grouping").is_some_and(is_true);

        if grouping {
            self.code.push_str("#,##");
        }
        self.code
            .extend(std::iter::repeat_n('0', min_integer as usize));
        if decimals > 0 {
            self.code.push('.');
            self.code
                .extend(std::iter::repeat_n('0', decimals as usize));
        }
    }

    /// Экспоненциальный шаблон `number:scientific-number`.
    fn push_scientific(&mut self, attrs: &[Attr<'_>]) {
        let min_integer = attr_u32(attrs, "min-integer-digits")
            .unwrap_or(1)
            .clamp(1, MAX_DIGITS);
        let decimals = attr_u32(attrs, "decimal-places")
            .unwrap_or(0)
            .min(MAX_DIGITS);
        let exponent = attr_u32(attrs, "min-exponent-digits")
            .unwrap_or(2)
            .clamp(1, 8);

        self.code
            .extend(std::iter::repeat_n('0', min_integer as usize));
        if decimals > 0 {
            self.code.push('.');
            self.code
                .extend(std::iter::repeat_n('0', decimals as usize));
        }
        self.code.push_str("E+");
        self.code
            .extend(std::iter::repeat_n('0', exponent as usize));
    }

    /// Дописать буквенный токен даты без кавычек: он часть кода, а не литерал.
    fn push_letters(&mut self, token: &str) {
        self.code.push_str(token);
    }
}

/// Числовой стиль по локальному имени элемента.
fn number_style_kind(name: &[u8]) -> Option<(&'static str, NumberKind)> {
    Some(match name {
        b"number-style" => ("number-style", NumberKind::Number),
        b"percentage-style" => ("percentage-style", NumberKind::Percentage),
        b"date-style" => ("date-style", NumberKind::Date),
        b"time-style" => ("time-style", NumberKind::Time),
        b"currency-style" => ("currency-style", NumberKind::Currency),
        // `boolean-style` и `text-style` к ячейкам таблицы не относятся.
        _ => return None,
    })
}

/// Литерал формата: кавычки не дают разобрать буквы как части даты
/// (`text` с «Total» иначе стал бы `T` + день + ...).
fn push_literal(code: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    code.push('"');
    for ch in text.chars() {
        if ch == '"' {
            code.push('"');
        }
        code.push(ch);
    }
    code.push('"');
}

/// `number:style` = `long`; по умолчанию ODF считает стиль коротким.
fn style_is_long(attrs: &[Attr<'_>]) -> bool {
    find(attrs, "style").is_some_and(|value| value == "long")
}

/// Целочисленный атрибут; мусор игнорируется, как и всё в стилях.
fn attr_u32(attrs: &[Attr<'_>], name: &str) -> Option<u32> {
    find(attrs, name)?.trim().parse().ok()
}

/// Длина ODF (`4cm`, `0.8in`, `12pt`) в миллиметрах.
fn length_mm(raw: &str) -> Option<f32> {
    const MM_PER_INCH: f32 = 25.4;
    const PT_PER_INCH: f32 = 72.0;
    const PX_PER_INCH: f32 = 96.0;

    let raw = raw.trim();
    let split = raw
        .find(|ch: char| !(ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+')))
        .unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    let value: f32 = number.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let mm = match unit {
        "mm" => value,
        "cm" => value * 10.0,
        "in" => value * MM_PER_INCH,
        "pt" => value * MM_PER_INCH / PT_PER_INCH,
        "pc" => value * MM_PER_INCH / 6.0,
        "px" => value * MM_PER_INCH / PX_PER_INCH,
        _ => return None,
    };
    Some(mm)
}

/// Ширина столбца ODF → «символы» Excel.
///
/// Пиксели считаются по 96 dpi, символы — по ширине `0` основного шрифта
/// (`MAX_DIGIT_WIDTH`), той же, что у раскладки XLSX: иначе столбец ODS и
/// столбец XLSX одной ширины в сантиметрах занимали бы разное место.
fn column_width_chars(raw: &str) -> Option<f32> {
    const PX_PER_INCH: f32 = 96.0;
    const MM_PER_INCH: f32 = 25.4;

    let px = length_mm(raw)? * PX_PER_INCH / MM_PER_INCH;
    let chars = px / MAX_DIGIT_WIDTH;
    (chars > 0.0).then_some(chars)
}

/// Высота строки ODF → пункты.
fn row_height_points(raw: &str) -> Option<f32> {
    const PT_PER_INCH: f32 = 72.0;
    const MM_PER_INCH: f32 = 25.4;

    let pt = length_mm(raw)? * PT_PER_INCH / MM_PER_INCH;
    (pt > 0.0).then_some(pt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book_with(xml: &str) -> StyleBook {
        let mut book = StyleBook::default();
        collect(xml.as_bytes(), "styles.xml", &mut book).unwrap();
        book
    }

    #[test]
    fn decimal_format_becomes_excel_code() {
        let book = book_with(
            r#"<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
                xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0">
              <number:number-style style:name="N_decimal">
                <number:number number:decimal-places="2" number:min-integer-digits="1"/>
              </number:number-style>
              <style:style style:name="ce_number" style:family="table-cell"
                  style:data-style-name="N_decimal"/>
            </office:document-styles>"#,
        );
        let styles = book.freeze();
        let index = styles.index_of(Some("ce_number"));

        assert_eq!(index, 1);
        assert_eq!(
            styles.table().format_code(FIRST_CUSTOM_FORMAT_ID),
            Some("0.00")
        );
    }

    #[test]
    fn percentage_and_date_formats() {
        let book = book_with(
            r#"<number:percentage-style xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"
                  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" style:name="N_percent">
                 <number:number number:decimal-places="0" number:min-integer-digits="1"/>
                 <number:text>%</number:text>
               </number:percentage-style>
               <number:date-style xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"
                  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" style:name="N_date">
                 <number:day number:style="long"/>
                 <number:text>.</number:text>
                 <number:month number:style="long"/>
                 <number:text>.</number:text>
                 <number:year number:style="long"/>
               </number:date-style>"#,
        );

        assert_eq!(
            book.number_codes.get("N_percent").map(String::as_str),
            Some("0%")
        );
        assert_eq!(
            book.number_codes.get("N_date").map(String::as_str),
            Some("DD\".\"MM\".\"YYYY")
        );
    }

    #[test]
    fn column_and_row_sizes_are_converted() {
        let book = book_with(
            r#"<style:style xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
                  style:name="co1" style:family="table-column">
                 <style:table-column-properties style:column-width="4cm"/>
               </style:style>
               <style:style xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
                  style:name="ro1" style:family="table-row">
                 <style:table-row-properties style:row-height="0.8cm"/>
               </style:style>"#,
        );

        let width = book.column_widths.get("co1").copied().unwrap();
        // 4 см при 96 dpi — 151,2 px; при ширине цифры 7,4 px — ~20,4 символа.
        assert!((width - 20.43).abs() < 0.1, "ширина {width}");

        let height = book.row_heights.get("ro1").copied().unwrap();
        // 0,8 см = 8 мм = 22,68 pt.
        assert!((height - 22.68).abs() < 0.01, "высота {height}");
    }

    #[test]
    fn unknown_number_style_is_skipped() {
        let book = book_with(
            r#"<number:boolean-style xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"
                  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" style:name="N_bool">
                 <number:boolean/>
               </number:boolean-style>"#,
        );

        assert!(book.number_codes.is_empty());
    }
}
