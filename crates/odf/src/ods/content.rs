//! `content.xml`: тело электронной таблицы в листы модели `xlsx`.
//!
//! Разбор потоковый и однопроходный: `table:table` открывает лист,
//! `table:table-row` — строку, `table:table-cell` — ячейку. Значения копятся в
//! буфере ячейки и попадают в [`WorksheetBuilder`] только на её закрытии.
//!
//! # Повторы
//!
//! ODF экономит место: одна строка может объявить
//! `table:number-rows-repeated="1048576"`, а столбец — тысячи повторов. Строку
//! без содержимого разворачивать не нужно вовсе — `LibreOffice` так пишет хвост
//! листа; у строки с содержимым повтор ограничен лимитом Excel
//! (`MAX_ROW`/`MAX_COL`), а общее число ячеек — [`MAX_CELLS_PER_SHEET`]. Оба
//! предела проверяются **до** разворачивания, поэтому гигантский повтор даёт
//! [`OdfError::Limit`], а не исчерпание памяти.
//!
//! # Значения
//!
//! `office:value-type` определяет значение, но текст абзацев сохраняется в
//! любом случае: если число/дата не разобрались, ячейка остаётся строкой, а не
//! пустой. `date` и `time` переводятся в серийный номер Excel (система 1900 с
//! «фантомным» 29.02.1900); видимый вид задаёт числовой формат из стиля
//! (см. [`super::styles`]). Дата раньше 01.01.1900 серийного номера не имеет и
//! остаётся строкой.

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_xlsx::cellref::{MAX_COL, MAX_ROW};
use doc_converter_xlsx::{
    Cell, CellRef, CellValue, ColWidth, Merges, Range, RowHeight, Sheet, SheetContent, SheetDims,
    SheetState, WorksheetBuilder, WorksheetMeta, XlsxError,
};
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};

use crate::error::{OdfError, Result};
use crate::xml::{find, map_core, resolve_reference};

use super::styles::CellStyles;

/// Предел числа непустых ячеек одного листа.
///
/// Excel адресует 1 048 576 строк, и книга такого размера уже на грани памяти
/// воркера; повтор, размноженный сверх этого, — почти наверняка испорченный или
/// враждебный файл. Предел проверяется по уже развёрнутым ячейкам.
pub const MAX_CELLS_PER_SHEET: usize = 1_048_576;

/// Разобрать `content.xml` в листы книги.
///
/// Имя листа — `table:name`; лист без имени получает `SheetN` (так файл
/// остаётся открываемым, а не отвергается). Все листы ссылаются на одну часть
/// [`CONTENT_PART`](super::CONTENT_PART): в ODF лист — не отдельная часть
/// пакета.
///
/// # Errors
/// [`OdfError::Core`] — XML не разбирается; [`OdfError::Malformed`] — ячейка
/// стоит вне строки, объединение выходит за лист или ячейка нарушает модель
/// `xlsx`; [`OdfError::Limit`] — повтор строк/ячеек вышел за лимиты Excel или
/// предел числа ячеек.
pub(crate) fn parse(bytes: &[u8], part: &str, styles: &CellStyles) -> Result<Vec<Sheet>> {
    // Пробелы в тексте ODF значимы (`text:s`, `xml:space`), поэтому читатель
    // не обрезает текстовые узлы.
    let mut reader = XmlReader::preserving(bytes, part.to_owned());
    let mut parser = Parser {
        part,
        styles,
        sheets: Vec::new(),
        table: None,
        nested: 0,
        cell: None,
    };

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }
    parser.finish()
}

