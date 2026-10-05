//! Геометрия листа: границы, ширины столбцов, высоты строк.
//!
//! Всё здесь — **объявленная** геометрия: файл рассказывает о ней отдельными
//! элементами (`<dimension>`, `<cols>`, `<sheetFormatPr>`), и верить ей нельзя.
//! Атрибут `ref` у `<dimension>` регулярно врёт — Excel оставляет в нём старые
//! границы после удаления строк и столбцов, а Google Sheets пишет туда
//! прямоугольник всего используемого диапазона ещё до записи ячеек. Поэтому
//! фактические границы считает [`Worksheet::used_range`](crate::Worksheet::used_range)
//! по разобранным ячейкам, а объявленные лежат рядом: расхождение между ними —
//! это то, что видно в отчёте о разборе, и молча подменять одно другим значило
//! бы потерять признак битого файла.

use crate::cellref::{Range, MAX_COL};
use crate::error::{Result, XlsxError};
use crate::xml::{find, is_true, Attr};

/// Высота строки по умолчанию, пункты: столько ставит Excel для Calibri 11.
pub const DEFAULT_ROW_HEIGHT: f32 = 15.0;
/// Ширина столбца по умолчанию, «символы» Excel.
pub const DEFAULT_COL_WIDTH: f32 = 8.43;
/// Базовая ширина столбца, «символы»: ширина символа `0` в шрифте книги.
pub const DEFAULT_BASE_COL_WIDTH: f32 = 8.0;

/// Ширина одного столбца или группы подряд идущих (`<col>`).
///
/// Excel пишет один `<col>` на диапазон одинаковых столбцов, поэтому `min`/`max`
/// здесь — не редкость, а норма: лист на 16 384 столбца часто описан одной
/// записью.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColWidth {
    /// Первый столбец диапазона, 0-based, включительно.
    pub first: u32,
    /// Последний столбец диапазона, 0-based, включительно.
    pub last: u32,
    /// Ширина в «символах» Excel — единицах ширины символа `0` основного шрифта.
    pub width: f32,
    /// Ширина задана пользователем (`customWidth`), а не выведена из содержимого.
    pub custom: bool,
    /// Столбец скрыт.
    pub hidden: bool,
    /// Ширина подобрана по содержимому (`bestFit`).
    pub best_fit: bool,
}

impl ColWidth {
    /// Попадает ли столбец в диапазон.
    #[must_use]
    pub const fn contains(&self, col: u32) -> bool {
        col >= self.first && col <= self.last
    }

    /// Ширина с учётом скрытости: у скрытого столбца она нулевая.
    #[must_use]
    pub fn effective_width(&self) -> f32 {
        if self.hidden {
            0.0
        } else {
            self.width
        }
    }
}

/// Ширины столбцов листа в порядке из файла.
///
/// Диапазоны не пересекаются — так их пишет Excel, — но полагаться на это в
/// разборе чужого файла нельзя, поэтому поиск возвращает первый подходящий
/// диапазон и не пытается разрешать наложения.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColWidths {
    spans: Vec<ColWidth>,
}

impl ColWidths {
    /// Все диапазоны в порядке из файла.
    #[must_use]
    pub fn spans(&self) -> &[ColWidth] {
        &self.spans
    }

    /// Число диапазонов.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Ни одного `<col>` не объявлено.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Диапазон, накрывающий столбец.
    #[must_use]
    pub fn span(&self, col: u32) -> Option<&ColWidth> {
        self.spans.iter().find(|span| span.contains(col))
    }

    /// Ширина столбца в «символах» Excel с учётом скрытости; `None` — столбец
    /// описан не был, действует [`SheetFormat::default_col_width`].
    #[must_use]
    pub fn width_of(&self, col: u32) -> Option<f32> {
        self.span(col).map(ColWidth::effective_width)
    }

    /// Скрыт ли столбец.
    #[must_use]
    pub fn is_hidden(&self, col: u32) -> bool {
        self.span(col).is_some_and(|span| span.hidden)
    }

    /// Добавить диапазон.
    pub fn push(&mut self, span: ColWidth) {
        self.spans.push(span);
    }
}

/// Высота одной строки (`<row ht="…">`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowHeight {
    /// Номер строки, 0-based.
    pub row: u32,
    /// Высота в пунктах.
    pub height: f32,
    /// Высота задана пользователем (`customHeight`).
    pub custom: bool,
    /// Строка скрыта.
    pub hidden: bool,
    /// Уровень группировки (`outlineLevel`), 0 — вне группы.
    pub outline_level: u8,
}

/// Высоты строк листа по возрастанию номера.
///
/// Строк с нестандартной высотой в книге единицы, поэтому список разреженный:
/// строка, которой тут нет, рисуется высотой по умолчанию.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowHeights {
    rows: Vec<RowHeight>,
}

