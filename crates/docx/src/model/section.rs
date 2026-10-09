//! Секции документа: параметры страницы, колонки и ссылки на колонтитулы.

use doc_converter_core::NodeId;
use serde::{Deserialize, Serialize};

use super::raw::Twips;
use super::Body;

/// Секция документа: свойства страницы и колонтитулы, сведённые в удобный вид.
///
/// Часть полей дублирует [`SectionProperties`]: их заполняет парсер при сборке секции,
/// а `properties` остаётся сырым разобранным `w:sectPr` (план §3.1: «Section с колонтитулами
/// и page setup»).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Section {
    /// Идентификатор узла: сопоставление модели с XML и snapshot-тесты (ADR-0019).
    pub id: NodeId,
    /// Сырые свойства секции — ровно то, что было в XML.
    pub properties: SectionProperties,
    /// Колонтитул по умолчанию (`w:headerReference w:type="default"`).
    pub header_default: Option<PartRef>,
    /// Колонтитул первой страницы (`w:type="first"`); печатается при `title_pg`.
    pub header_first: Option<PartRef>,
    /// Колонтитул чётных страниц (`w:type="even"`); печатается при `even_and_odd_headers`.
    pub header_even: Option<PartRef>,
    /// Нижний колонтитул по умолчанию (`w:footerReference w:type="default"`).
    pub footer_default: Option<PartRef>,
    /// Нижний колонтитул первой страницы.
    pub footer_first: Option<PartRef>,
    /// Нижний колонтитул чётных страниц.
    pub footer_even: Option<PartRef>,
    /// `w:titlePg`: у первой страницы секции свой колонтитул.
    pub title_pg: bool,
    /// Размер страницы.
    pub page_size: PageSize,
    /// Ориентация страницы.
    pub orientation: Orientation,
    /// Поля страницы.
    pub margins: Margins,
    /// Колонки текста.
    pub columns: Columns,
}

/// Сырые свойства секции: ровно то, что было в `w:sectPr`.
///
/// Один тип обслуживает оба места: `w:sectPr` в конце тела документа и `w:sectPr` внутри `w:pPr`,
/// где он завершает секцию, а не открывает её.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SectionProperties {
    /// Размер страницы (`w:pgSz`).
    pub page_size: PageSize,
    /// Ориентация (`w:pgSz w:orient`); без атрибута — книжная.
    pub orientation: Orientation,
    /// Поля страницы (`w:pgMar`).
    pub margins: Margins,
    /// Колонки (`w:cols`).
    pub columns: Columns,
    /// `w:titlePg`.
    pub title_pg: bool,
    /// Колонтитул по умолчанию.
    pub header_default: Option<PartRef>,
    /// Колонтитул первой страницы.
    pub header_first: Option<PartRef>,
    /// Колонтитул чётных страниц.
    pub header_even: Option<PartRef>,
    /// Нижний колонтитул по умолчанию.
    pub footer_default: Option<PartRef>,
    /// Нижний колонтитул первой страницы.
    pub footer_first: Option<PartRef>,
    /// Нижний колонтитул чётных страниц.
    pub footer_even: Option<PartRef>,
    /// Вид разрыва перед секцией (`w:type`); `None` — умолчание `nextPage`.
    pub section_type: Option<SectionType>,
    /// Нераспознанные дочерние элементы: (локальное имя, XML элемента).
    pub unknown: Vec<(String, String)>,
}

/// Ссылка на часть пакета (`w:headerReference`/`w:footerReference`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PartRef {
    /// Идентификатор отношения (`r:id`) в rels той части, где встретилась ссылка.
    pub rel_id: String,
    /// Цель отношения, разрешённая в имя части пакета (`word/header1.xml`).
    pub target: String,
    /// Внешняя цель (`TargetMode="External"`); для колонтитулов не встречается, но модель общая.
    pub external: bool,
}

/// Размер страницы (`w:pgSz`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PageSize {
    /// Ширина страницы в twips.
    pub width: Twips,
    /// Высота страницы в twips.
    pub height: Twips,
}

/// Ориентация страницы (`w:pgSz w:orient`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Книжная (умолчание `WordprocessingML`).
    Portrait,
    /// Альбомная.
    Landscape,
}

/// Поля страницы (`w:pgMar`) в twips.
///
/// Отсутствие `w:pgMar` — не нулевые поля: умолчания `WordprocessingML` расставляет парсер,
/// поэтому `Default` у типа нет.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Margins {
    /// Верхнее поле.
    pub top: Twips,
    /// Правое поле.
    pub right: Twips,
    /// Нижнее поле.
    pub bottom: Twips,
    /// Левое поле.
    pub left: Twips,
    /// Поле до верхнего колонтитула (`w:header`).
    pub header: Option<Twips>,
    /// Поле до нижнего колонтитула (`w:footer`).
    pub footer: Option<Twips>,
    /// Добавка на переплёт (`w:gutter`).
    pub gutter: Option<Twips>,
}

/// Колонки текста (`w:cols`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Columns {
    /// Число колонок (`w:num`).
    pub count: u32,
    /// Промежуток между колонками (`w:space`).
    pub space: Twips,
    /// `w:equalWidth`: колонки одинаковой ширины, `defs` в этом случае может быть пуст.
    pub equal_width: bool,
    /// `w:sep`: разделительная линия между колонками.
    pub separator: bool,
    /// Явные ширины колонок (`w:col`).
    pub defs: Vec<ColumnDef>,
}

/// Ширина одной колонки (`w:col`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ColumnDef {
    /// Ширина колонки в twips.
    pub width: Twips,
    /// Промежуток после колонки в twips.
    pub space: Twips,
}

/// Вид разрыва перед секцией (`w:type`, `ST_SectionMark`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SectionType {
    /// С новой страницы (умолчание `WordprocessingML`).
    NextPage,
    /// На текущей странице.
    Continuous,
    /// С ближайшей чётной страницы.
    EvenPage,
    /// С ближайшей нечётной страницы.
    OddPage,
}

/// Разобранный колонтитул: имя части и её тело.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HeaderFooter {
    /// Идентификатор узла.
    pub id: NodeId,
    /// Имя части в пакете (`word/header1.xml`) — по нему на колонтитул ссылается `w:headerReference`.
    pub part: String,
    /// Тело колонтитула: блоки те же, что и в теле документа, но без секций.
    pub body: Body,
}