/// Ячейка, которую ещё нечем отдать сборщику: она не закрыта.
#[derive(Debug, Default)]
struct PendingCell {
    /// `table:number-columns-repeated`.
    repeat: u32,
    /// Это `table:covered-table-cell`: значения и объединения у неё нет.
    covered: bool,
    /// `table:style-name`.
    style_name: Option<String>,
    /// `office:value-type`.
    value_type: Option<String>,
    /// `office:value` (число, проценты, валюта).
    value: Option<String>,
    /// `office:date-value`.
    date: Option<String>,
    /// `office:time-value`.
    time: Option<String>,
    /// `office:boolean-value`.
    boolean: Option<String>,
    /// `table:formula`.
    formula: Option<String>,
    /// `table:number-columns-spanned`, минимум 1.
    span_columns: u32,
    /// `table:number-rows-spanned`, минимум 1.
    span_rows: u32,
    /// Абзацы `text:p`/`text:h` в порядке файла.
    paragraphs: Vec<String>,
    /// Текст идёт внутри абзаца, а не между ними.
    in_paragraph: bool,
}

impl PendingCell {
    /// Собрать ячейку из атрибутов открывающего тега.
    fn from_attrs(covered: bool, attrs: &[Attr<'_>]) -> Self {
        if covered {
            return Self {
                repeat: repeat(attrs, "number-columns-repeated"),
                covered,
                ..Self::default()
            };
        }
        Self {
            repeat: repeat(attrs, "number-columns-repeated"),
            covered: false,
            style_name: owned(attrs, "style-name"),
            value_type: owned(attrs, "value-type"),
            value: owned(attrs, "value"),
            date: owned(attrs, "date-value"),
            time: owned(attrs, "time-value"),
            boolean: owned(attrs, "boolean-value"),
            formula: owned(attrs, "formula"),
            span_columns: repeat(attrs, "number-columns-spanned"),
            span_rows: repeat(attrs, "number-rows-spanned"),
            paragraphs: Vec::new(),
            in_paragraph: false,
        }
    }

    /// Значение ячейки; `text` — склеенные абзацы.
    fn to_value(&self, text: &str) -> CellValue {
        let Some(value_type) = self.value_type.as_deref() else {
            return text_value(text);
        };
        match value_type {
            // Проценты ODF хранит долей (0,25 = 25 %), как и Excel: значение
            // переносится без пересчёта, вид задаёт формат.
            "float" | "currency" | "percentage" => self
                .number()
                .map_or_else(|| text_value(text), CellValue::Number),
            "boolean" => self
                .boolean_value()
                .or_else(|| parse_bool(text))
                .map_or_else(|| text_value(text), CellValue::Bool),
            "date" => self
                .date
                .as_deref()
                .and_then(parse_date)
                .or_else(|| parse_date(text))
                .map_or_else(|| text_value(text), CellValue::Number),
            "time" => self
                .time
                .as_deref()
                .and_then(parse_time)
                .or_else(|| clock_seconds(text))
                .map_or_else(|| text_value(text), CellValue::Number),
            // Незнакомый тип не повод терять текст: показываем его строкой.
            _ => text_value(text),
        }
    }

    /// Число из `office:value`; `NaN` и бесконечность отбрасываются.
    fn number(&self) -> Option<f64> {
        let value: f64 = self.value.as_deref()?.trim().parse().ok()?;
        value.is_finite().then_some(value)
    }

    /// Логическое значение из `office:boolean-value`.
    fn boolean_value(&self) -> Option<bool> {
        self.boolean.as_deref().and_then(parse_bool)
    }

    /// Собрать [`Cell`] и размер объединения; `None` — пустая ячейка без
    /// формулы, её в модели не нужно представлять вовсе.
    fn into_cell(self, col: u32, styles: &CellStyles) -> Option<(Cell, (u32, u32))> {
        let text = self.paragraphs.join("\n");
        let value = self.to_value(&text);
        let style = styles.index_of(self.style_name.as_deref());
        let formula = self
            .formula
            .as_deref()
            .map(normalize_formula)
            .filter(|formula| !formula.is_empty());

        // Пустую ячейку без формулы в модели не представляем — кроме якоря
        // объединения: сам диапазон это данные, и потерять его нельзя.
        let span = (self.span_columns.max(1), self.span_rows.max(1));
        if matches!(&value, CellValue::Empty) && formula.is_none() && span == (1, 1) {
            return None;
        }

        let mut cell = Cell::new(col, style, value);
        if let Some(formula) = formula {
            cell = cell.with_formula(formula);
        }
        Some((cell, span))
    }
}

/// Развёрнутая ячейка строки, ещё не отданная сборщику.
#[derive(Debug, Clone)]
struct RowCell {
    /// 0-based столбец.
    col: u32,
    /// Значение и формат.
    cell: Cell,
    /// `(столбцы, строки)` объединения, минимум 1×1.
    span: (u32, u32),
}

/// Строка, которую ещё не отдали сборщику: нужен признак «есть содержимое»,
/// чтобы не разворачивать пустые повторы.
#[derive(Debug)]
struct PendingRow {
    /// `table:number-rows-repeated`.
    repeat: u32,
    /// Высота из стиля строки, пункты.
    height: Option<f32>,
    /// Ячейки по возрастанию столбца.
    cells: Vec<RowCell>,
    /// Следующий свободный столбец.
    column: u32,
}

/// Открытый лист.
#[derive(Debug)]
struct Table {
    name: String,
    /// Номер следующей строки.
    row: u32,
    /// Номер следующего столбца для `table:table-column`.
    column: u32,
    builder: WorksheetBuilder,
    dims: SheetDims,
    merges: Merges,
    current: Option<PendingRow>,
    /// Сколько ячеек уже отдано сборщику — против размножения повторов.
    pushed: usize,
}

impl Table {
    fn new(name: String, part: &str) -> Self {
        Self {
            name,
            row: 0,
            column: 0,
            builder: WorksheetBuilder::new(part.to_owned()),
            dims: SheetDims::default(),
            merges: Merges::default(),
            current: None,
            pushed: 0,
        }
    }
}

/// Состояние разбора `content.xml`.
struct Parser<'a> {
    part: &'a str,
    styles: &'a CellStyles,
    sheets: Vec<Sheet>,
    table: Option<Table>,
    /// Глубина вложенных `table:table`: их содержимое модель XLSX не выражает.
    nested: u32,
    cell: Option<PendingCell>,
}