impl RowHeights {
    /// Все записи по возрастанию номера строки.
    #[must_use]
    pub fn rows(&self) -> &[RowHeight] {
        &self.rows
    }

    /// Число строк с собственной высотой.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Ни одной строки с собственной высотой.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Высота строки с учётом скрытости: у скрытой она нулевая. `None` — строка
    /// описана не была, действует [`SheetFormat::default_row_height`].
    #[must_use]
    pub fn height_of(&self, row: u32) -> Option<f32> {
        let i = self.rows.binary_search_by_key(&row, |r| r.row).ok()?;
        let entry = self.rows.get(i)?;
        Some(if entry.hidden { 0.0 } else { entry.height })
    }

    /// Скрыта ли строка.
    #[must_use]
    pub fn is_hidden(&self, row: u32) -> bool {
        self.rows
            .binary_search_by_key(&row, |r| r.row)
            .ok()
            .and_then(|i| self.rows.get(i))
            .is_some_and(|entry| entry.hidden)
    }

    /// Добавить запись; порядок восстанавливается в [`Self::sort`].
    pub fn push(&mut self, height: RowHeight) {
        self.rows.push(height);
    }

    /// Привести записи в порядок по номеру строки.
    ///
    /// `<row>` идут по возрастанию, но строка без ячеек порядок не проверяет:
    /// у битого файла запись о высоте может встать не на место, а бинарный
    /// поиск этого не прощает.
    pub fn sort(&mut self) {
        self.rows.sort_unstable_by_key(|entry| entry.row);
    }
}

/// Параметры листа по умолчанию (`<sheetFormatPr>`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetFormat {
    /// Высота строки по умолчанию, пункты.
    pub default_row_height: f32,
    /// Ширина столбца по умолчанию, «символы» Excel.
    pub default_col_width: f32,
    /// Базовая ширина столбца, «символы»: ширина символа `0`.
    pub base_col_width: f32,
    /// Высота по умолчанию задана пользователем (`customHeight`).
    pub custom_height: bool,
    /// Скрыты все строки (`zeroHeight`); отдельные `<row>` это переопределяют.
    pub zero_height: bool,
}

impl Default for SheetFormat {
    fn default() -> Self {
        Self {
            default_row_height: DEFAULT_ROW_HEIGHT,
            default_col_width: DEFAULT_COL_WIDTH,
            base_col_width: DEFAULT_BASE_COL_WIDTH,
            custom_height: false,
            zero_height: false,
        }
    }
}

impl SheetFormat {
    /// Высота строки по умолчанию с учётом `zeroHeight`.
    #[must_use]
    pub fn effective_row_height(&self) -> f32 {
        if self.zero_height {
            0.0
        } else {
            self.default_row_height
        }
    }
}

/// Геометрия листа, как её объявил файл.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SheetDims {
    /// Диапазон из `<dimension ref="…">`, если он есть в файле.
    pub declared: Option<Range>,
    /// Ширины столбцов.
    pub cols: ColWidths,
    /// Высоты строк.
    pub rows: RowHeights,
    /// Параметры по умолчанию.
    pub format: SheetFormat,
}

impl SheetDims {
    /// Ширина столбца в «символах» Excel: своя, если объявлена, иначе общая.
    #[must_use]
    pub fn col_width(&self, col: u32) -> f32 {
        self.cols
            .width_of(col)
            .unwrap_or(self.format.default_col_width)
    }

    /// Высота строки в пунктах: своя, если объявлена, иначе общая.
    #[must_use]
    pub fn row_height(&self, row: u32) -> f32 {
        self.rows
            .height_of(row)
            .unwrap_or_else(|| self.format.effective_row_height())
    }
}

