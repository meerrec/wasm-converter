//! Модель книги: листы, ячейки, форматы.
//!
//! Ячейки листа лежат в CSR-раскладке: `row_ids` хранит номера непустых строк
//! по возрастанию, `row_starts` — начало каждой строки в общем массиве `cells`
//! (конец строки — начало следующей, у последней — конец массива). Пустые
//! строки не занимают памяти вовсе, а ячейки внутри строки отсортированы по
//! столбцу, поэтому поиск идёт двоичным поиском.

use std::collections::BTreeMap;
use std::fmt;

use crate::cellref::{CellRef, Range, MAX_COL, MAX_ROW};
use crate::dims::SheetDims;
use crate::error::{Result, XlsxError};
use crate::sheet_meta::{Hyperlink, Merges, SheetView};
use crate::strings::SharedStrings;

/// Значение ячейки, как оно записано в файле.
///
/// Строки из `sharedStrings.xml` остаются индексами: общая таблица строк — это
/// ресурс книги, а не ячейки.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// Значения нет — в файле был только формат.
    Empty,
    /// Число. Даты, время и деньги — тоже числа: их смысл задаёт формат.
    Number(f64),
    /// Логическое значение.
    Bool(bool),
    /// Ошибка листа: `#DIV/0!`, `#N/A` и родственные.
    Error(CellError),
    /// Индекс в `sharedStrings.xml` (ячейка `t="s"`).
    SharedString(u32),
    /// Строка записана прямо в ячейке (`t="str"`, `t="inlineStr"`).
    ///
    /// `Box<str>`, а не `String`: ячеек в книге миллион, и лишние восемь байт
    /// на каждую — это восемь мегабайт; расти строка всё равно не будет.
    InlineString(Box<str>),
}

impl CellValue {
    /// Текст ячейки, если он у неё есть: строка из общей таблицы или записанная
    /// прямо в ячейке. У кода ошибки текстом служит он сам — так его и
    /// показывает Excel. У чисел и логических значений текста нет: их вид
    /// задаёт формат.
    #[must_use]
    pub fn text<'a>(&'a self, strings: &'a SharedStrings) -> Option<&'a str> {
        match self {
            Self::SharedString(index) => strings.get(*index),
            Self::InlineString(text) => Some(text),
            Self::Error(error) => Some(error.as_str()),
            Self::Empty | Self::Number(_) | Self::Bool(_) => None,
        }
    }
}

/// Ошибка вычисления, записанная в ячейке (`ST_Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellError {
    /// `#NULL!` — пересечение областей не существует.
    Null,
    /// `#DIV/0!` — деление на ноль.
    Div0,
    /// `#VALUE!` — несовместимый тип аргумента.
    Value,
    /// `#REF!` — ссылка на удалённую ячейку.
    Ref,
    /// `#NAME?` — неизвестное имя.
    Name,
    /// `#NUM!` — недопустимое число.
    Num,
    /// `#N/A` — значение недоступно.
    Na,
}

impl CellError {
    /// Разобрать текст ошибки из `<v>`; `None` — не ошибка.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "#NULL!" => Self::Null,
            "#DIV/0!" => Self::Div0,
            "#VALUE!" => Self::Value,
            "#REF!" => Self::Ref,
            "#NAME?" => Self::Name,
            "#NUM!" => Self::Num,
            "#N/A" => Self::Na,
            _ => return None,
        })
    }

    /// Текст, как он записан в файле.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "#NULL!",
            Self::Div0 => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::Num => "#NUM!",
            Self::Na => "#N/A",
        }
    }
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ячейка внутри строки.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// 0-based индекс столбца; внутри строки столбцы идут по возрастанию.
    pub col: u32,
    /// Индекс формата в [`StyleTable`]; 0 — формат по умолчанию.
    pub style: u32,
    /// Значение.
    pub value: CellValue,
    /// Текст формулы без ведущего `=`, если ячейка вычисляется формулой.
    ///
    /// Формулы есть у единиц процентов ячеек, но поле платит каждая: `Box<str>`
    /// в `Option` занимает восемь байт вместо двадцати четырёх.
    pub formula: Option<Box<str>>,
}

impl Cell {
    /// Ячейка со значением и форматом.
    #[must_use]
    pub const fn new(col: u32, style: u32, value: CellValue) -> Self {
        Self {
            col,
            style,
            value,
            formula: None,
        }
    }

    /// Приписать текст формулы (без ведущего `=`) и вернуть ячейку.
    #[must_use]
    pub fn with_formula(mut self, formula: impl Into<Box<str>>) -> Self {
        self.formula = Some(formula.into());
        self
    }

    /// Адрес ячейки в строке `row`.
    #[must_use]
    pub const fn at(&self, row: u32) -> CellRef {
        CellRef::new(row, self.col)
    }
}

/// Ячейки листа в CSR-раскладке.
///
/// Имени и видимости тут нет: это свойства книги, они лежат в
/// [`WorksheetMeta`], а вместе их сводит [`Sheet`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Worksheet {
    /// Начало каждой строки в `cells`; длина совпадает с `row_ids`.
    row_starts: Vec<u32>,
    /// Номера непустых строк по возрастанию.
    row_ids: Vec<u32>,
    /// Ячейки всех строк подряд, внутри строки — по возрастанию столбца.
    cells: Vec<Cell>,
}

