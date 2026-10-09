//! Рисунки `DrawingML`: inline-картинки и плавающие объекты (`w:drawing`).

use doc_converter_core::NodeId;
use serde::{Deserialize, Serialize};

/// Размер в EMU (английская метрическая единица `DrawingML`): `914_400` EMU = 1 дюйм.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Extent {
    /// Ширина (`wp:extent cx`).
    pub cx: i64,
    /// Высота (`wp:extent cy`).
    pub cy: i64,
}

/// Рисунок в абзаце: заполнено ровно одно из полей `inline`/`anchor`.
///
/// `wp:inline` и `wp:anchor` — разные наборы свойств: у плавающего рисунка есть
/// привязка к позиции и обтекание, которых у inline-картинки нет, поэтому они
/// не сводятся к одному типу.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InlineOrAnchor {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Картинка в потоке текста (`wp:inline`).
    pub inline: Option<InlineImage>,
    /// Плавающий рисунок (`wp:anchor`).
    pub anchor: Option<Anchor>,
}

/// Картинка: ссылка на relationship и разрешённая цель в пакете.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InlineImage {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Идентификатор relationship (`r:embed` или `r:link`).
    pub rel_id: String,
    /// Имя части пакета (`word/media/image1.png`), разрешённое из rels;
    /// `None`, пока rels не разрешены.
    pub part: Option<String>,
    /// Имя объекта (`wp:docPr w:name`) — для отладки и UI.
    pub name: Option<String>,
    /// Описание объекта (`wp:docPr w:descr`) — альтернативный текст.
    pub description: Option<String>,
    /// Размер рисунка (`wp:extent`).
    pub extent: Extent,
}

/// Плавающий рисунок (`wp:anchor`): привязка к позиции и обтекание.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Anchor {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Размер рисунка (`wp:extent`).
    pub extent: Extent,
    /// Горизонтальная позиция (`wp:positionH`).
    pub horizontal: PositionH,
    /// Вертикальная позиция (`wp:positionV`).
    pub vertical: PositionV,
    /// Обтекание рисунка текстом.
    pub wrap: WrapKind,
    /// Рисунок под текстом (`behindDoc="1"`).
    pub behind_text: bool,
    /// Сама картинка.
    pub image: InlineImage,
}

/// Горизонтальная позиция плавающего рисунка (`wp:positionH`).
///
/// `relative_from` задаёт базу отсчёта, а из `align`/`offset`/`percent` заполнено
/// ровно одно — это choice-блок `CT_PosH`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PositionH {
    /// База отсчёта (`relativeFrom`).
    pub relative_from: RelFromH,
    /// Выравнивание относительно базы (`wp:align`).
    pub align: Option<AlignH>,
    /// Смещение от базы в EMU (`wp:posOffset`).
    pub offset: Option<i64>,
    /// Доля от базы в тысячных долях процента: `100_000` = 100%.
    pub percent: Option<i32>,
}

/// Вертикальная позиция плавающего рисунка (`wp:positionV`).
///
/// `relative_from` задаёт базу отсчёта, а из `align`/`offset`/`percent` заполнено
/// ровно одно — это choice-блок `CT_PosV`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PositionV {
    /// База отсчёта (`relativeFrom`).
    pub relative_from: RelFromV,
    /// Выравнивание относительно базы (`wp:align`).
    pub align: Option<AlignV>,
    /// Смещение от базы в EMU (`wp:posOffset`).
    pub offset: Option<i64>,
    /// Доля от базы в тысячных долях процента: `100_000` = 100%.
    pub percent: Option<i32>,
}

/// Обтекание рисунка текстом (`wp:wrap*`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WrapKind {
    /// Обтекания нет, рисунок перекрывает текст (`wp:wrapNone`).
    None,
    /// Обтекание по прямоугольнику (`wp:wrapSquare`).
    Square,
    /// Обтекание по контуру рисунка (`wp:wrapTight`).
    Tight,
    /// Текст проходит сквозь рисунок (`wp:wrapThrough`).
    Through,
    /// Текст только сверху и снизу (`wp:wrapTopAndBottom`).
    TopAndBottom,
}

/// База отсчёта по горизонтали (`ST_RelFromH`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RelFromH {
    /// Поле страницы (`margin`).
    Margin,
    /// Край страницы (`page`).
    Page,
    /// Колонка текста (`column`).
    Column,
    /// Позиция в строке (`character`).
    Character,
    /// Левое поле (`leftMargin`).
    LeftMargin,
    /// Правое поле (`rightMargin`).
    RightMargin,
    /// Внутреннее поле, у корешка (`insideMargin`).
    InsideMargin,
    /// Внешнее поле (`outsideMargin`).
    OutsideMargin,
    /// Незнакомое значение: сохраняем как есть, чтобы не потерять разметку.
    Other(String),
}

/// База отсчёта по вертикали (`ST_RelFromV`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RelFromV {
    /// Поле страницы (`margin`).
    Margin,
    /// Край страницы (`page`).
    Page,
    /// Границы абзаца (`paragraph`).
    Paragraph,
    /// Границы строки текста (`line`).
    Line,
    /// Верхнее поле (`topMargin`).
    TopMargin,
    /// Нижнее поле (`bottomMargin`).
    BottomMargin,
    /// Внутреннее поле, у корешка (`insideMargin`).
    InsideMargin,
    /// Внешнее поле (`outsideMargin`).
    OutsideMargin,
    /// Незнакомое значение: сохраняем как есть, чтобы не потерять разметку.
    Other(String),
}

/// Выравнивание по горизонтали (`ST_AlignH`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AlignH {
    /// По левому краю (`left`).
    Left,
    /// По центру (`center`).
    Center,
    /// По правому краю (`right`).
    Right,
    /// По внутреннему краю, у корешка (`inside`).
    Inside,
    /// По внешнему краю (`outside`).
    Outside,
    /// Незнакомое значение: сохраняем как есть, чтобы не потерять разметку.
    Other(String),
}

/// Выравнивание по вертикали (`ST_AlignV`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AlignV {
    /// По верхнему краю (`top`).
    Top,
    /// По центру (`center`).
    Center,
    /// По нижнему краю (`bottom`).
    Bottom,
    /// По внутреннему краю, у корешка (`inside`).
    Inside,
    /// По внешнему краю (`outside`).
    Outside,
    /// Незнакомое значение: сохраняем как есть, чтобы не потерять разметку.
    Other(String),
}