impl Parser<'_> {
    fn handle(&mut self, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.start(&element, true),
            Event::Empty(element) => self.start(&element, false),
            Event::End(element) => {
                self.end(element.local_name().as_ref())?;
                Ok(())
            }
            Event::Text(text) => self.text(&text),
            Event::GeneralRef(reference) => self.reference(&reference),
            _ => Ok(()),
        }
    }

    /// Открыть элемент; `open` ложно для самозакрытого (`<.../>`).
    fn start(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let name = element.local_name();
        let name = name.as_ref();

        // Вложенная таблица: считаем глубину и не трогаем ни ячейки, ни текст.
        if self.nested > 0 {
            if name == b"table" && open {
                self.nested += 1;
            }
            return Ok(());
        }

        match name {
            b"table" => {
                if self.table.is_some() {
                    // `table:table` внутри ячейки — вложенная таблица.
                    if open {
                        self.nested = 1;
                    }
                    return Ok(());
                }
                let attrs = self.attrs(element)?;
                let name = owned(&attrs, "name")
                    .unwrap_or_else(|| format!("Sheet{}", self.sheets.len() + 1));
                self.table = Some(Table::new(name, self.part));
                // `<table:table/>` — законный пустой лист; закрывать его
                // событием End никто не будет.
                if !open {
                    self.finish_table()?;
                }
                Ok(())
            }
            b"table-column" => self.table_column(element),
            b"table-row" => self.table_row(element, open),
            b"table-cell" | b"covered-table-cell" => self.table_cell(element, open),
            b"p" | b"h" => {
                if let Some(cell) = self.cell.as_mut() {
                    cell.paragraphs.push(String::new());
                    // У самозакрытого абзаца нет события End, и «внутри
                    // абзаца» выставлять нельзя — иначе текст после него
                    // приклеится к пустой строке.
                    cell.in_paragraph = open;
                }
                Ok(())
            }
            b"s" => {
                let attrs = self.attrs(element)?;
                let count = find(&attrs, "c")
                    .and_then(|raw| raw.trim().parse::<u32>().ok())
                    .unwrap_or(1)
                    .min(MAX_COL + 1);
                self.push_text(&" ".repeat(count as usize));
                Ok(())
            }
            b"tab" => {
                self.push_text("\t");
                Ok(())
            }
            b"line-break" => {
                self.push_text("\n");
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// `table:table-column`: ширина столбца и её повтор.
    fn table_column(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(table) = self.table.as_mut() else {
            return Ok(());
        };
        let attrs = attrs(element, self.part)?;
        let repeat = repeat(&attrs, "number-columns-repeated");
        let width = self
            .styles
            .column_width(owned(&attrs, "style-name").as_deref());

        let first = table.column;
        let end = u64::from(first) + u64::from(repeat);
        if width.is_some() && end > u64::from(MAX_COL) + 1 {
            return Err(OdfError::Limit(format!(
                "a column style repeated {repeat} times at column {} goes beyond the 16384-column limit",
                u64::from(first) + 1
            )));
        }
        if let Some(width) = width {
            table.dims.cols.push(ColWidth {
                first,
                last: u32::try_from(end).unwrap_or(MAX_COL + 1) - 1,
                width,
                custom: true,
                hidden: false,
                best_fit: false,
            });
        }
        table.column = u32::try_from(end).unwrap_or(MAX_COL + 1);
        Ok(())
    }

    /// `table:table-row`: открыть строку или закрыть самозакрытую.
    fn table_row(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.table.is_none() {
            return Ok(());
        }
        self.finish_row()?;
        let attrs = self.attrs(element)?;
        let repeat = repeat(&attrs, "number-rows-repeated");
        let height = self
            .styles
            .row_height(owned(&attrs, "style-name").as_deref());
        if let Some(table) = self.table.as_mut() {
            table.current = Some(PendingRow {
                repeat,
                height,
                cells: Vec::new(),
                column: 0,
            });
        }
        if !open {
            self.finish_row()?;
        }
        Ok(())
    }

    /// `table:table-cell` / `table:covered-table-cell`.
    fn table_cell(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.table.is_none() {
            return Ok(());
        }
        // Предыдущая ячейка обязана была закрыться; незакрытую закрываем,
        // чтобы не потерять её значение.
        self.finish_cell()?;
        let attrs = self.attrs(element)?;
        let covered = element.local_name().as_ref() == b"covered-table-cell";
        self.cell = Some(PendingCell::from_attrs(covered, &attrs));
        if !open {
            self.finish_cell()?;
        }
        Ok(())
    }

    /// Закрыть элемент.
    fn end(&mut self, name: &[u8]) -> Result<()> {
        if self.nested > 0 {
            if name == b"table" {
                self.nested -= 1;
            }
            return Ok(());
        }
        match name {
            b"table" => self.finish_table(),
            b"table-row" => self.finish_row(),
            b"table-cell" | b"covered-table-cell" => self.finish_cell(),
            b"p" | b"h" => {
                if let Some(cell) = self.cell.as_mut() {
                    cell.in_paragraph = false;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Закрыть ячейку: развернуть повтор, проверить лимиты и положить в строку.
    fn finish_cell(&mut self) -> Result<()> {
        let Some(pending) = self.cell.take() else {
            return Ok(());
        };
        let Some(table) = self.table.as_mut() else {
            return Ok(());
        };
        let Some(row) = table.current.as_mut() else {
            return Err(OdfError::malformed(
                self.part,
                "a table cell is outside a table row",
            ));
        };

        let first = row.column;
        let repeat = pending.repeat.max(1);
        let end = u64::from(first) + u64::from(repeat);
        if end > u64::from(MAX_COL) + 1 {
            return Err(OdfError::Limit(format!(
                "a cell repeated {repeat} times at column {} goes beyond the 16384-column limit",
                u64::from(first) + 1
            )));
        }
        row.column = u32::try_from(end).unwrap_or(MAX_COL + 1);

        // Покрытая ячейка — дырка в чужом объединении: столбец она занимает,
        // но значения и собственного объединения у неё нет.
        if pending.covered {
            return Ok(());
        }

        let last_col = u64::from(first) + u64::from(pending.span_columns.max(1));
        let last_row = u64::from(table.row) + u64::from(pending.span_rows.max(1));
        if last_col > u64::from(MAX_COL) + 1 || last_row > u64::from(MAX_ROW) + 1 {
            return Err(OdfError::malformed(
                self.part,
                format!(
                    "a merged range from column {} and row {} goes beyond the Excel grid",
                    u64::from(first) + 1,
                    u64::from(table.row) + 1
                ),
            ));
        }

        let Some((cell, span)) = pending.into_cell(first, self.styles) else {
            return Ok(());
        };
        for offset in 0..repeat {
            let col = first + offset;
            let mut repeated = cell.clone();
            repeated.col = col;
            row.cells.push(RowCell {
                col,
                cell: repeated,
                span,
            });
        }
        Ok(())
    }

    /// Закрыть строку: пустой повтор не разворачивается, непустой — да.
    fn finish_row(&mut self) -> Result<()> {
        let Some(table) = self.table.as_mut() else {
            return Ok(());
        };
        let Some(row) = table.current.take() else {
            return Ok(());
        };
        let part = self.part;
        let repeat = row.repeat.max(1);
        let base = table.row;

        // Хвост листа LibreOffice пишет одной строкой с повтором в миллион:
        // разворачивать пустоту незачем, достаточно сдвинуть номер строки.
        // Высота пустой строки при этом теряется — геометрия пустых строк
        // вместе с остальными свойствами стиля будет в следующем срезе.
        if row.cells.is_empty() {
            table.row = table.row.saturating_add(repeat);
            return Ok(());
        }

        let end = u64::from(base) + u64::from(repeat);
        if end > u64::from(MAX_ROW) + 1 {
            return Err(OdfError::Limit(format!(
                "a row repeated {repeat} times at row {} goes beyond the 1048576-row limit",
                u64::from(base) + 1
            )));
        }
        let added = u64::try_from(row.cells.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(repeat));
        let total = u64::try_from(table.pushed)
            .unwrap_or(u64::MAX)
            .saturating_add(added);
        if total > u64::try_from(MAX_CELLS_PER_SHEET).unwrap_or(u64::MAX) {
            return Err(OdfError::Limit(format!(
                "a sheet would hold more than {MAX_CELLS_PER_SHEET} cells"
            )));
        }

        for offset in 0..repeat {
            let row_index = base + offset;
            if let Some(height) = row.height {
                table.dims.rows.push(RowHeight {
                    row: row_index,
                    height,
                    custom: true,
                    hidden: false,
                    outline_level: 0,
                });
            }
            for item in &row.cells {
                table
                    .builder
                    .push(row_index, item.cell.clone())
                    .map_err(|error| builder_error(part, error))?;
                if item.span.0 > 1 || item.span.1 > 1 {
                    table.merges.push(Range {
                        first: CellRef::new(row_index, item.col),
                        last: CellRef::new(
                            (row_index + item.span.1 - 1).min(MAX_ROW),
                            (item.col + item.span.0 - 1).min(MAX_COL),
                        ),
                    });
                }
            }
            table.pushed += row.cells.len();
        }
        table.row = u32::try_from(end).unwrap_or(MAX_ROW + 1);
        Ok(())
    }

    /// Закрыть лист и положить его в книгу.
    fn finish_table(&mut self) -> Result<()> {
        self.finish_cell()?;
        self.finish_row()?;
        let Some(table) = self.table.take() else {
            return Ok(());
        };

        let mut dims = table.dims;
        // `<row>` идут по возрастанию, но строка без ячеек порядок не
        // проверяет — бинарный поиск высот этого не прощает.
        dims.rows.sort();

        // Печать ODF (`style:page-layout`) — следующий срез: размер страницы и
        // поля выбирает стиль таблицы, а не лист, и связывать их надо вместе
        // с остальными свойствами таблицы.
        let content = SheetContent {
            cells: table.builder.finish(),
            dims,
            merges: table.merges,
            ..SheetContent::default()
        };
        self.sheets.push(Sheet::new(
            WorksheetMeta {
                name: table.name,
                part: self.part.to_owned(),
                // Видимость в ODF задаёт `table:display` стиля таблицы; слой
                // стилей — следующий срез, поэтому пока все листы видимы.
                state: SheetState::Visible,
            },
            content,
        ));
        Ok(())
    }

    /// Конец части: незакрытая таблица — битый XML.
    fn finish(self) -> Result<Vec<Sheet>> {
        if self.table.is_some() || self.nested > 0 {
            return Err(OdfError::malformed(
                self.part,
                "the part ends inside a table",
            ));
        }
        Ok(self.sheets)
    }

    /// Текст текстового узла — только внутри абзаца ячейки.
    fn text(&mut self, text: &BytesText<'_>) -> Result<()> {
        if self.cell.is_none() {
            return Ok(());
        }
        let decoded = text.decode().map_err(|error| {
            OdfError::malformed(self.part, format!("text is not UTF-8: {error}"))
        })?;
        self.push_text(&decoded);
        Ok(())
    }

    /// Ссылка на сущность внутри текста ячейки.
    fn reference(&mut self, reference: &BytesRef<'_>) -> Result<()> {
        if self.cell.is_none() {
            return Ok(());
        }
        let decoded = resolve_reference(reference, self.part)?;
        self.push_text(&decoded);
        Ok(())
    }

    /// Дописать текст в текущий абзац; вне абзаца текст игнорируется.
    fn push_text(&mut self, text: &str) {
        if let Some(cell) = self.cell.as_mut() {
            if cell.in_paragraph {
                if let Some(paragraph) = cell.paragraphs.last_mut() {
                    paragraph.push_str(text);
                }
            }
        }
    }

    /// Атрибуты элемента с именем части в ошибке.
    fn attrs<'a>(&self, element: &'a BytesStart<'a>) -> Result<Vec<Attr<'a>>> {
        attrs(element, self.part)
    }
}

/// Атрибуты элемента; `Vec` вместо `SmallVec` — атрибутов у ячейки единицы, а
/// лишняя зависимость ради экономии одной аллокации не нужна.
fn attrs<'a>(element: &'a BytesStart<'a>, part: &str) -> Result<Vec<Attr<'a>>> {
    let attrs =
        doc_converter_core::xml::attributes(element).map_err(|error| map_core(part, error))?;
    Ok(attrs.into_vec())
}

/// Значение атрибута как owned-строка.
fn owned(attrs: &[Attr<'_>], name: &str) -> Option<String> {
    find(attrs, name).map(str::to_owned)
}

/// Положительное число повтора: `0` и мусор считаются одним элементом.
fn repeat(attrs: &[Attr<'_>], name: &str) -> u32 {
    find(attrs, name)
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .max(1)
}

/// Текстовая ячейка; пустая строка — это [`CellValue::Empty`].
fn text_value(text: &str) -> CellValue {
    if text.is_empty() {
        CellValue::Empty
    } else {
        CellValue::InlineString(text.into())
    }
}

/// `xsd:boolean` ODF: `true`/`false` и запасные `1`/`0`.
fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Текущая формула ячейки без префикса пространства имён и ведущего `=`.
///
/// ODF пишет `table:formula="of:=SUM([.B2:.B3])"`; в модели XLSX формула
/// хранится без `=` и без `of:`. Синтаксис внутри остаётся ODF (`[.B2:.B3]`) —
/// перевод в A1-синтаксис и вычисление это следующий срез.
fn normalize_formula(raw: &str) -> String {
    let raw = raw.trim();
    let raw = raw
        .split_once(":=")
        .map_or(raw, |(_, formula)| formula)
        .trim();
    raw.strip_prefix('=').unwrap_or(raw).to_owned()
}

/// Серийный номер Excel для даты `YYYY-MM-DD[THH:MM[:SS[.fff]]]`.
///
/// Система 1900 года: серийный номер 1 — 01.01.1900, а несуществующее
/// 29.02.1900 занимает номер 60, поэтому после 28.02.1900 отсчёт сдвинут на
/// день. Часовой пояс (`Z`, `+02:00`) не учитывается: модель даты — местное
/// время файла (TODO: часовые пояса).
fn parse_date(raw: &str) -> Option<f64> {
    let raw = raw.trim().trim_end_matches('Z');
    let (date, time) = match raw.split_once('T').or_else(|| raw.split_once(' ')) {
        Some((date, time)) => (date, Some(time)),
        None => (raw, None),
    };

    let mut parts = date.split('-');
    let year: i32 = parts.next()?.trim().parse().ok()?;
    let month: u32 = parts.next()?.trim().parse().ok()?;
    let day: u32 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    #[allow(clippy::cast_precision_loss)]
    // Серийный номер даты меньше 3·10⁶ и в f64 представим точно.
    let whole = excel_serial(year, month, day) as f64;
    if whole < 0.0 {
        // Раньше 01.01.1900 система 1900 не адресует; строка полезнее
        // отрицательного номера, который рисуется решётками.
        return None;
    }

    match time {
        None => Some(whole),
        Some(time) => {
            let seconds = clock_seconds(strip_timezone(time))?;
            Some(whole + seconds / 86_400.0)
        }
    }
}

/// Длительность ODF (`PT12H30M00S`) или время (`12:30:00`) долей суток.
fn parse_time(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let seconds = if raw.starts_with('P') {
        duration_seconds(raw)?
    } else {
        clock_seconds(raw)?
    };
    let fraction = seconds / 86_400.0;
    fraction.is_finite().then_some(fraction)
}

/// ISO 8601 duration в секундах: `P1DT2H30M5.5S`.
fn duration_seconds(raw: &str) -> Option<f64> {
    let rest = raw.strip_prefix('P')?;
    let (days, rest) = match rest.split_once('T') {
        Some((days, time)) => (days.strip_suffix('D').unwrap_or(days), time),
        None => (rest.strip_suffix('D')?, ""),
    };
    let days: f64 = if days.is_empty() {
        0.0
    } else {
        days.parse().ok()?
    };

    let mut total = days * 86_400.0;
    let mut number = String::new();
    for ch in rest.chars() {
        if ch.is_ascii_digit() || ch == '.' || ch == ',' {
            number.push(if ch == ',' { '.' } else { ch });
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        total += match ch {
            'H' => value * 3_600.0,
            'M' => value * 60.0,
            'S' => value,
            _ => return None,
        };
    }
    // Число без единицы в конце — битая длительность.
    if !number.is_empty() {
        return None;
    }
    total.is_finite().then_some(total)
}

/// Время `HH:MM[:SS[.fff]]` в секундах.
fn clock_seconds(raw: &str) -> Option<f64> {
    let mut parts = raw.trim().split(':');
    let hours: f64 = parts.next()?.trim().parse().ok()?;
    let minutes: f64 = parts.next()?.trim().parse().ok()?;
    let seconds: f64 = match parts.next() {
        Some(value) => value.trim().parse().ok()?,
        None => 0.0,
    };
    if parts.next().is_some()
        || hours < 0.0
        || !(0.0..60.0).contains(&minutes)
        || !(0.0..60.0).contains(&seconds)
    {
        return None;
    }
    let total = hours * 3_600.0 + minutes * 60.0 + seconds;
    total.is_finite().then_some(total)
}

/// Отрезать часовой пояс от времени: `10:30:00Z`, `10:30:00+02:00`.
fn strip_timezone(time: &str) -> &str {
    let time = time.strip_suffix('Z').unwrap_or(time);
    match time.find(['+', '-']) {
        Some(index) => &time[..index],
        None => time,
    }
}

/// Серийный номер Excel (система 1900) для даты.
fn excel_serial(year: i32, month: u32, day: u32) -> i64 {
    let days = days_from_civil(year, month, day);
    let epoch = if (year, month, day) < (1900, 3, 1) {
        days_from_civil(1899, 12, 31)
    } else {
        // Фантомное 29.02.1900: после него Excel сдвигает отсчёт на день.
        days_from_civil(1899, 12, 30)
    };
    days - epoch
}

/// Дней от 01.01.1970 до даты — алгоритм Говарда Хиннанта.
///
/// Високосность считается по эре из 400 лет, поэтому формула верна и для
/// дат до 1970 года, включая отрицательные номера лет.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = i64::from((month + 9) % 12);
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Ошибку сборщика листа — в ошибку ODF с именем части.
fn builder_error(part: &str, error: XlsxError) -> OdfError {
    match error {
        XlsxError::Malformed { part, reason } => OdfError::Malformed { part, reason },
        other => OdfError::malformed(part, other.to_string()),
    }
}

#[cfg(test)]
// Даты в тестах — точные серийные номера, сравнение на равенство осмысленно.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn date_serial_follows_excel_1900() {
        assert_eq!(parse_date("1900-01-01"), Some(1.0));
        assert_eq!(parse_date("1900-02-28"), Some(59.0));
        // Фантомный день 60 пропускается: 01.03.1900 — это 61.
        assert_eq!(parse_date("1900-03-01"), Some(61.0));
        assert_eq!(parse_date("2024-01-15"), Some(45_306.0));
        assert_eq!(parse_date("2024-01-15T12:00:00"), Some(45_306.5));
    }

    #[test]
    fn invalid_dates_fall_back() {
        assert_eq!(parse_date("not a date"), None);
        assert_eq!(parse_date("2024-13-01"), None);
        // Серийный номер 0 — 31.12.1899; раньше него система 1900 отрицательна,
        // и такая дата остаётся строкой.
        assert_eq!(parse_date("1899-12-31"), Some(0.0));
        assert_eq!(parse_date("1899-12-30"), None);
    }

    #[test]
    fn durations_and_clocks_are_fractions_of_a_day() {
        let half_past_twelve = 45_000.0 / 86_400.0;
        assert_eq!(parse_time("PT12H30M00S"), Some(half_past_twelve));
        assert_eq!(parse_time("12:30:00"), Some(half_past_twelve));
        assert_eq!(parse_time("P1DT0H0M0S"), Some(1.0));
        assert_eq!(parse_time("PT"), Some(0.0));
        assert_eq!(parse_time("12:99:00"), None);
    }

    #[test]
    fn formula_loses_the_namespace_prefix_only() {
        assert_eq!(normalize_formula("of:=SUM([.B2:.B3])"), "SUM([.B2:.B3])");
        assert_eq!(normalize_formula("=A1+B1"), "A1+B1");
        assert_eq!(normalize_formula("SUM(A1:B1)"), "SUM(A1:B1)");
    }

    #[test]
    fn timezone_is_ignored() {
        assert_eq!(strip_timezone("10:30:00Z"), "10:30:00");
        assert_eq!(strip_timezone("10:30:00+02:00"), "10:30:00");
        assert_eq!(strip_timezone("10:30:00"), "10:30:00");
    }
}