impl Worksheet {
    /// Число ячеек на листе.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Число непустых строк.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_ids.len()
    }

    /// Номер последней непустой строки, 0-based; `None` — лист пуст.
    #[must_use]
    pub fn last_row(&self) -> Option<u32> {
        self.row_ids.last().copied()
    }

    /// Ячейки строки; для отсутствующей строки — пустой срез.
    #[must_use]
    pub fn cells_of_row(&self, row: u32) -> &[Cell] {
        let Ok(i) = self.row_ids.binary_search(&row) else {
            return &[];
        };
        let start = self.row_starts[i] as usize;
        let end = self
            .row_starts
            .get(i + 1)
            .map_or(self.cells.len(), |&next| next as usize);
        &self.cells[start..end]
    }

    /// Ячейка по адресу.
    #[must_use]
    pub fn cell(&self, at: CellRef) -> Option<&Cell> {
        let row = self.cells_of_row(at.row);
        let i = row.binary_search_by_key(&at.col, |cell| cell.col).ok()?;
        row.get(i)
    }

    /// Обход непустых строк по возрастанию номера.
    pub fn rows(&self) -> impl Iterator<Item = (u32, &[Cell])> + '_ {
        (0..self.row_ids.len()).map(|i| (self.row_ids[i], self.cells_of_row(self.row_ids[i])))
    }

    /// Фактические границы ячеек: первая и последняя непустые строки, а внутри
    /// них — крайние столбцы. `None` — на листе нет ячеек.
    ///
    /// Считается по модели, а не берётся из `<dimension ref>`: этот атрибут
    /// врёт в живой книге (Excel не пересчитывает его после удаления строк),
    /// а цена обхода — число непустых строк, а не ячеек: внутри строки столбцы
    /// отсортированы, поэтому крайние берутся с её концов.
    #[must_use]
    pub fn used_range(&self) -> Option<Range> {
        let first_row = *self.row_ids.first()?;
        let last_row = *self.row_ids.last()?;

        let mut min_col = MAX_COL;
        let mut max_col = 0;
        for (_, cells) in self.rows() {
            if let (Some(first), Some(last)) = (cells.first(), cells.last()) {
                min_col = min_col.min(first.col);
                max_col = max_col.max(last.col);
            }
        }
        // Строка попадает в `row_ids` только вместе с ячейкой, поэтому счётчики
        // уже изменились; защита нужна лишь от вырожденного случая.
        if max_col < min_col {
            return None;
        }

        Some(Range {
            first: CellRef::new(first_row, min_col),
            last: CellRef::new(last_row, max_col),
        })
    }
}

/// Сборщик листа.
///
/// Ячейки принимаются в порядке возрастания: сначала строки, внутри строки —
/// столбцы. Именно в таком порядке их отдаёт потоковый парсер `worksheet.xml`,
/// а CSR хранит ячейки подряд, поэтому «шаг назад» — это ошибка, а не повод
/// отсортировать молча.
#[derive(Debug)]
pub struct WorksheetBuilder {
    /// Часть пакета, из которой читается лист, — попадает в текст ошибок.
    part: String,
    worksheet: Worksheet,
    current_row: Option<u32>,
    last_col: Option<u32>,
}

impl WorksheetBuilder {
    /// Начать лист из части пакета `part`; она нужна только для текстов ошибок.
    #[must_use]
    pub fn new(part: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            worksheet: Worksheet::default(),
            current_row: None,
            last_col: None,
        }
    }

    /// Добавить ячейку в строку `row`.
    ///
    /// # Errors
    ///
    /// [`XlsxError::Malformed`], если строка или столбец идут назад, выходят за
    /// лимиты Excel (`XFD1048576`), либо ячеек стало больше, чем адресует
    /// 32-битный индекс CSR.
    pub fn push(&mut self, row: u32, cell: Cell) -> Result<()> {
        if row > MAX_ROW {
            return Err(self.malformed(format!(
                "row number {} is beyond the 1048576 limit",
                u64::from(row) + 1
            )));
        }
        if cell.col > MAX_COL {
            return Err(self.malformed(format!(
                "column number {} is beyond the XFD limit",
                u64::from(cell.col) + 1
            )));
        }

        match self.current_row {
            None => self.start_row(row)?,
            Some(current) if row == current => {
                if let Some(prev) = self.last_col {
                    if cell.col <= prev {
                        return Err(self.malformed(format!(
                            "cells of a row must go in ascending column order, but column {} \
                             follows column {}",
                            u64::from(cell.col) + 1,
                            u64::from(prev) + 1
                        )));
                    }
                }
            }
            Some(current) if row > current => self.start_row(row)?,
            Some(current) => {
                return Err(self.malformed(format!(
                    "rows must go in ascending order, but row {} follows row {}",
                    u64::from(row) + 1,
                    u64::from(current) + 1
                )));
            }
        }

        self.last_col = Some(cell.col);
        self.worksheet.cells.push(cell);
        Ok(())
    }

    /// Закончить лист.
    #[must_use]
    pub fn finish(self) -> Worksheet {
        self.worksheet
    }

    /// Открыть новую строку: запомнить её номер и начало в `cells`.
    fn start_row(&mut self, row: u32) -> Result<()> {
        let start = u32::try_from(self.worksheet.cells.len()).map_err(|_| {
            self.malformed("worksheet holds more cells than the 32-bit CSR index can address")
        })?;
        self.worksheet.row_ids.push(row);
        self.worksheet.row_starts.push(start);
        self.current_row = Some(row);
        self.last_col = None;
        Ok(())
    }

    /// Ошибка с указанием части пакета.
    fn malformed(&self, reason: impl Into<String>) -> XlsxError {
        XlsxError::malformed(self.part.clone(), reason)
    }
}

/// Метаданные листа из `xl/workbook.xml`; содержимое листа — отдельная часть.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorksheetMeta {
    /// Имя листа, как его показывает Excel.
    pub name: String,
    /// Часть пакета с содержимым листа (`xl/worksheets/sheet1.xml`).
    pub part: String,
    /// Видимость листа.
    pub state: SheetState,
}

/// Видимость листа (атрибут `state` в `workbook.xml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SheetState {
    /// Обычный видимый лист.
    #[default]
    Visible,
    /// Скрытый лист.
    Hidden,
    /// Скрытый лист, который нельзя показать через меню Excel.
    VeryHidden,
}

/// Цвет в том виде, в каком он записан в файле.
///
/// `Rgb` записан как `AARRGGBB`; `Theme` и `Indexed` — индексы в палитрах.
/// Конкретный RGB из индекса получается при отрисовке: палитру темы даёт
/// [`Theme`], устаревшая палитра Excel зашита в рендер. Здесь цвет хранится
/// как есть, без потери информации.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// Цвет не задан — действует унаследованный.
    #[default]
    None,
    /// `rgb="AARRGGBB"`.
    Rgb(u32),
    /// `theme="n"` — индекс в палитре темы.
    Theme(u32),
    /// `indexed="n"` — индекс в устаревшей палитре.
    Indexed(u32),
}

/// Число цветов в палитре темы.
pub const THEME_COLOR_COUNT: usize = 12;

