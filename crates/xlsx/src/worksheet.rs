//! Потоковый разбор `xl/worksheets/sheetN.xml`.
//!
//! Лист читается одним проходом: `<sheetData>` → `<row>` → `<c>`. Готовые
//! ячейки уходят в [`WorksheetBuilder`], который проверяет порядок и лимиты
//! Excel, — поэтому битый файл падает громко, а не превращается в кривую модель.
//!
//! Адресация в живых файлах встречается в двух видах: явная
//! (`<row r="5"><c r="C5">`) и позиционная (`<row><c><c>`), где номер строки и
//! столбца выводятся из порядка. Поддерживается и смесь: ячейка без `r`
//! получает столбец, следующий за предыдущей ячейкой строки.

use doc_converter_core::rels::RelMap;
use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use crate::cellref::{CellRef, MAX_ROW};
use crate::dims::{read_col, read_dimension, read_format, read_row, SheetDims};
use crate::error::{Result, XlsxError};
use crate::model::{Cell, CellError, CellValue, SheetContent, WorksheetBuilder};
use crate::sheet_meta::{
    read_hyperlink, read_merge, read_pane, read_view, Hyperlink, Merges, SheetView,
};
use crate::strings::read_item_text;
use crate::xml::{attributes, find, is_true, resolve_reference, Attr};

/// Последняя допустимая строка в 1-based нумерации файла.
const MAX_ROW_ONE_BASED: u32 = MAX_ROW + 1;

/// Тип значения ячейки — атрибут `t` из `<c>` (ECMA-376 `ST_CellType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellType {
    /// `n` и отсутствующий атрибут.
    Number,
    /// `s` — индекс в `sharedStrings.xml`.
    SharedString,
    /// `str` — строковый результат формулы.
    FormulaString,
    /// `inlineStr` — строка в `<is>`.
    Inline,
    /// `b` — логическое значение.
    Bool,
    /// `e` — ошибка листа.
    Error,
    /// `d` — дата ISO 8601 (strict OOXML).
    Date,
}

impl CellType {
    /// Тип из атрибута `t`; незнакомое значение считаем числом, как и отсутствие.
    fn from_attrs(attrs: &[Attr<'_>]) -> Self {
        match find(attrs, "t") {
            Some("s") => Self::SharedString,
            Some("str") => Self::FormulaString,
            Some("inlineStr") => Self::Inline,
            Some("b") => Self::Bool,
            Some("e") => Self::Error,
            Some("d") => Self::Date,
            _ => Self::Number,
        }
    }
}

/// Накопитель одной `<c>`: адрес и тип известны из открывающего тега, значение
/// приходит из `<v>` или `<is>`.
#[derive(Debug)]
struct CellState {
    col: u32,
    style: u32,
    kind: CellType,
    value: CellValue,
    formula: Option<String>,
}

impl CellState {
    fn new(col: u32, style: u32, kind: CellType) -> Self {
        Self {
            col,
            style,
            kind,
            value: CellValue::Empty,
            formula: None,
        }
    }

