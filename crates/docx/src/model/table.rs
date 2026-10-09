//! Таблицы `WordprocessingML`: сетка, строки, ячейки и их свойства (`w:tbl`).

use doc_converter_core::NodeId;
use serde::{Deserialize, Serialize};

use super::raw::{Border, CellVAlign, Justification, Shading, StyleId, Twips};
use super::BlockItem;

/// Таблица (`w:tbl`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Table {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Стиль таблицы (`w:tblStyle`).
    pub style_ref: Option<StyleId>,
    /// Сетка столбцов (`w:tblGrid`).
    pub grid: Vec<GridCol>,
    /// Строки (`w:tr`).
    pub rows: Vec<Row>,
    /// Раскладка таблицы: «не задано» в модели не выражается — если `w:tblLayout`
    /// отсутствует, парсер ставит `Autofit`, умолчание `WordprocessingML`.
    pub layout: TableLayout,
    /// Предпочтительная ширина (`w:tblW`); `None` — ширина не задана.
    pub width: Option<TableWidth>,
    /// Границы таблицы (`w:tblBorders`).
    pub borders: TableBorders,
    /// Признаки оформления из стиля (`w:tblLook`).
    pub look: TableLook,
    /// Выравнивание таблицы (`w:jc` внутри `w:tblPr`).
    pub jc: Option<Justification>,
    /// Отступ таблицы от левого поля (`w:tblInd`).
    pub indent: Option<Twips>,
    /// Умолчания полей ячеек (`w:tblCellMar`).
    pub cell_margins: CellMargins,
}

/// Столбец сетки таблицы (`w:gridCol`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GridCol {
    /// Ширина столбца (`w:gridCol w:w`).
    pub width: Twips,
}

/// Строка таблицы (`w:tr`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Row {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Ячейки строки (`w:tc`).
    pub cells: Vec<Cell>,
    /// Высота строки (`w:trHeight`); `None` — высота по содержимому.
    pub height: Option<RowHeight>,
    /// Строку нельзя разрывать между страницами (`w:cantSplit`).
    pub cant_split: bool,
    /// Повторять строку как заголовок на каждой странице (`w:tblHeader`).
    pub header: bool,
}

/// Ячейка таблицы (`w:tc`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cell {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Число объединяемых по горизонтали столбцов (`w:gridSpan`); `1` — объединения нет.
    pub grid_span: u32,
    /// Вертикальное объединение (`w:vMerge`); `None` — объединения нет.
    pub v_merge: Option<VMerge>,
    /// Ширина ячейки (`w:tcW`).
    pub width: Option<CellWidth>,
    /// Поля ячейки (`w:tcMar`).
    pub margins: CellMargins,
    /// Вертикальное выравнивание содержимого (`w:vAlign`).
    pub v_align: CellVAlign,
    /// Границы ячейки (`w:tcBorders`).
    pub borders: CellBorders,
    /// Заливка ячейки (`w:shd`).
    pub shading: Option<Shading>,
    /// Содержимое ячейки: абзацы и вложенные таблицы.
    pub items: Vec<BlockItem>,
}

/// Вид вертикального объединения ячеек (`w:vMerge`).
///
/// «Объединения нет» выражается `Option`-ом поля `Cell::v_merge`, а не отдельным
/// вариантом: у ячейки без `w:vMerge` свойства нет вовсе.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VMerge {
    /// Продолжение объединения (`continue`): ячейка входит в объединение сверху.
    Continue,
    /// Начало объединения (`restart`).
    Restart,
}

/// Раскладка таблицы (`w:tblLayout`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TableLayout {
    /// Ширины столбцов берутся из сетки (`fixed`).
    Fixed,
    /// Ширины подбираются по содержимому (`autofit`) — умолчание `WordprocessingML`
    /// при отсутствии `w:tblLayout`.
    Autofit,
}

/// Предпочтительная ширина таблицы (`w:tblW`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TableWidth {
    /// Ширина по содержимому (`auto`).
    Auto,
    /// Ширина в twips (`dxa`).
    Dxa(Twips),
    /// Ширина в процентах (`pct`).
    Pct(f32),
}

/// Ширина ячейки (`w:tcW`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CellWidth {
    /// Ширина в twips (`dxa`).
    Dxa(Twips),
    /// Ширина в процентах (`pct`).
    Pct(f32),
    /// Ширина не задана (`nil`).
    Nil,
}

/// Правило, по которому понимается высота строки (`w:trHeight w:hRule`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HeightRule {
    /// Высота по содержимому (`auto`).
    Auto,
    /// Не меньше заданной (`atLeast`).
    AtLeast,
    /// Ровно заданная (`exact`).
    Exact,
}

/// Высота строки (`w:trHeight`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RowHeight {
    /// Значение высоты.
    pub value: Twips,
    /// Как понимать значение.
    pub rule: HeightRule,
}

/// Границы таблицы (`w:tblBorders`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TableBorders {
    /// Верхняя граница (`w:top`).
    pub top: Option<Border>,
    /// Левая граница (`w:left`).
    pub left: Option<Border>,
    /// Нижняя граница (`w:bottom`).
    pub bottom: Option<Border>,
    /// Правая граница (`w:right`).
    pub right: Option<Border>,
    /// Границы между строками (`w:insideH`).
    pub inside_h: Option<Border>,
    /// Границы между столбцами (`w:insideV`).
    pub inside_v: Option<Border>,
}

/// Границы ячейки (`w:tcBorders`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CellBorders {
    /// Верхняя граница (`w:top`).
    pub top: Option<Border>,
    /// Левая граница (`w:left`).
    pub left: Option<Border>,
    /// Нижняя граница (`w:bottom`).
    pub bottom: Option<Border>,
    /// Правая граница (`w:right`).
    pub right: Option<Border>,
}

/// Поля ячейки (`w:tcMar` и умолчания `w:tblCellMar`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CellMargins {
    /// Верхнее поле (`w:top`).
    pub top: Option<Twips>,
    /// Левое поле (`w:left`).
    pub left: Option<Twips>,
    /// Нижнее поле (`w:bottom`).
    pub bottom: Option<Twips>,
    /// Правое поле (`w:right`).
    pub right: Option<Twips>,
}

/// Признаки оформления из табличного стиля (`w:tblLook`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TableLook {
    /// Выделять первую строку (`firstRow`).
    pub first_row: bool,
    /// Выделять последнюю строку (`lastRow`).
    pub last_row: bool,
    /// Выделять первый столбец (`firstColumn`).
    pub first_column: bool,
    /// Выделять последний столбец (`lastColumn`).
    pub last_column: bool,
    /// Отключить чередование заливки строк (`noHBand`).
    pub no_h_band: bool,
    /// Отключить чередование заливки столбцов (`noVBand`).
    pub no_v_band: bool,
}