/// Тема книги (`xl/theme/theme1.xml`): палитра и схема шрифтов.
///
/// Цвета палитры лежат в порядке индексов `SpreadsheetML`, а не в порядке
/// элементов `<a:clrScheme>`: 0 — `lt1`, 1 — `dk1`, 2 — `lt2`, 3 — `dk2`,
/// 4–9 — `accent1`–`accent6`, 10 — `hlink`, 11 — `folHlink`. Именно к этому
/// порядку отсылает `theme="n"` в `styles.xml`; если взять порядок как в XML,
/// цвет текста по умолчанию (`theme="1"`) станет белым вместо чёрного.
///
/// Схема шрифтов пока никем не читается: у [`Font`] нет поля `scheme`, и
/// рендер гарнитуру из темы не подставляет. Она хранится разобранной, чтобы
/// данные не терялись.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Theme {
    colors: [Color; THEME_COLOR_COUNT],
    major_font: Option<String>,
    minor_font: Option<String>,
}

impl Theme {
    /// Собрать тему из палитры в порядке индексов `SpreadsheetML` (см. описание
    /// [`Theme`]) и гарнитур схемы шрифтов — заголовков и основного текста.
    #[must_use]
    pub fn new(
        colors: [Color; THEME_COLOR_COUNT],
        major_font: Option<String>,
        minor_font: Option<String>,
    ) -> Self {
        Self {
            colors,
            major_font,
            minor_font,
        }
    }

    /// Цвет по индексу темы (`theme="n"`; порядок — в описании [`Theme`]).
    ///
    /// `None` — индекс вне палитры или цвет в слоте не задан.
    #[must_use]
    pub fn color(&self, index: u32) -> Option<Color> {
        self.colors
            .get(usize::try_from(index).ok()?)
            .copied()
            .filter(|color| *color != Color::None)
    }

    /// Гарнитура заголовков (`<a:majorFont><a:latin typeface="…"/>`).
    #[must_use]
    pub fn major_font(&self) -> Option<&str> {
        self.major_font.as_deref()
    }

    /// Гарнитура основного текста (`<a:minorFont><a:latin typeface="…"/>`).
    #[must_use]
    pub fn minor_font(&self) -> Option<&str> {
        self.minor_font.as_deref()
    }
}

/// Гарнитура и начертание (`<font>` из `styles.xml`).
///
/// Четыре независимых флага — это ровно то, что записано в файле: здесь булев
/// набор не состояние объекта, а данные.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    /// Имя шрифта (`<name val="Calibri"/>`).
    pub name: String,
    /// Кегль в пунктах (`<sz val="11"/>`).
    pub size: f32,
    /// Полужирный.
    pub bold: bool,
    /// Курсив.
    pub italic: bool,
    /// Подчёркивание; `<u val="none"/>` его выключает.
    pub underline: bool,
    /// Зачёркивание.
    pub strike: bool,
    /// Цвет текста.
    pub color: Color,
}

impl Default for Font {
    /// Кегль по умолчанию — 11 pt: столько ставит Excel, когда `<sz/>` нет.
    fn default() -> Self {
        Self {
            name: String::new(),
            size: 11.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: Color::None,
        }
    }
}

/// Узор заливки (`ST_PatternType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillPattern {
    /// Заливки нет.
    #[default]
    None,
    /// Сплошная заливка цветом узора.
    Solid,
    /// Серые узоры.
    Gray125,
    Gray0625,
    LightGray,
    MediumGray,
    DarkGray,
    /// Штриховки и сетки.
    LightHorizontal,
    LightVertical,
    LightDown,
    LightUp,
    LightGrid,
    LightTrellis,
    DarkHorizontal,
    DarkVertical,
    DarkDown,
    DarkUp,
    DarkGrid,
    DarkTrellis,
}

impl FillPattern {
    /// Узор по значению `patternType`; незнакомый считается отсутствием заливки.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "solid" => Self::Solid,
            "gray125" => Self::Gray125,
            "gray0625" => Self::Gray0625,
            "lightGray" => Self::LightGray,
            "mediumGray" => Self::MediumGray,
            "darkGray" => Self::DarkGray,
            "lightHorizontal" => Self::LightHorizontal,
            "lightVertical" => Self::LightVertical,
            "lightDown" => Self::LightDown,
            "lightUp" => Self::LightUp,
            "lightGrid" => Self::LightGrid,
            "lightTrellis" => Self::LightTrellis,
            "darkHorizontal" => Self::DarkHorizontal,
            "darkVertical" => Self::DarkVertical,
            "darkDown" => Self::DarkDown,
            "darkUp" => Self::DarkUp,
            "darkGrid" => Self::DarkGrid,
            "darkTrellis" => Self::DarkTrellis,
            _ => Self::None,
        }
    }
}

/// Заливка (`<fill>`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Fill {
    /// Узор.
    pub pattern: FillPattern,
    /// Цвет узора (`fgColor`); у сплошной заливки это и есть цвет ячейки.
    pub foreground: Color,
    /// Цвет фона (`bgColor`).
    pub background: Color,
}

/// Стиль линии рамки (`ST_BorderStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    /// Линии нет.
    #[default]
    None,
    Thin,
    Medium,
    Dashed,
    Dotted,
    Thick,
    Double,
    Hair,
    MediumDashed,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
}

impl BorderStyle {
    /// Стиль по значению `style`; незнакомый считается отсутствием линии.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "thin" => Self::Thin,
            "medium" => Self::Medium,
            "dashed" => Self::Dashed,
            "dotted" => Self::Dotted,
            "thick" => Self::Thick,
            "double" => Self::Double,
            "hair" => Self::Hair,
            "mediumDashed" => Self::MediumDashed,
            "dashDot" => Self::DashDot,
            "mediumDashDot" => Self::MediumDashDot,
            "dashDotDot" => Self::DashDotDot,
            "mediumDashDotDot" => Self::MediumDashDotDot,
            "slantDashDot" => Self::SlantDashDot,
            _ => Self::None,
        }
    }
}

/// Одна сторона рамки.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderSide {
    /// Стиль линии.
    pub style: BorderStyle,
    /// Цвет линии.
    pub color: Color,
}

/// Рамка ячейки (`<border>`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Border {
    /// Левая сторона.
    pub left: BorderSide,
    /// Правая сторона.
    pub right: BorderSide,
    /// Верхняя сторона.
    pub top: BorderSide,
    /// Нижняя сторона.
    pub bottom: BorderSide,
    /// Диагональ.
    pub diagonal: BorderSide,
    /// Диагональ идёт снизу вверх (`diagonalUp`).
    pub diagonal_up: bool,
    /// Диагональ идёт сверху вниз (`diagonalDown`).
    pub diagonal_down: bool,
}