    fn into_cell(self) -> Cell {
        let mut cell = Cell::new(self.col, self.style, self.value);
        if let Some(formula) = self.formula {
            cell = cell.with_formula(formula);
        }
        cell
    }
}

/// Разобрать лист в модель.
///
/// # Errors
///
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::CellRef`] — адрес
/// ячейки не разбирается или выходит за лимиты Excel; [`XlsxError::Malformed`] —
/// структура нарушает ECMA-376: ячейка вне строки, адрес ячейки противоречит
/// объемлющей строке, номер строки вне `1..=1048576`, файл оборван внутри `<c>`,
/// не разбирается ссылка у `<dimension>`.
pub fn parse(bytes: &[u8], part: impl Into<String>, rels: Option<&RelMap>) -> Result<SheetContent> {
    let mut parser = SheetParser::new(part.into(), rels);
    // Пробелы значимы: внутри `<is>` лежит текст ячейки.
    let mut reader = XmlReader::preserving(bytes, parser.part.clone());

    while let Some(event) = reader.next_significant()? {
        parser.handle(&mut reader, event)?;
    }

    parser.finish()
}

/// Состояние одного прохода по листу.
struct SheetParser<'a> {
    /// Часть пакета: попадает в тексты ошибок и в модель.
    part: String,
    /// Связи листа: по ним разрешаются цели гиперссылок.
    rels: Option<&'a RelMap>,
    builder: WorksheetBuilder,
    /// Объявленная геометрия листа.
    dims: SheetDims,
    /// Вид листа и закреплённые области.
    view: SheetView,
    /// Первый `<sheetView>` уже прочитан.
    view_seen: bool,
    /// Объединённые ячейки.
    merges: Merges,
    /// Гиперссылки.
    hyperlinks: Vec<Hyperlink>,
    /// Вне `<sheetData>` элементов `<row>`/`<c>` не бывает, а похожие имена
    /// внутри `<extLst>` не должны попадать в модель.
    in_sheet_data: bool,
    /// Строка, в которой мы находимся.
    row: Option<u32>,
    /// Номер для следующей строки без атрибута `r`.
    next_row: u32,
    /// Столбец для следующей ячейки без адреса.
    next_col: u32,
    cell: Option<CellState>,
    /// Куда собирается текст: `<v>` и `<f>` — разные приёмники.
    sink: Sink,
    /// Текст текущего `<v>` или `<f>`.
    text: String,
}

/// Приёмник текста внутри `<c>`.
///
/// `<v>` и `<f>` не вкладываются друг в друга, поэтому это одно состояние,
/// а не пара независимых флагов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink {
    /// Мы не внутри `<v>`/`<f>`: текст пропускается.
    Idle,
    /// Внутри `<v>` — значение ячейки.
    Value,
    /// Внутри `<f>` — формула.
    Formula,
}

impl<'r> SheetParser<'r> {
    fn new(part: String, rels: Option<&'r RelMap>) -> Self {
        Self {
            builder: WorksheetBuilder::new(part.clone()),
            part,
            rels,
            dims: SheetDims::default(),
            view: SheetView::default(),
            view_seen: false,
            merges: Merges::default(),
            hyperlinks: Vec::new(),
            in_sheet_data: false,
            row: None,
            next_row: 0,
            next_col: 0,
            cell: None,
            sink: Sink::Idle,
            text: String::new(),
        }
    }

    /// Обработать очередное событие потока.
    fn handle(&mut self, reader: &mut XmlReader<'_>, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.on_start(reader, &element),
            Event::Empty(element) => self.on_empty(&element),
            Event::Text(chunk) if self.sink != Sink::Idle => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| XlsxError::malformed(&self.part, format!("bad text: {e}")))?;
                self.text.push_str(&decoded);
                Ok(())
            }
            // Ссылки на сущности quick-xml отдаёт отдельным событием.
            Event::GeneralRef(reference) if self.sink != Sink::Idle => {
                let decoded = resolve_reference(&reference, &self.part)?;
                self.text.push_str(&decoded);
                Ok(())
            }
            Event::CData(chunk) if self.sink != Sink::Idle => {
                let decoded = chunk
                    .xml10_content()
                    .map_err(|e| XlsxError::malformed(&self.part, format!("bad CDATA: {e}")))?;
                self.text.push_str(&decoded);
                Ok(())
            }
            Event::End(element) => self.on_end(&element),
            _ => Ok(()),
        }
    }

    fn on_start<'a>(
        &mut self,
        reader: &mut XmlReader<'_>,
        element: &'a BytesStart<'a>,
    ) -> Result<()> {
        match element.local_name().as_ref() {
            b"sheetData" => self.in_sheet_data = true,
            b"row" if self.in_sheet_data => {
                let index = self.open_row(element)?;
                self.row = Some(index);
            }
            b"c" if self.in_sheet_data => self.cell = Some(self.open_cell(element)?),
            b"v" if self.cell.is_some() => {
                self.text.clear();
                self.sink = Sink::Value;
            }
            b"f" if self.cell.is_some() => {
                self.text.clear();
                self.sink = Sink::Formula;
            }
            b"is" if self.cell.is_some() => {
                let value = read_item_text(reader, &self.part, "is")?;
                if let Some(state) = self.cell.as_mut() {
                    state.value = CellValue::InlineString(value.into());
                }
            }
            b"dimension" | b"sheetFormatPr" | b"col" => self.read_dims(element)?,
            b"sheetView" | b"pane" | b"mergeCell" | b"hyperlink" => self.read_meta(element)?,
            _ => {}
        }
        Ok(())
    }

    /// Элементы вида листа, закреплений, объединений и гиперссылок.
    fn read_meta<'e>(&mut self, element: &'e BytesStart<'e>) -> Result<()> {
        let attrs = attributes(element, &self.part)?;
        match element.local_name().as_ref() {
            // Видов у книги столько, сколько окон открыто в Excel, а лист
            // один: остальные описывают то же содержимое.
            b"sheetView" if !self.view_seen => {
                self.view_seen = true;
                read_view(&mut self.view, &attrs);
            }
            b"pane" => self.view.pane = Some(read_pane(&attrs, &self.part)?),
            b"mergeCell" => self.merges.push(read_merge(&attrs, &self.part)?),
            b"hyperlink" => self
                .hyperlinks
                .push(read_hyperlink(&attrs, self.rels, &self.part)?),
            _ => {}
        }
        Ok(())
    }

    /// Элементы объявленной геометрии листа: границы, параметры по умолчанию,
    /// ширины столбцов. Все три — пустые элементы в живых файлах, но закрытая
    /// пара тегов тоже законна, поэтому обрабатываются и в `Start`, и в `Empty`.
    fn read_dims<'a>(&mut self, element: &'a BytesStart<'a>) -> Result<()> {
        let attrs = attributes(element, &self.part)?;
        match element.local_name().as_ref() {
            b"dimension" => {
                if let Some(declared) = read_dimension(&attrs, &self.part)? {
                    self.dims.declared = Some(declared);
                }
            }
            b"sheetFormatPr" => self.dims.format = read_format(&attrs),
            b"col" => self.dims.cols.push(read_col(&attrs)),
            _ => {}
        }
        Ok(())
    }

    fn on_empty<'a>(&mut self, element: &'a BytesStart<'a>) -> Result<()> {
        match element.local_name().as_ref() {
            // Строка без ячеек: модели она не нужна, но номер занимает —
            // иначе позиционная нумерация следующих строк сдвинется.
            b"row" if self.in_sheet_data => {
                self.open_row(element)?;
            }
            // Ячейка с одним форматом: значения нет, но формат нужен.
            b"c" if self.in_sheet_data => {
                let row = self.current_row()?;
                let cell = self.open_cell(element)?.into_cell();
                self.builder.push(row, cell)?;
            }
            b"is" if self.cell.is_some() => {
                if let Some(state) = self.cell.as_mut() {
                    state.value = CellValue::InlineString(String::new().into());
                }
            }
            b"dimension" | b"sheetFormatPr" | b"col" => self.read_dims(element)?,
            b"sheetView" | b"pane" | b"mergeCell" | b"hyperlink" => self.read_meta(element)?,
            _ => {}
        }
        Ok(())
    }

    fn on_end<'a>(&mut self, element: &'a BytesEnd<'a>) -> Result<()> {
        match element.local_name().as_ref() {
            b"v" => {
                self.sink = Sink::Idle;
                if let Some(state) = self.cell.as_mut() {
                    state.value = value_of(state.kind, &self.text);
                }
                self.text.clear();
            }
            b"f" => {
                self.sink = Sink::Idle;
                if let Some(state) = self.cell.as_mut() {
                    let formula = self.text.trim();
                    if !formula.is_empty() {
                        // У общих формул (`t="shared"`) текст записан только в
                        // первой ячейке группы, у остальных его нет.
                        state.formula = Some(formula.into());
                    }
                }
                self.text.clear();
            }
            b"c" => {
                if let Some(state) = self.cell.take() {
                    let row = self.current_row()?;
                    self.builder.push(row, state.into_cell())?;
                }
            }
            b"row" => self.row = None,
            b"sheetData" => self.in_sheet_data = false,
            _ => {}
        }
        Ok(())
    }

    /// Открыть строку: номер из `r` либо позиционный.
    fn open_row<'a>(&mut self, element: &'a BytesStart<'a>) -> Result<u32> {
        let attrs = attributes(element, &self.part)?;
        let index = row_index(&attrs, self.next_row, &self.part)?;
        if let Some(height) = read_row(index, &attrs) {
            self.dims.rows.push(height);
        }
        self.next_row = index + 1;
        self.next_col = 0;
        Ok(index)
    }

    /// Начать ячейку: столбец из адреса `r` либо следующий за предыдущей.
    fn open_cell<'a>(&mut self, element: &'a BytesStart<'a>) -> Result<CellState> {
        let row = self.current_row()?;
        let attrs = attributes(element, &self.part)?;
        let col = column(&attrs, self.next_col, row, &self.part)?;
        self.next_col = col + 1;
        Ok(CellState::new(
            col,
            style_index(&attrs),
            CellType::from_attrs(&attrs),
        ))
    }

    /// Строка, в которой находится текущая ячейка.
    fn current_row(&self) -> Result<u32> {
        self.row
            .ok_or_else(|| XlsxError::malformed(&self.part, "<c> outside <row>"))
    }

    /// Закончить лист.
    fn finish(mut self) -> Result<SheetContent> {
        // Файл оборвался внутри ячейки: молча потерять её нельзя.
        if self.cell.is_some() {
            return Err(XlsxError::malformed(
                &self.part,
                "unexpected end of file inside <c>",
            ));
        }
        self.dims.rows.sort();
        Ok(SheetContent {
            cells: self.builder.finish(),
            dims: self.dims,
            view: self.view,
            merges: self.merges,
            hyperlinks: self.hyperlinks,
        })
    }
}

/// Номер строки: из атрибута `r` (в файле он 1-based) либо позиционный.
fn row_index(attrs: &[Attr<'_>], positional: u32, part: &str) -> Result<u32> {
    let Some(raw) = find(attrs, "r") else {
        return Ok(positional);
    };
    let one_based: u32 = raw
        .trim()
        .parse()
        .map_err(|_| XlsxError::malformed(part, format!("row number `{raw}` is not a number")))?;
    if one_based == 0 || one_based > MAX_ROW_ONE_BASED {
        return Err(XlsxError::malformed(
            part,
            format!("row number `{raw}` is outside 1..=1048576"),
        ));
    }
    Ok(one_based - 1)
}

/// Столбец ячейки: из адреса в `r` либо следующий за предыдущей ячейкой строки.
fn column(attrs: &[Attr<'_>], positional: u32, row: u32, part: &str) -> Result<u32> {
    let Some(reference) = find(attrs, "r") else {
        return Ok(positional);
    };
    let cell = CellRef::parse(reference)?;
    if cell.row != row {
        return Err(XlsxError::malformed(
            part,
            format!(
                "cell `{reference}` is declared inside row {}",
                u64::from(row) + 1
            ),
        ));
    }
    Ok(cell.col)
}

/// Индекс формата ячейки; битый индекс — не повод не открыть книгу.
fn style_index(attrs: &[Attr<'_>]) -> u32 {
    find(attrs, "s")
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(0)
}

/// Значение из текста `<v>` по объявленному типу.
///
/// То, что не разбирается — число, индекс общей строки, код ошибки, — остаётся
/// текстом: значение хотя бы видно, и одна битая ячейка не роняет книгу.
fn value_of(kind: CellType, raw: &str) -> CellValue {
    let raw = raw.trim();
    if raw.is_empty() {
        return CellValue::Empty;
    }
    match kind {
        CellType::Number => raw
            .parse::<f64>()
            .map_or_else(|_| as_text(raw), CellValue::Number),
        CellType::SharedString => raw
            .parse::<u32>()
            .map_or_else(|_| as_text(raw), CellValue::SharedString),
        CellType::Bool => {
            if is_true(raw) {
                CellValue::Bool(true)
            } else if matches!(raw, "0" | "false") {
                CellValue::Bool(false)
            } else {
                as_text(raw)
            }
        }
        CellType::Error => CellError::parse(raw).map_or_else(|| as_text(raw), CellValue::Error),
        // `str` — строковый результат формулы, `d` — дата ISO 8601 (strict
        // OOXML), `inlineStr` — значение приходит из `<is>`. Все три пока
        // остаются текстом: serial-даты разберёт `numfmt` (шаг 8).
        CellType::FormulaString | CellType::Date | CellType::Inline => as_text(raw),
    }
}

/// Текстовое значение.
fn as_text(raw: &str) -> CellValue {
    CellValue::InlineString(raw.into())
}

#[cfg(test)]
// Ширины и высоты в тестах — точные литералы, представимые в `f32`,
// поэтому сравнение на равенство здесь осмысленно.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::model::Worksheet;

    const PART: &str = "xl/worksheets/sheet1.xml";

    fn content(body: &str) -> SheetContent {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
               <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#
        );
        parse(xml.as_bytes(), PART, None).unwrap()
    }

    /// Только ячейки — большинству тестов геометрия не нужна.
    fn sheet(body: &str) -> Worksheet {
        content(body).cells
    }

    fn fails(body: &str) -> XlsxError {
        let xml = format!(r"<worksheet>{body}</worksheet>");
        parse(xml.as_bytes(), PART, None).unwrap_err()
    }

    /// Разбор заведомо обрезанного документа: закрывающие теги не достраиваются.
    fn fails_raw(xml: &str) -> XlsxError {
        parse(xml.as_bytes(), PART, None).unwrap_err()
    }

    fn value(sheet: &Worksheet, row: u32, col: u32) -> CellValue {
        sheet.cell(CellRef::new(row, col)).unwrap().value.clone()
    }

    #[test]
    fn reads_explicit_addresses() {
        let ws = sheet(
            r#"<sheetData>
                 <row r="1"><c r="A1" s="1"><v>1.5</v></c><c r="C1"><v>3</v></c></row>
                 <row r="3"><c r="B3"><v>7</v></c></row>
               </sheetData>"#,
        );

        assert_eq!(ws.cell_count(), 3);
        assert_eq!(ws.row_count(), 2);
        assert_eq!(ws.last_row(), Some(2));

        assert_eq!(value(&ws, 0, 0), CellValue::Number(1.5));
        assert_eq!(ws.cell(CellRef::new(0, 0)).unwrap().style, 1);
        assert_eq!(ws.cell(CellRef::new(0, 1)), None);
        assert_eq!(value(&ws, 0, 2), CellValue::Number(3.0));
        assert_eq!(value(&ws, 2, 1), CellValue::Number(7.0));
    }

    #[test]
    fn reads_positional_addresses() {
        let ws = sheet(
            r"<sheetData>
                 <row><c><v>1</v></c><c><v>2</v></c></row>
                 <row><c><v>3</v></c></row>
               </sheetData>",
        );

        assert_eq!(ws.cell_count(), 3);
        assert_eq!(value(&ws, 0, 0), CellValue::Number(1.0));
        assert_eq!(value(&ws, 0, 1), CellValue::Number(2.0));
        assert_eq!(value(&ws, 1, 0), CellValue::Number(3.0));
        assert_eq!(ws.cell(CellRef::new(0, 2)), None);
    }

    #[test]
    fn mixes_both_address_modes() {
        let ws = sheet(
            r#"<sheetData>
                 <row r="2"><c r="C2"><v>1</v></c><c><v>2</v></c></row>
                 <row><c r="A3"><v>3</v></c></row>
               </sheetData>"#,
        );

        // Ячейка без `r` продолжает строку за явной (C → D).
        assert_eq!(value(&ws, 1, 2), CellValue::Number(1.0));
        assert_eq!(value(&ws, 1, 3), CellValue::Number(2.0));
        // Строка без `r` берёт номер из адреса своей ячейки.
        assert_eq!(value(&ws, 2, 0), CellValue::Number(3.0));
    }

    #[test]
    fn reads_every_cell_type() {
        let ws = sheet(
            r#"<sheetData>
                 <row r="1">
                   <c r="A1" t="s"><v>2</v></c>
                   <c r="B1" t="str"><v>итог</v></c>
                   <c r="C1" t="b"><v>1</v></c>
                   <c r="D1" t="b"><v>0</v></c>
                   <c r="E1" t="e"><v>#DIV/0!</v></c>
                   <c r="F1" t="d"><v>2024-01-31T00:00:00</v></c>
                   <c r="G1" t="inlineStr"><is><t>строка</t></is></c>
                   <c r="H1"><v>42</v></c>
                 </row>
               </sheetData>"#,
        );

        assert_eq!(value(&ws, 0, 0), CellValue::SharedString(2));
        assert_eq!(value(&ws, 0, 1), CellValue::InlineString("итог".into()));
        assert_eq!(value(&ws, 0, 2), CellValue::Bool(true));
        assert_eq!(value(&ws, 0, 3), CellValue::Bool(false));
        assert_eq!(value(&ws, 0, 4), CellValue::Error(CellError::Div0));
        assert_eq!(
            value(&ws, 0, 5),
            CellValue::InlineString("2024-01-31T00:00:00".into())
        );
        assert_eq!(value(&ws, 0, 6), CellValue::InlineString("строка".into()));
        assert_eq!(value(&ws, 0, 7), CellValue::Number(42.0));
    }

    #[test]
    fn inline_string_keeps_runs_and_drops_phonetics() {
        let ws = sheet(
            r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is>
                 <r><t>Hello </t></r><r><t xml:space="preserve">world </t></r>
                 <rPh sb="0" eb="5"><t>へろー</t></rPh>
               </is></c></row></sheetData>"#,
        );

        assert_eq!(
            value(&ws, 0, 0),
            CellValue::InlineString("Hello world ".into())
        );
    }

    #[test]
    fn keeps_formula_next_to_cached_value() {
        let ws = sheet(
            r#"<sheetData><row r="1">
                 <c r="A1"><f>SUM(B1:B2)</f><v>3</v></c>
                 <c r="B1"><f>NOW()</f></c>
               </row></sheetData>"#,
        );

        let with_value = ws.cell(CellRef::new(0, 0)).unwrap();
        assert_eq!(with_value.formula.as_deref(), Some("SUM(B1:B2)"));
        assert_eq!(with_value.value, CellValue::Number(3.0));

        // Формула без кэша: значения нет, текст формулы есть.
        let without_value = ws.cell(CellRef::new(0, 1)).unwrap();
        assert_eq!(without_value.formula.as_deref(), Some("NOW()"));
        assert_eq!(without_value.value, CellValue::Empty);
    }

    #[test]
    fn styled_cells_without_values_are_kept() {
        let ws = sheet(
            r#"<sheetData><row r="1">
                 <c r="A1" s="4"/>
                 <c r="B1" s="5"><v/></c>
               </row></sheetData>"#,
        );

        assert_eq!(ws.cell_count(), 2);
        let empty = ws.cell(CellRef::new(0, 0)).unwrap();
        assert_eq!(empty.style, 4);
        assert_eq!(empty.value, CellValue::Empty);
        assert_eq!(value(&ws, 0, 1), CellValue::Empty);
    }

    #[test]
    fn broken_values_degrade_to_text() {
        let ws = sheet(
            r#"<sheetData><row r="1">
                 <c r="A1"><v>не число</v></c>
                 <c r="B1" t="s"><v>x</v></c>
                 <c r="C1" t="e"><v>#ЧТО-ТО</v></c>
                 <c r="D1" t="b"><v>может быть</v></c>
               </row></sheetData>"#,
        );

        assert_eq!(value(&ws, 0, 0), CellValue::InlineString("не число".into()));
        assert_eq!(value(&ws, 0, 1), CellValue::InlineString("x".into()));
        assert_eq!(value(&ws, 0, 2), CellValue::InlineString("#ЧТО-ТО".into()));
        assert_eq!(
            value(&ws, 0, 3),
            CellValue::InlineString("может быть".into())
        );
    }

    #[test]
    fn numbers_are_trimmed() {
        let ws = sheet(
            r#"<sheetData><row r="1"><c r="A1"><v>
                 2.5
               </v></c></row></sheetData>"#,
        );

        assert_eq!(value(&ws, 0, 0), CellValue::Number(2.5));
    }

    #[test]
    fn cells_do_not_leak_from_neighbouring_elements() {
        let ws = sheet(
            r#"<sheetPr><tabColor rgb="FFFF0000"/></sheetPr>
               <dimension ref="A1:B2"/>
               <sheetViews><sheetView tabSelected="1" workbookViewId="0">
                 <pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/>
               </sheetView></sheetViews>
               <sheetFormatPr defaultRowHeight="15"/>
               <cols><col min="1" max="2" width="12.5" customWidth="1"/></cols>
               <sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
               <mergeCells count="1"><mergeCell ref="A1:B2"/></mergeCells>
               <hyperlinks><hyperlink ref="A1" r:id="rId1"/></hyperlinks>"#,
        );

        assert_eq!(ws.cell_count(), 1);
        assert_eq!(value(&ws, 0, 0), CellValue::Number(1.0));
    }

    #[test]
    fn reads_declared_geometry() {
        let content = content(
            r#"<dimension ref="A1:D10"/>
               <sheetFormatPr defaultRowHeight="12.75" defaultColWidth="9"/>
               <cols>
                 <col min="1" max="1" width="4.5" customWidth="1"/>
                 <col min="3" max="4" width="20"/>
               </cols>
               <sheetData>
                 <row r="1" ht="30" customHeight="1"><c r="A1"><v>1</v></c></row>
                 <row r="2" hidden="1"><c r="A2"><v>2</v></c></row>
                 <row r="3"><c r="A3"><v>3</v></c></row>
               </sheetData>"#,
        );
        let dims = &content.dims;

        assert_eq!(
            dims.declared,
            Some(crate::cellref::Range {
                first: CellRef::new(0, 0),
                last: CellRef::new(9, 3),
            })
        );
        assert_eq!(dims.format.default_row_height, 12.75);
        assert_eq!(dims.col_width(0), 4.5);
        // Столбец без записи берёт общую ширину.
        assert_eq!(dims.col_width(1), 9.0);
        assert_eq!(dims.col_width(2), 20.0);
        assert_eq!(dims.row_height(0), 30.0);
        assert_eq!(dims.row_height(1), 0.0);
        assert_eq!(dims.row_height(2), 12.75);

        // Фактические границы считаются по ячейкам, а не по `<dimension>`.
        let used = content.cells.used_range().unwrap();
        assert_eq!(used.first, CellRef::new(0, 0));
        assert_eq!(used.last, CellRef::new(2, 0));
    }

    #[test]
    fn geometry_elements_may_be_paired_tags() {
        let content = content(
            r#"<dimension ref="B2:C3"></dimension>
               <sheetFormatPr defaultRowHeight="20"></sheetFormatPr>
               <cols><col min="2" max="2" width="15" hidden="1"></col></cols>
               <sheetData/>"#,
        );

        assert_eq!(content.dims.col_width(1), 0.0);
        assert_eq!(content.dims.format.default_row_height, 20.0);
        assert!(content.dims.declared.is_some());
    }

    #[test]
    fn empty_row_keeps_positional_numbering() {
        let ws = sheet(
            r#"<sheetData>
                 <row r="1"><c r="A1"><v>1</v></c></row>
                 <row r="2"/>
                 <row><c><v>3</v></c></row>
               </sheetData>"#,
        );

        // Позиционная строка идёт третьей, а не второй.
        assert_eq!(value(&ws, 2, 0), CellValue::Number(3.0));
        assert_eq!(ws.row_count(), 2);
    }

    #[test]
    fn cell_outside_row_is_malformed() {
        let err = fails(r#"<sheetData><c r="A1"><v>1</v></c></sheetData>"#);

        assert!(matches!(err, XlsxError::Malformed { .. }));
        assert!(err.to_string().contains("outside <row>"));
    }

    #[test]
    fn cell_address_must_match_its_row() {
        let err = fails(r#"<sheetData><row r="5"><c r="A7"><v>1</v></c></row></sheetData>"#);

        assert!(matches!(err, XlsxError::Malformed { .. }));
        assert!(err.to_string().contains("A7"));
        assert!(err.to_string().contains("row 5"));
    }

    #[test]
    fn out_of_range_cell_reference_is_a_cellref_error() {
        let err = fails(r#"<sheetData><row r="1"><c r="XFE1"><v>1</v></c></row></sheetData>"#);

        assert!(matches!(err, XlsxError::CellRef(_)));
    }

    #[test]
    fn broken_row_numbers_are_malformed() {
        for bad in ["0", "1048577", "первая"] {
            let err = fails(&format!(
                r#"<sheetData><row r="{bad}"><c><v>1</v></c></row></sheetData>"#
            ));
            assert!(matches!(err, XlsxError::Malformed { .. }), "{bad}");
        }
    }

    #[test]
    fn rows_must_go_in_order() {
        let err = fails(
            r#"<sheetData>
                 <row r="5"><c r="A5"><v>1</v></c></row>
                 <row r="2"><c r="A2"><v>2</v></c></row>
               </sheetData>"#,
        );

        assert!(err.to_string().contains("ascending order"));
    }

    #[test]
    fn truncated_cell_is_malformed() {
        // Тег дочитан, а элемент — нет: quick-xml доходит до конца файла молча,
        // и незакрытую ячейку ловит уже парсер листа.
        let err = fails_raw(r#"<worksheet><sheetData><row r="1"><c r="B1"><v>2</v>"#);

        assert!(matches!(err, XlsxError::Malformed { .. }), "{err}");
        assert!(err.to_string().contains("unexpected end of file"));

        // Обрыв внутри самого тега видит quick-xml.
        let err = fails_raw(r#"<worksheet><sheetData><row r="1"><c r="B1""#);

        assert!(matches!(err, XlsxError::Core(_)), "{err}");
    }

    #[test]
    fn empty_sheet_data_gives_an_empty_worksheet() {
        let ws = sheet("<sheetData/>");

        assert_eq!(ws.cell_count(), 0);
        assert_eq!(ws.last_row(), None);
    }
}