/// Разобрать `<dimension ref="A1:C5"/>`.
///
/// Диапазон из одной ячейки (`ref="A1"`) — законная запись для листа 1×1,
/// поэтому двоеточие здесь необязательно, в отличие от [`Range::parse`].
///
/// # Errors
///
/// [`XlsxError::Malformed`] — ссылка не разбирается как диапазон или ячейка.
pub(crate) fn read_dimension(attrs: &[Attr<'_>], part: &str) -> Result<Option<Range>> {
    let Some(raw) = find(attrs, "ref") else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    Range::parse_ref(raw)
        .map(|range| Some(range.normalized()))
        .map_err(|e| XlsxError::malformed(part, format!("dimension `{raw}` is not a range: {e}")))
}

/// Прочитать `<col>`.
///
/// Столбцы в файле нумеруются с единицы, в модели — с нуля.
pub(crate) fn read_col(attrs: &[Attr<'_>]) -> ColWidth {
    let number = |name: &str| find(attrs, name).and_then(|raw| raw.trim().parse::<u32>().ok());

    let min = number("min").unwrap_or(1).max(1);
    let max = number("max").unwrap_or(min).max(min);
    let first = (min - 1).min(MAX_COL);
    let last = (max - 1).min(MAX_COL);

    ColWidth {
        first,
        last,
        width: read_f32(attrs, "width").unwrap_or(DEFAULT_COL_WIDTH),
        custom: is_flag(attrs, "customWidth"),
        hidden: is_flag(attrs, "hidden"),
        best_fit: is_flag(attrs, "bestFit"),
    }
}

/// Прочитать `<row>`; `None` — строка не отличается от общей по умолчанию.
pub(crate) fn read_row(row: u32, attrs: &[Attr<'_>]) -> Option<RowHeight> {
    let height = read_f32(attrs, "ht");
    let hidden = is_flag(attrs, "hidden");
    let outline_level = find(attrs, "outlineLevel")
        .and_then(|raw| raw.trim().parse::<u8>().ok())
        .unwrap_or(0);

    if height.is_none() && !hidden && outline_level == 0 {
        return None;
    }

    Some(RowHeight {
        row,
        height: height.unwrap_or(DEFAULT_ROW_HEIGHT),
        custom: is_flag(attrs, "customHeight"),
        hidden,
        outline_level,
    })
}

/// Прочитать `<sheetFormatPr>`.
pub(crate) fn read_format(attrs: &[Attr<'_>]) -> SheetFormat {
    let mut format = SheetFormat::default();
    if let Some(value) = read_f32(attrs, "defaultRowHeight") {
        format.default_row_height = value;
    }
    if let Some(value) = read_f32(attrs, "defaultColWidth") {
        format.default_col_width = value;
    }
    if let Some(value) = read_f32(attrs, "baseColWidth") {
        format.base_col_width = value;
    }
    format.custom_height = is_flag(attrs, "customHeight");
    format.zero_height = is_flag(attrs, "zeroHeight");
    format
}

/// Число из атрибута; мусор и `NaN` игнорируются — как и всё остальное в
/// геометрии, это не повод не открыть книгу.
fn read_f32(attrs: &[Attr<'_>], name: &str) -> Option<f32> {
    let value: f32 = find(attrs, name)?.trim().parse().ok()?;
    value.is_finite().then_some(value)
}

/// Флаг `xsd:boolean` из атрибута.
fn is_flag(attrs: &[Attr<'_>], name: &str) -> bool {
    find(attrs, name).is_some_and(is_true)
}

#[cfg(test)]
// Ширины и высоты в тестах — точные литералы, представимые в `f32`,
// поэтому сравнение на равенство здесь осмысленно.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::cellref::CellRef;
    use crate::xml::attributes;
    use quick_xml::events::BytesStart;

    /// Атрибуты из строки вида `min="1" max="3"`.
    ///
    /// Элемент утекает намеренно: `attributes` возвращает значения, одолженные
    /// у него, а держать рядом владеющий буфер в каждой проверке — лишний шум.
    fn attrs(xml: &'static str) -> Vec<Attr<'static>> {
        let start: &'static BytesStart<'static> =
            Box::leak(Box::new(BytesStart::from_content(xml, 0)));
        attributes(start, "test")
            .unwrap_or_else(|e| panic!("attributes: {e}"))
            .into_vec()
    }

    #[test]
    fn dimension_accepts_range_and_single_cell() {
        let range = read_dimension(&attrs(r#"ref="B2:D9""#), "s")
            .unwrap()
            .unwrap();
        assert_eq!(range.first, CellRef::new(1, 1));
        assert_eq!(range.last, CellRef::new(8, 3));

        // Лист 1×1 записывается одной ячейкой.
        let single = read_dimension(&attrs(r#"ref="A1""#), "s").unwrap().unwrap();
        assert_eq!(single.first, single.last);
        assert_eq!(single.first, CellRef::new(0, 0));

        // Границы в обратном порядке нормализуются.
        let backwards = read_dimension(&attrs(r#"ref="D9:B2""#), "s")
            .unwrap()
            .unwrap();
        assert_eq!(backwards.first, CellRef::new(1, 1));
        assert_eq!(backwards.last, CellRef::new(8, 3));
    }

    #[test]
    fn dimension_is_optional_but_must_parse() {
        assert_eq!(read_dimension(&attrs(""), "s").unwrap(), None);
        assert_eq!(read_dimension(&attrs(r#"ref="""#), "s").unwrap(), None);

        let err = read_dimension(&attrs(r#"ref="A1:B""#), "s").unwrap_err();
        assert!(matches!(err, XlsxError::Malformed { .. }));
        assert!(err.to_string().contains("A1:B"));
    }

    #[test]
    fn col_span_is_one_based_in_file() {
        let col = read_col(&attrs(r#"min="2" max="4" width="12.5" customWidth="1""#));

        assert_eq!(col.first, 1);
        assert_eq!(col.last, 3);
        assert_eq!(col.width, 12.5);
        assert!(col.custom);
        assert!(!col.hidden);
        assert!(col.contains(1) && col.contains(3));
        assert!(!col.contains(0) && !col.contains(4));
    }

    #[test]
    fn col_defaults_and_limits() {
        // Без `max` диапазон — один столбец; без `width` — ширина по умолчанию.
        let single = read_col(&attrs(r#"min="1""#));
        assert_eq!((single.first, single.last), (0, 0));
        assert_eq!(single.width, DEFAULT_COL_WIDTH);

        // Мусор в номерах не выводит диапазон за лимиты Excel.
        let broken = read_col(&attrs(r#"min="0" max="999999""#));
        assert_eq!(broken.first, 0);
        assert_eq!(broken.last, MAX_COL);

        let hidden = read_col(&attrs(r#"min="1" width="0" hidden="1""#));
        assert!(hidden.hidden);
        assert_eq!(hidden.effective_width(), 0.0);
    }

    #[test]
    fn cols_lookup_and_defaults() {
        let mut cols = ColWidths::default();
        cols.push(read_col(&attrs(r#"min="2" max="3" width="20""#)));

        assert_eq!(cols.len(), 1);
        assert!(!cols.is_empty());
        assert_eq!(cols.width_of(1), Some(20.0));
        assert_eq!(cols.width_of(2), Some(20.0));
        assert_eq!(cols.width_of(0), None);
        assert!(!cols.is_hidden(1));

        let dims = SheetDims {
            cols,
            ..SheetDims::default()
        };
        assert_eq!(dims.col_width(1), 20.0);
        // Столбец без записи берёт общую ширину.
        assert_eq!(dims.col_width(0), DEFAULT_COL_WIDTH);
    }

    #[test]
    fn row_entry_only_for_remarkable_rows() {
        assert_eq!(read_row(0, &attrs(r#"r="1""#)), None);

        let tall = read_row(4, &attrs(r#"r="5" ht="30" customHeight="1""#)).unwrap();
        assert_eq!(tall.row, 4);
        assert_eq!(tall.height, 30.0);
        assert!(tall.custom);

        // Скрытая строка без высоты: высота по умолчанию, но запись нужна.
        let hidden = read_row(1, &attrs(r#"r="2" hidden="1""#)).unwrap();
        assert!(hidden.hidden);
        assert_eq!(hidden.height, DEFAULT_ROW_HEIGHT);

        let grouped = read_row(2, &attrs(r#"r="3" outlineLevel="2""#)).unwrap();
        assert_eq!(grouped.outline_level, 2);
    }

    #[test]
    fn row_heights_are_looked_up_by_number() {
        let mut rows = RowHeights::default();
        rows.push(read_row(5, &attrs(r#"ht="30""#)).unwrap());
        rows.push(read_row(1, &attrs(r#"hidden="1""#)).unwrap());
        rows.sort();

        assert_eq!(rows.rows().first().unwrap().row, 1);
        assert_eq!(rows.height_of(5), Some(30.0));
        // Скрытая строка имеет нулевую высоту.
        assert_eq!(rows.height_of(1), Some(0.0));
        assert!(rows.is_hidden(1));
        assert!(!rows.is_hidden(5));
        assert_eq!(rows.height_of(2), None);

        let dims = SheetDims {
            rows,
            ..SheetDims::default()
        };
        assert_eq!(dims.row_height(5), 30.0);
        assert_eq!(dims.row_height(2), DEFAULT_ROW_HEIGHT);
    }

    #[test]
    fn format_defaults_and_overrides() {
        let format = SheetFormat::default();
        assert_eq!(format.effective_row_height(), DEFAULT_ROW_HEIGHT);
        assert_eq!(format.default_col_width, DEFAULT_COL_WIDTH);
        assert_eq!(format.base_col_width, DEFAULT_BASE_COL_WIDTH);

        let explicit = read_format(&attrs(
            r#"defaultRowHeight="12.75" defaultColWidth="9" baseColWidth="7""#,
        ));
        assert_eq!(explicit.default_row_height, 12.75);
        assert_eq!(explicit.default_col_width, 9.0);
        assert_eq!(explicit.base_col_width, 7.0);

        let hidden = read_format(&attrs(r#"defaultRowHeight="15" zeroHeight="1""#));
        assert!(hidden.zero_height);
        assert_eq!(hidden.effective_row_height(), 0.0);

        // Мусор вместо числа не ломает разбор.
        let broken = read_format(&attrs(r#"defaultRowHeight="высокая""#));
        assert_eq!(broken.default_row_height, DEFAULT_ROW_HEIGHT);
    }
}