/// Таблица стилей — `styles.xml`.
///
/// Ячейка хранит только индекс формата; индекс 0 — формат по умолчанию, как и
/// в OOXML. Формат ссылается на записи таблиц шрифтов, заливок и рамок.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleTable {
    formats: Vec<CellFormat>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    /// Пользовательские форматы чисел: `numFmtId` → код формата.
    number_formats: BTreeMap<u32, String>,
    /// Дифференциальные форматы (`dxfs`) для условного форматирования.
    dxfs: Vec<Dxf>,
}

impl StyleTable {
    /// Собрать таблицу из разобранных частей `styles.xml` в порядке:
    /// форматы, шрифты, заливки, рамки, пользовательские форматы чисел.
    #[must_use]
    pub fn new(
        formats: Vec<CellFormat>,
        fonts: Vec<Font>,
        fills: Vec<Fill>,
        borders: Vec<Border>,
        number_formats: BTreeMap<u32, String>,
    ) -> Self {
        Self {
            formats,
            fonts,
            fills,
            borders,
            number_formats,
            dxfs: Vec::new(),
        }
    }

    /// Число разобранных форматов.
    #[must_use]
    pub fn len(&self) -> usize {
        self.formats.len()
    }

    /// Таблица пуста — ни одного `xf` не разобрано.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.formats.is_empty()
    }

    /// Формат по индексу, если он объявлен.
    #[must_use]
    pub fn get(&self, index: u32) -> Option<&CellFormat> {
        self.formats.get(index as usize)
    }

    /// Формат по индексу; неизвестный индекс даёт формат по умолчанию.
    ///
    /// Excel так же терпим к битым файлам: ссылка за пределы `cellXfs` не должна
    /// ронять открытие книги.
    #[must_use]
    pub fn resolve(&self, index: u32) -> CellFormat {
        self.formats
            .get(index as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Шрифт по индексу из [`CellFormat::font`].
    #[must_use]
    pub fn font(&self, index: u32) -> Option<&Font> {
        self.fonts.get(index as usize)
    }

    /// Заливка по индексу из [`CellFormat::fill`].
    #[must_use]
    pub fn fill(&self, index: u32) -> Option<&Fill> {
        self.fills.get(index as usize)
    }

    /// Рамка по индексу из [`CellFormat::border`].
    #[must_use]
    pub fn border(&self, index: u32) -> Option<&Border> {
        self.borders.get(index as usize)
    }

    /// Код пользовательского формата числа. Встроенные коды (`numFmtId < 164`)
    /// тут не хранятся — их знает `numfmt` (шаг 8).
    #[must_use]
    pub fn number_format(&self, id: u32) -> Option<&str> {
        self.number_formats.get(&id).map(String::as_str)
    }

    /// Код формата числа: сначала пользовательский из `styles.xml`, затем
    /// встроенный. `None` — код неизвестен, значение показывается как есть.
    #[must_use]
    pub fn format_code(&self, id: u32) -> Option<&str> {
        self.number_format(id)
            .or_else(|| crate::numfmt::builtin(id))
    }

    /// Все шрифты в порядке индексов.
    pub fn fonts(&self) -> impl Iterator<Item = &Font> {
        self.fonts.iter()
    }

    /// Все заливки в порядке индексов.
    pub fn fills(&self) -> impl Iterator<Item = &Fill> {
        self.fills.iter()
    }

    /// Все рамки в порядке индексов.
    pub fn borders(&self) -> impl Iterator<Item = &Border> {
        self.borders.iter()
    }

    /// Все пользовательские форматы чисел.
    pub fn number_formats(&self) -> impl Iterator<Item = (u32, &str)> {
        self.number_formats
            .iter()
            .map(|(&id, code)| (id, code.as_str()))
    }

    /// Добавить дифференциальные форматы (`dxfs`), разобранные из `styles.xml`.
    ///
    /// Отдельным шагом, а не аргументом [`StyleTable::new`]: таблица нужна и
    /// без условного форматирования, а `dxfs` — необязательная секция.
    #[must_use]
    pub fn with_dxfs(mut self, dxfs: Vec<Dxf>) -> Self {
        self.dxfs = dxfs;
        self
    }

    /// Дифференциальный формат по индексу из `dxfId` правила.
    ///
    /// `None` — индекс вне `dxfs`: правило без формата, его нечем применить.
    #[must_use]
    pub fn dxf(&self, index: u32) -> Option<&Dxf> {
        self.dxfs.get(index as usize)
    }

    /// Все дифференциальные форматы в порядке индексов.
    pub fn dxfs(&self) -> impl Iterator<Item = &Dxf> {
        self.dxfs.iter()
    }
}

/// Дифференциальный формат (`<dxf>`): изменения поверх формата ячейки.
///
/// На него ссылается правило условного форматирования (`dxfId`). Группы
/// необязательны: `None` — формат эту группу не трогает. Внутри группы значения
/// по умолчанию тоже означают «не задано»: например, `Font::bold == false`
/// полужирность не включает, но и не гарантирует её снятия — OOXML не различает
/// эти случаи.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dxf {
    /// Шрифт.
    pub font: Option<Font>,
    /// Заливка.
    pub fill: Option<Fill>,
    /// Рамка.
    pub border: Option<Border>,
    /// Формат числа.
    pub number_format: Option<DxfNumberFormat>,
}

/// Формат числа внутри `dxf`.
///
/// В отличие от `cellXfs`, здесь код может лежать прямо в элементе, без ссылки
/// на секцию `numFmts`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DxfNumberFormat {
    /// `numFmtId` — ссылка на встроенный или пользовательский код.
    pub id: Option<u32>,
    /// `formatCode` — код, записанный прямо в `dxf`.
    pub code: Option<String>,
}

/// Блок `<conditionalFormatting>`: диапазоны и действующие на них правила.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalFormatting {
    /// Диапазоны из атрибута `sqref`; в файле их бывает несколько через пробел.
    pub ranges: Vec<Range>,
    /// Правила в порядке файла; очерёдность применения задаёт
    /// [`ConditionalRule::priority`], а не этот порядок.
    pub rules: Vec<ConditionalRule>,
}

/// Правило `<cfRule>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalRule {
    /// Приоритет: меньшее число применяется раньше.
    ///
    /// В ECMA-376 атрибут `priority` обязателен; при его отсутствии или
    /// нечисловом значении правило получает 0 и проверяется первым. Это
    /// допущение разбора, а не требование формата.
    pub priority: u32,
    /// Остановить проверку следующих правил, если это истинно (`stopIfTrue`).
    pub stop_if_true: bool,
    /// Индекс дифференциального формата в [`StyleTable::dxf`]; `None` — формат
    /// не задан: так записаны шкалы, гистограммы и значки, и так же выглядит
    /// правило с потерянным `dxfId`.
    pub dxf_id: Option<u32>,
    /// Содержимое правила.
    pub kind: RuleKind,
}

/// Содержимое правила условного форматирования.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// `cellIs`: значение ячейки сравнивается с формулами.
    CellIs {
        /// Оператор сравнения.
        operator: CellIsOperator,
        /// Формулы: одна у бинарных операторов, две у `between`/`notBetween`.
        formulas: Vec<String>,
    },
    /// `expression`: правило срабатывает, когда истинна формула.
    Expression {
        /// Формулы правила (Excel пишет одну).
        formulas: Vec<String>,
    },
    /// `colorScale`: цвет ячейки по её значению.
    ColorScale(ColorScale),
    /// `dataBar`: полоса пропорционально значению.
    DataBar(DataBar),
    /// `iconSet`: значок по значению.
    IconSet(IconSet),
    /// Вид, который модель не разбирает (`top10`, `aboveAverage`,
    /// `containsText`, `timePeriod`, …). Имя вида сохраняется, чтобы потребитель
    /// мог отличить правило и решить, поддерживать ли его.
    Other {
        /// Значение атрибута `type`, как оно записано в файле.
        rule_type: String,
    },
}

/// Оператор правила `cellIs` (`ST_ConditionalFormattingOperator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellIsOperator {
    /// `<`.
    LessThan,
    /// `<=`.
    LessThanOrEqual,
    /// `=`; сюда же сводится отсутствующий или незнакомый оператор — иначе
    /// правило нельзя было бы вычислить (допущение разбора).
    #[default]
    Equal,
    /// `<>`.
    NotEqual,
    /// `>=`.
    GreaterThanOrEqual,
    /// `>`.
    GreaterThan,
    /// Между двумя формулами включительно.
    Between,
    /// Вне двух формул.
    NotBetween,
}

impl CellIsOperator {
    /// Оператор по значению `operator`; незнакомое значение считается равенством.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "lessThan" => Self::LessThan,
            "lessThanOrEqual" => Self::LessThanOrEqual,
            "notEqual" => Self::NotEqual,
            "greaterThanOrEqual" => Self::GreaterThanOrEqual,
            "greaterThan" => Self::GreaterThan,
            "between" => Self::Between,
            "notBetween" => Self::NotBetween,
            _ => Self::Equal,
        }
    }
}

/// Порог условного правила (`<cfvo>`): точка на шкале значений.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threshold {
    /// Как считается значение порога.
    pub kind: ThresholdKind,
    /// `val` — значение для числовых порогов и формул; у `min`/`max` его нет:
    /// границы берутся из данных.
    pub value: Option<f64>,
    /// `gte`: значение, равное порогу, попадает в диапазон. По умолчанию
    /// включено — так описывает атрибут ECMA-376.
    pub gte: bool,
}

/// Вид порога (`type` у `<cfvo>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThresholdKind {
    /// Минимум диапазона (`min`).
    Min,
    /// Максимум диапазона (`max`).
    Max,
    /// Число (`num`); незнакомый вид тоже считается числом.
    #[default]
    Number,
    /// Процент от диапазона (`percent`), 0…100.
    Percent,
    /// Процентиль (`percentile`), 0…100.
    Percentile,
    /// Формула (`formula`): значение вычисляется выражением.
    Formula,
}

impl ThresholdKind {
    /// Вид порога по значению `type`; незнакомое значение считается числом.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "min" => Self::Min,
            "max" => Self::Max,
            "percent" => Self::Percent,
            "percentile" => Self::Percentile,
            "formula" => Self::Formula,
            _ => Self::Number,
        }
    }
}

/// Цветовая шкала (`<colorScale>`): два или три порога и столько же цветов.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColorScale {
    /// Пороги, как записаны в файле.
    pub thresholds: Vec<Threshold>,
    /// Цвета; в файле их столько же, сколько порогов.
    pub colors: Vec<Color>,
}

/// Гистограмма (`<dataBar>`).
#[derive(Debug, Clone, PartialEq)]
pub struct DataBar {
    /// Пороги длины полосы; обычно `min` и `max`.
    pub thresholds: Vec<Threshold>,
    /// Цвет полосы.
    pub color: Color,
    /// Показывать значение ячейки (`showValue`); умолчание ECMA-376 — да.
    pub show_value: bool,
}

impl Default for DataBar {
    /// Значение показывается, пока в файле не сказано обратное.
    fn default() -> Self {
        Self {
            thresholds: Vec::new(),
            color: Color::None,
            show_value: true,
        }
    }
}

/// Набор значков (`<iconSet>`).
#[derive(Debug, Clone, PartialEq)]
pub struct IconSet {
    /// Имя набора из атрибута `iconSet` (`3TrafficLights1`, `4Arrows`, …);
    /// пустая строка — атрибута не было, действует умолчание Excel.
    pub icon_set: String,
    /// Обратный порядок значков (`reverse`).
    pub reverse: bool,
    /// Показывать значение ячейки (`showValue`); умолчание ECMA-376 — да.
    pub show_value: bool,
    /// Пороги, по одному на значок.
    pub thresholds: Vec<Threshold>,
}

impl Default for IconSet {
    /// Значение показывается, пока в файле не сказано обратное.
    fn default() -> Self {
        Self {
            icon_set: String::new(),
            reverse: false,
            show_value: true,
            thresholds: Vec::new(),
        }
    }
}

/// Горизонтальное выравнивание содержимого ячейки (`ST_HorizontalAlignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalAlign {
    /// Как в файле не сказано — Excel выбирает по типу значения: числа вправо,
    /// текст влево, логические по центру.
    #[default]
    General,
    /// Влево.
    Left,
    /// По центру.
    Center,
    /// Вправо.
    Right,
    /// По центру выделенного диапазона (`centerContinuous`).
    CenterContinuous,
    /// По ширине с заполнением повтором (`fill`).
    Fill,
    /// По ширине (`justify`).
    Justify,
    /// Равномерно (`distributed`).
    Distributed,
}

impl HorizontalAlign {
    /// Выравнивание по значению из `styles.xml`; незнакомое — как в файле.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "left" => Self::Left,
            "center" => Self::Center,
            "right" => Self::Right,
            "centerContinuous" => Self::CenterContinuous,
            "fill" => Self::Fill,
            "justify" => Self::Justify,
            "distributed" => Self::Distributed,
            _ => Self::General,
        }
    }
}

/// Вертикальное выравнивание содержимого ячейки (`ST_VerticalAlignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    /// По нижнему краю — так Excel показывает текст по умолчанию.
    #[default]
    Bottom,
    /// По центру.
    Center,
    /// По верхнему краю.
    Top,
    /// По высоте (`justify`).
    Justify,
    /// Равномерно (`distributed`).
    Distributed,
}

impl VerticalAlign {
    /// Выравнивание по значению из `styles.xml`; незнакомое — по нижнему краю.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "center" => Self::Center,
            "top" => Self::Top,
            "justify" => Self::Justify,
            "distributed" => Self::Distributed,
            _ => Self::Bottom,
        }
    }
}

/// Выравнивание и отступы ячейки (`<alignment>` внутри `<xf>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Alignment {
    /// По горизонтали.
    pub horizontal: HorizontalAlign,
    /// По вертикали.
    pub vertical: VerticalAlign,
    /// Переносить текст по словам.
    pub wrap_text: bool,
    /// Сжимать текст, чтобы поместился (`shrinkToFit`).
    pub shrink_to_fit: bool,
    /// Отступ в единицах ширины символа.
    pub indent: u8,
    /// Поворот текста в градусах (`textRotation`), 0…180.
    pub rotation: u16,
}

/// Формат ячейки: ссылки на записи соответствующих таблиц `styles.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellFormat {
    /// Индекс в `fonts`.
    pub font: u32,
    /// Индекс в `fills`.
    pub fill: u32,
    /// Индекс в `borders`.
    pub border: u32,
    /// `numFmtId` — код встроенного формата или индекс пользовательского.
    pub num_fmt: u32,
    /// Выравнивание.
    pub alignment: Alignment,
}

/// Разобранное содержимое части листа `sheetN.xml`.
///
/// Отдельно от [`Sheet`], потому что парсер части не знает ни имени листа, ни
/// его места в книге: это свойства каталога `workbook.xml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SheetContent {
    /// Ячейки в CSR-раскладке.
    pub cells: Worksheet,
    /// Геометрия, объявленная самим листом.
    pub dims: SheetDims,
    /// Вид листа и закреплённые области.
    pub view: SheetView,
    /// Объединённые ячейки.
    pub merges: Merges,
    /// Гиперссылки.
    pub hyperlinks: Vec<Hyperlink>,
    /// Условное форматирование.
    pub conditional_formatting: Vec<ConditionalFormatting>,
}

/// Лист книги: метаданные из каталога и разобранное содержимое.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    /// Имя, видимость и часть пакета.
    pub meta: WorksheetMeta,
    /// Ячейки в CSR-раскладке.
    pub cells: Worksheet,
    /// Геометрия, объявленная самим листом.
    pub dims: SheetDims,
    /// Вид листа и закреплённые области.
    pub view: SheetView,
    /// Объединённые ячейки.
    pub merges: Merges,
    /// Гиперссылки.
    pub hyperlinks: Vec<Hyperlink>,
    /// Условное форматирование.
    pub conditional_formatting: Vec<ConditionalFormatting>,
}

impl Sheet {
    /// Собрать лист из метаданных каталога и разобранной части.
    #[must_use]
    pub fn new(meta: WorksheetMeta, content: SheetContent) -> Self {
        Self {
            meta,
            cells: content.cells,
            dims: content.dims,
            view: content.view,
            merges: content.merges,
            hyperlinks: content.hyperlinks,
            conditional_formatting: content.conditional_formatting,
        }
    }

    /// Объединение, накрывающее ячейку.
    #[must_use]
    pub fn merged_range(&self, cell: CellRef) -> Option<Range> {
        self.merges.covering(cell)
    }

    /// Гиперссылка, лежащая на ячейке.
    #[must_use]
    pub fn hyperlink_at(&self, cell: CellRef) -> Option<&Hyperlink> {
        self.hyperlinks
            .iter()
            .find(|link| link.range.contains(cell))
    }
}

/// Книга: листы и общие для них таблицы.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workbook {
    sheets: Vec<Sheet>,
    shared_strings: SharedStrings,
    styles: StyleTable,
    theme: Theme,
    date1904: bool,
}

impl Workbook {
    /// Собрать книгу из листов и общих таблиц.
    #[must_use]
    pub fn new(
        sheets: Vec<Sheet>,
        shared_strings: SharedStrings,
        styles: StyleTable,
        theme: Theme,
        date1904: bool,
    ) -> Self {
        Self {
            sheets,
            shared_strings,
            styles,
            theme,
            date1904,
        }
    }

    /// Листы в порядке из `workbook.xml`.
    #[must_use]
    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    /// Число листов.
    #[must_use]
    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    /// Лист по имени.
    #[must_use]
    pub fn sheet(&self, name: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|sheet| sheet.meta.name == name)
    }

    /// Таблица форматов ячеек.
    #[must_use]
    pub fn styles(&self) -> &StyleTable {
        &self.styles
    }

    /// Общая таблица строк: ячейки хранят индексы в ней.
    #[must_use]
    pub fn shared_strings(&self) -> &SharedStrings {
        &self.shared_strings
    }

    /// Тема книги: палитра для `theme="n"` и схема шрифтов.
    ///
    /// Если части темы в пакете нет, тема пуста, и такие цвета не разрешаются.
    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Даты книги отсчитываются от 1904-01-01, а не от 1899-12-30.
    #[must_use]
    pub fn date1904(&self) -> bool {
        self.date1904
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(col: u32, value: f64) -> Cell {
        Cell::new(col, 0, CellValue::Number(value))
    }

    fn bold_arial() -> Font {
        Font {
            name: "Arial".into(),
            size: 12.0,
            bold: true,
            ..Font::default()
        }
    }

    #[test]
    fn empty_worksheet_has_nothing() {
        let ws = Worksheet::default();

        assert_eq!(ws.cell_count(), 0);
        assert_eq!(ws.row_count(), 0);
        assert_eq!(ws.last_row(), None);
        assert!(ws.cells_of_row(0).is_empty());
        assert_eq!(ws.cell(CellRef::new(0, 0)), None);
        assert_eq!(ws.rows().count(), 0);
    }

    #[test]
    fn empty_rows_cost_nothing() {
        let mut b = WorksheetBuilder::new("sheet1.xml");
        b.push(0, number(0, 1.0)).unwrap();
        b.push(5, number(2, 2.0)).unwrap();
        b.push(5, number(3, 3.0)).unwrap();
        let ws = b.finish();

        assert_eq!(ws.cell_count(), 3);
        assert_eq!(ws.row_count(), 2);
        assert_eq!(ws.last_row(), Some(5));

        assert_eq!(ws.cells_of_row(0).len(), 1);
        assert!(ws.cells_of_row(1).is_empty());
        assert_eq!(ws.cells_of_row(5).len(), 2);
        // Строка 5 — последняя: её конец берётся из длины массива ячеек.
        assert_eq!(ws.cells_of_row(5)[1].value, CellValue::Number(3.0));
    }

    #[test]
    fn lookup_by_address() {
        let mut b = WorksheetBuilder::new("sheet1.xml");
        for (row, col) in [(0, 0), (0, 3), (2, 1), (7, 9)] {
            b.push(row, number(col, f64::from(col))).unwrap();
        }
        let ws = b.finish();

        assert_eq!(ws.cell(CellRef::new(0, 3)).unwrap().col, 3);
        assert_eq!(ws.cell(CellRef::new(7, 9)).unwrap().col, 9);
        assert_eq!(ws.cell(CellRef::new(0, 1)), None);
        assert_eq!(ws.cell(CellRef::new(3, 0)), None);
        assert_eq!(ws.cell(CellRef::new(8, 9)), None);
    }

    #[test]
    fn rows_walk_in_order() {
        let mut b = WorksheetBuilder::new("sheet1.xml");
        b.push(2, number(0, 1.0)).unwrap();
        b.push(9, number(1, 2.0)).unwrap();
        let ws = b.finish();

        let seen: Vec<(u32, usize)> = ws.rows().map(|(row, cells)| (row, cells.len())).collect();
        assert_eq!(seen, vec![(2, 1), (9, 1)]);
    }

    #[test]
    fn used_range_covers_extreme_cells() {
        assert_eq!(Worksheet::default().used_range(), None);

        let mut b = WorksheetBuilder::new("sheet1.xml");
        b.push(0, number(4, 1.0)).unwrap();
        b.push(3, number(1, 2.0)).unwrap();
        b.push(3, number(7, 3.0)).unwrap();
        b.push(9, number(2, 4.0)).unwrap();
        let ws = b.finish();

        // Крайние столбцы берутся с концов строк, а не с первой и последней ячейки.
        assert_eq!(
            ws.used_range(),
            Some(Range {
                first: CellRef::new(0, 1),
                last: CellRef::new(9, 7),
            })
        );

        let single = {
            let mut b = WorksheetBuilder::new("sheet1.xml");
            b.push(2, number(3, 1.0)).unwrap();
            b.finish()
        };
        assert_eq!(
            single.used_range(),
            Some(Range {
                first: CellRef::new(2, 3),
                last: CellRef::new(2, 3),
            })
        );
    }

    #[test]
    fn builder_rejects_wrong_order() {
        let mut b = WorksheetBuilder::new("sheet1.xml");
        b.push(3, number(1, 1.0)).unwrap();
        b.push(3, number(2, 2.0)).unwrap();

        let backwards_row = b.push(2, number(0, 3.0)).unwrap_err();
        assert!(matches!(backwards_row, XlsxError::Malformed { .. }));
        assert!(backwards_row.to_string().contains("ascending order"));

        let backwards_col = b.push(3, number(2, 4.0)).unwrap_err();
        assert!(backwards_col.to_string().contains("column order"));

        let same_col = b.push(3, number(1, 5.0)).unwrap_err();
        assert!(same_col.to_string().contains("column order"));
    }

    #[test]
    fn builder_rejects_out_of_range() {
        let mut b = WorksheetBuilder::new("sheet1.xml");

        let bad_row = b.push(MAX_ROW + 1, number(0, 1.0)).unwrap_err();
        assert!(bad_row.to_string().contains("1048576 limit"));

        let bad_col = b.push(0, number(MAX_COL + 1, 1.0)).unwrap_err();
        assert!(bad_col.to_string().contains("XFD limit"));
    }

    #[test]
    fn formula_is_kept_next_to_value() {
        let cell = number(0, 3.0).with_formula("SUM(B1:B2)");

        assert_eq!(cell.formula.as_deref(), Some("SUM(B1:B2)"));
        assert_eq!(cell.at(4), CellRef::new(4, 0));
    }

    #[test]
    fn cell_error_round_trip() {
        let all = [
            CellError::Null,
            CellError::Div0,
            CellError::Value,
            CellError::Ref,
            CellError::Name,
            CellError::Num,
            CellError::Na,
        ];
        for err in all {
            assert_eq!(CellError::parse(err.as_str()), Some(err));
            assert_eq!(err.to_string(), err.as_str());
        }
        assert_eq!(CellError::parse("42"), None);
        assert_eq!(CellError::parse("#div/0!"), None);
    }

    #[test]
    fn style_table_resolves_and_falls_back() {
        let table = StyleTable::new(
            vec![
                CellFormat::default(),
                CellFormat {
                    font: 1,
                    fill: 2,
                    border: 3,
                    num_fmt: 14,
                    ..CellFormat::default()
                },
            ],
            vec![Font::default(), bold_arial()],
            vec![Fill::default()],
            vec![Border::default()],
            BTreeMap::from([(164, "0.00%".to_owned())]),
        );

        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
        assert_eq!(table.get(1).unwrap().num_fmt, 14);
        assert_eq!(table.resolve(1).font, 1);
        // Битая ссылка не роняет разбор: отдаём формат по умолчанию.
        assert_eq!(table.resolve(99), CellFormat::default());
        assert_eq!(table.get(99), None);

        assert_eq!(table.font(1).unwrap(), &bold_arial());
        assert_eq!(table.font(2), None);
        assert_eq!(table.fill(0).unwrap().pattern, FillPattern::None);
        assert_eq!(table.fill(1), None);
        assert_eq!(table.border(0), Some(&Border::default()));
        assert_eq!(table.number_format(164), Some("0.00%"));
        assert_eq!(table.number_format(0), None);
        // Пользовательский код важнее встроенного с тем же номером.
        assert_eq!(table.format_code(164), Some("0.00%"));
        assert_eq!(table.format_code(14), Some("m/d/yyyy"));
        assert_eq!(table.format_code(999), None);
        assert_eq!(table.fonts().count(), 2);
        assert_eq!(table.number_formats().count(), 1);
        assert!(StyleTable::default().is_empty());
    }

    #[test]
    fn workbook_finds_sheet_by_name() {
        let meta = |name: &str, part: &str, state: SheetState| WorksheetMeta {
            name: name.into(),
            part: part.into(),
            state,
        };
        let wb = Workbook::new(
            vec![
                Sheet::new(
                    meta("Данные", "xl/worksheets/sheet1.xml", SheetState::Visible),
                    SheetContent::default(),
                ),
                Sheet::new(
                    meta(
                        "Скрытый",
                        "xl/worksheets/sheet2.xml",
                        SheetState::VeryHidden,
                    ),
                    SheetContent::default(),
                ),
            ],
            SharedStrings::default(),
            StyleTable::default(),
            Theme::default(),
            true,
        );

        assert_eq!(wb.sheet_count(), 2);
        assert!(wb.date1904());
        assert!(wb.theme().color(1).is_none(), "темы в книге нет");
        assert!(wb.shared_strings().is_empty());
        assert_eq!(
            wb.sheet("Данные").unwrap().meta.part,
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            wb.sheet("Скрытый").unwrap().meta.state,
            SheetState::VeryHidden
        );
        assert_eq!(wb.sheet("нет такого"), None);
        assert_eq!(wb.sheets().len(), 2);
        assert!(wb.styles().is_empty());
        assert_eq!(SheetState::default(), SheetState::Visible);
    }

    #[test]
    fn cell_text_resolves_shared_strings() {
        let xml = "<sst><si><t>Привет</t></si></sst>";
        let strings = SharedStrings::parse(xml.as_bytes(), "xl/sharedStrings.xml").unwrap();

        assert_eq!(CellValue::SharedString(0).text(&strings), Some("Привет"));
        // Индекс за пределами таблицы текста не даёт.
        assert_eq!(CellValue::SharedString(7).text(&strings), None);
        assert_eq!(
            CellValue::InlineString("строка".into()).text(&strings),
            Some("строка")
        );
        // Код ошибки показывается как есть — так его рисует и Excel.
        assert_eq!(
            CellValue::Error(CellError::Div0).text(&strings),
            Some("#DIV/0!")
        );
        assert_eq!(CellValue::Number(1.0).text(&strings), None);
        assert_eq!(CellValue::Empty.text(&strings), None);
    }

    #[test]
    fn cell_is_operators_parse_and_degrade() {
        let cases = [
            ("lessThan", CellIsOperator::LessThan),
            ("lessThanOrEqual", CellIsOperator::LessThanOrEqual),
            ("equal", CellIsOperator::Equal),
            ("notEqual", CellIsOperator::NotEqual),
            ("greaterThanOrEqual", CellIsOperator::GreaterThanOrEqual),
            ("greaterThan", CellIsOperator::GreaterThan),
            ("between", CellIsOperator::Between),
            ("notBetween", CellIsOperator::NotBetween),
        ];
        for (raw, expected) in cases {
            assert_eq!(CellIsOperator::parse(raw), expected, "{raw}");
        }
        // Незнакомый оператор сводится к равенству, а не роняет разбор.
        assert_eq!(CellIsOperator::parse("beginsWith"), CellIsOperator::Equal);
        assert_eq!(CellIsOperator::parse(""), CellIsOperator::Equal);
    }

    #[test]
    fn threshold_kinds_parse_and_degrade() {
        let cases = [
            ("min", ThresholdKind::Min),
            ("max", ThresholdKind::Max),
            ("num", ThresholdKind::Number),
            ("percent", ThresholdKind::Percent),
            ("percentile", ThresholdKind::Percentile),
            ("formula", ThresholdKind::Formula),
        ];
        for (raw, expected) in cases {
            assert_eq!(ThresholdKind::parse(raw), expected, "{raw}");
        }
        // Незнакомый вид считается числом: `val` у него осмыслен.
        assert_eq!(ThresholdKind::parse("autoMin"), ThresholdKind::Number);
        assert_eq!(ThresholdKind::parse(""), ThresholdKind::Number);
    }

    #[test]
    fn visual_rules_show_the_value_by_default() {
        assert!(DataBar::default().show_value);
        assert!(IconSet::default().show_value);
        assert!(ColorScale::default().thresholds.is_empty());
        assert!(IconSet::default().thresholds.is_empty());
    }

    #[test]
    fn dxf_defaults_to_touching_nothing() {
        let dxf = Dxf::default();

        assert_eq!(dxf, Dxf::default());
        assert_eq!(dxf.font, None);
        assert_eq!(dxf.fill, None);
        assert_eq!(dxf.border, None);
        assert_eq!(dxf.number_format, None);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// CSR-поиск должен согласовываться с множеством поданных адресов.
        #[test]
        fn builder_matches_address_set(
            addresses in proptest::collection::btree_set((0u32..40, 0u32..12), 0..60),
        ) {
            let mut builder = WorksheetBuilder::new("sheet1.xml");
            for &(row, col) in &addresses {
                builder.push(row, Cell::new(col, 0, CellValue::Number(f64::from(col)))).unwrap();
            }
            let sheet = builder.finish();

            prop_assert_eq!(sheet.cell_count(), addresses.len());
            for row in 0..40u32 {
                for col in 0..12u32 {
                    let found = sheet.cell(CellRef::new(row, col)).is_some();
                    prop_assert_eq!(found, addresses.contains(&(row, col)));
                }
            }
        }
    }
}
