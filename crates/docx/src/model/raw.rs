//! Сырые свойства абзацев и run'ов: ровно то, что было в XML.
//!
//! Каскад стилей на разборе не резолвится (ADR-0013 §1): `RawPPr` и `RawRPr` хранят
//! значения как есть, а какое из них победит — решает раскладка.

use serde::{Deserialize, Serialize};

use super::numbering::NumId;
use super::section::SectionProperties;

/// Двадцать вторых долей пункта (twips): 1/1440 дюйма — единица длины в `WordprocessingML`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(transparent)]
pub struct Twips(i32);

impl Twips {
    /// Длина из значения в twips.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Значение в twips.
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl From<i32> for Twips {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

/// Половинные пункты: размер шрифта `w:sz` = 24 — это 12 pt.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(transparent)]
pub struct HalfPoint(i32);

impl HalfPoint {
    /// Размер из значения в полупунктах.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Значение в полупунктах.
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl From<i32> for HalfPoint {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

/// Идентификатор стиля из XML (`w:pStyle w:val="Heading1"`).
///
/// Служит ключом таблиц стилей (`BTreeMap<StyleId, _>`), поэтому кроме базового набора
/// реализует `Ord`/`Eq`/`Hash` (ADR-0019 §3) — исключение к §0 контракта модели.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct StyleId(String);

impl StyleId {
    /// Идентификатор из строки, как она записана в XML.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Идентификатор строкой — так он и записан в XML.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StyleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for StyleId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for StyleId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Тумблер свойства (`w:b`, `w:keepNext`, …): элемент с `w:val`, но без полезной нагрузки.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Toggle {
    /// Включено; элемент без `w:val` значит то же самое — потому `On` и есть `Default`.
    #[default]
    On,
    /// Выключено явно (`w:val` = `0`, `false`, `off`).
    Off,
    /// Наследуется от стиля (`w:val="inherit"`).
    Inherit,
}

impl Toggle {
    /// Разбирает `w:val` тумблера.
    ///
    /// `None` → `Some(On)` (элемент без `w:val` включён); `"0"`, `"false"`, `"off"` → `Off`;
    /// `"inherit"` → `Inherit`; `"1"`, `"true"`, `"on"` → `On`; прочее → `None`
    /// (парсер обязан выдать `InvalidAttribute` и взять `On`).
    /// Регистр не важен, пробелы по краям обрезаются.
    #[must_use]
    pub fn from_val(value: Option<&str>) -> Option<Self> {
        let Some(raw) = value else {
            return Some(Self::On);
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "on" => Some(Self::On),
            "0" | "false" | "off" => Some(Self::Off),
            "inherit" => Some(Self::Inherit),
            _ => None,
        }
    }
}

/// Цвет в записи файла: `RRGGBB` без альфы.
///
/// У цвета XLSX каналы другие — `AARRGGBB`; приведение делает `xlsx::paint::resolve_color`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    /// Значение `RRGGBB` (например, `FF0000` — красный).
    Rgb(u32),
    /// `w:val="auto"` — цвет выбирает приложение по контексту.
    Auto,
    /// Цвет не задан (`w:val="none"`).
    None,
}

/// Подчёркивание (`w:u`, `ST_Underline`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Underline {
    /// Одна линия.
    Single,
    /// Подчёркнуты только слова.
    Words,
    /// Двойная линия.
    Double,
    /// Утолщённая линия.
    Thick,
    /// Пунктир.
    Dotted,
    /// Утолщённый пунктир.
    DottedHeavy,
    /// Штрих.
    Dash,
    /// Утолщённый штрих.
    DashedHeavy,
    /// Длинный штрих.
    DashLong,
    /// Утолщённый длинный штрих.
    DashLongHeavy,
    /// Штрих-пунктир.
    DotDash,
    /// Утолщённый штрих-пунктир.
    DashDotHeavy,
    /// Штрих с двумя точками.
    DotDotDash,
    /// Утолщённый штрих с двумя точками.
    DashDotDotHeavy,
    /// Волнистая линия.
    Wave,
    /// Утолщённая волнистая линия.
    WavyHeavy,
    /// Двойная волнистая линия.
    WavyDouble,
    /// Подчёркивания нет — явное снятие наследования.
    None,
    /// Значение, которого нет в `ST_Underline`.
    Other(String),
}

/// Цвет выделения (`w:highlight`, `ST_HighlightColor`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Highlight {
    /// Чёрный.
    Black,
    /// Синий.
    Blue,
    /// Голубой.
    Cyan,
    /// Зелёный.
    Green,
    /// Пурпурный.
    Magenta,
    /// Красный.
    Red,
    /// Жёлтый.
    Yellow,
    /// Белый.
    White,
    /// Тёмно-синий.
    DarkBlue,
    /// Тёмно-голубой.
    DarkCyan,
    /// Тёмно-зелёный.
    DarkGreen,
    /// Тёмно-пурпурный.
    DarkMagenta,
    /// Тёмно-красный.
    DarkRed,
    /// Тёмно-жёлтый.
    DarkYellow,
    /// Тёмно-серый.
    DarkGray,
    /// Светло-серый.
    LightGray,
    /// Выделения нет — явное снятие наследования.
    None,
    /// Значение, которого нет в `ST_HighlightColor`.
    Other(String),
}

/// Шрифты run'а (`w:rFonts`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RFonts {
    /// Шрифт латиницы (`w:ascii`).
    pub ascii: Option<String>,
    /// Шрифт `ANSI`-диапазона (`w:hAnsi`); если не задан, берётся `ascii`.
    pub h_ansi: Option<String>,
    /// Шрифт восточноазиатских языков (`w:eastAsia`).
    pub east_asia: Option<String>,
    /// Шрифт языков со сложной раскладкой (`w:cs`).
    pub cs: Option<String>,
    /// Подсказка, каким из шрифтов набирать символ (`w:hint`).
    pub hint: Option<FontHint>,
}

/// Подсказка выбора шрифта по коду символа (`w:hint`, `ST_Hint`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FontHint {
    /// Не задана: приложение само выбирает шрифт по коду символа.
    Default,
    /// Восточноазиатский шрифт.
    EastAsia,
    /// Шрифт языков со сложной раскладкой (`complex script`).
    Cs,
    /// Значение, которого нет в `ST_Hint`.
    Other(String),
}

/// Выравнивание абзаца (`w:jc`, `ST_Jc`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Justification {
    /// По левому краю.
    Left,
    /// По центру.
    Center,
    /// По правому краю.
    Right,
    /// По ширине.
    Both,
    /// По ширине с разрядкой знаков, а не пробелов (восточноазиатский текст).
    Distribute,
    /// По началу строки: `left` для текста слева направо, `right` — для обратного.
    Start,
    /// По концу строки: `right` для текста слева направо, `left` — для обратного.
    End,
    /// Значение, которого нет в `ST_Jc`.
    Other(String),
}

/// Позиция табуляции (`w:tab`): где останавливаться, как выравнивать и чем заполнять.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TabStop {
    /// Позиция от левого поля, в twips (`w:pos`).
    pub val: Twips,
    /// Выравнивание текста относительно позиции (`w:val`).
    pub kind: TabStopKind,
    /// Заполнитель промежутка до позиции (`w:leader`).
    pub leader: TabLeader,
}

/// Выравнивание текста на позиции табуляции (`w:tab w:val`, `ST_TabTlc`/`ST_TabJc`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TabStopKind {
    /// Вертикальная черта в позиции табуляции.
    Bar,
    /// Центрирование текста по позиции.
    Center,
    /// Сброс ранее заданной позиции (`w:val="clear"`).
    Clear,
    /// Выравнивание по десятичному разделителю.
    Decimal,
    /// Выравнивание по концу строки с учётом направления текста.
    End,
    /// Выравнивание по разделителю нумерации (для списков).
    Num,
    /// Выравнивание по началу строки с учётом направления текста.
    Start,
    /// По левому краю.
    Left,
    /// По правому краю.
    Right,
    /// Значение, которого нет в модели.
    Other(String),
}

/// Заполнитель промежутка до позиции табуляции (`w:leader`, `ST_TabTlc`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TabLeader {
    /// Заполнитель не рисуется.
    None,
    /// Точки.
    Dot,
    /// Дефисы.
    Hyphen,
    /// Средние точки (`···`).
    MiddleDot,
    /// Жирная линия.
    Heavy,
    /// Подчёркивание.
    Underscore,
    /// Значение, которого нет в модели.
    Other(String),
}

/// Граница абзаца, ячейки или таблицы (`w:top`, `w:left`, …).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Border {
    /// Стиль линии (`w:val`).
    pub val: BorderStyle,
    /// Толщина в восьмых долях пункта (`w:sz`).
    pub sz: Option<u32>,
    /// Отступ от текста в пунктах (`w:space`).
    pub space: Option<u32>,
    /// Цвет линии (`w:color`).
    pub color: Option<Color>,
}

/// Стиль границы (`w:val` у `w:top`, `w:left`, …, `ST_Border`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BorderStyle {
    /// Границы нет, но формат сохраняется (в отличие от `None`).
    Nil,
    /// Границы нет.
    None,
    /// Одиночная линия.
    Single,
    /// Утолщённая одиночная линия.
    Thick,
    /// Двойная линия.
    Double,
    /// Пунктир.
    Dotted,
    /// Штрих.
    Dashed,
    /// Штрих-пунктир.
    DotDash,
    /// Штрих с двумя точками.
    DotDotDash,
    /// Тройная линия.
    Triple,
    /// Тонкая и толстая линии с малым зазором.
    ThinThickSmallGap,
    /// Толстая и тонкая линии с малым зазором.
    ThickThinSmallGap,
    /// Тонкая, толстая и тонкая линии с малым зазором.
    ThinThickThinSmallGap,
    /// Тонкая и толстая линии со средним зазором.
    ThinThickMediumGap,
    /// Толстая и тонкая линии со средним зазором.
    ThickThinMediumGap,
    /// Тонкая, толстая и тонкая линии со средним зазором.
    ThinThickThinMediumGap,
    /// Тонкая и толстая линии с большим зазором.
    ThinThickLargeGap,
    /// Толстая и тонкая линии с большим зазором.
    ThickThinLargeGap,
    /// Тонкая, толстая и тонкая линии с большим зазором.
    ThinThickThinLargeGap,
    /// Волнистая линия.
    Wave,
    /// Двойная волнистая линия.
    DoubleWave,
    /// Штрих с малым зазором.
    DashSmallGap,
    /// Штрих-пунктир с обводкой.
    DashDotStroked,
    /// Рельефная (выпуклая) линия.
    ThreeDEmboss,
    /// Вдавленная линия.
    ThreeDEngrave,
    /// Линия «наружу».
    Outset,
    /// Линия «внутрь».
    Inset,
    /// Значение, которого нет в `ST_Border`.
    Other(String),
}

impl BorderStyle {
    /// Все варианты без payload — перечень полон по `ST_Border`; `Other` хранит
    /// значение, которого в стандарте нет, и в список не входит.
    pub const ALL: [Self; 27] = [
        Self::Nil,
        Self::None,
        Self::Single,
        Self::Thick,
        Self::Double,
        Self::Dotted,
        Self::Dashed,
        Self::DotDash,
        Self::DotDotDash,
        Self::Triple,
        Self::ThinThickSmallGap,
        Self::ThickThinSmallGap,
        Self::ThinThickThinSmallGap,
        Self::ThinThickMediumGap,
        Self::ThickThinMediumGap,
        Self::ThinThickThinMediumGap,
        Self::ThinThickLargeGap,
        Self::ThickThinLargeGap,
        Self::ThinThickThinLargeGap,
        Self::Wave,
        Self::DoubleWave,
        Self::DashSmallGap,
        Self::DashDotStroked,
        Self::ThreeDEmboss,
        Self::ThreeDEngrave,
        Self::Outset,
        Self::Inset,
    ];
}

/// Заливка (`w:shd`): узор, цвет узора и цвет фона.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Shading {
    /// Узор заливки (`w:val`).
    pub val: ShadingPattern,
    /// Цвет узора (`w:color`).
    pub color: Option<Color>,
    /// Цвет фона (`w:fill`).
    pub fill: Option<Color>,
}

/// Узор заливки (`w:shd w:val`, `ST_Shd`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShadingPattern {
    /// Заливки нет.
    Nil,
    /// Узор «чистый» (фон цвета `w:fill` без штриховки).
    Clear,
    /// Сплошная заливка.
    Solid,
    /// Горизонтальные полосы.
    HorzStripe,
    /// Вертикальные полосы.
    VertStripe,
    /// Обратные диагональные полосы.
    ReverseDiagStripe,
    /// Диагональные полосы.
    DiagStripe,
    /// Горизонтальная штриховка.
    HorzCross,
    /// Диагональная штриховка.
    DiagCross,
    /// Тонкие горизонтальные полосы.
    ThinHorzStripe,
    /// Тонкие вертикальные полосы.
    ThinVertStripe,
    /// Тонкие обратные диагональные полосы.
    ThinReverseDiagStripe,
    /// Тонкие диагональные полосы.
    ThinDiagStripe,
    /// Тонкая горизонтальная штриховка.
    ThinHorzCross,
    /// Тонкая диагональная штриховка.
    ThinDiagCross,
    /// Плотность узора 5 %.
    Pct5,
    /// Плотность узора 10 %.
    Pct10,
    /// Плотность узора 12 %.
    Pct12,
    /// Плотность узора 15 %.
    Pct15,
    /// Плотность узора 20 %.
    Pct20,
    /// Плотность узора 25 %.
    Pct25,
    /// Плотность узора 30 %.
    Pct30,
    /// Плотность узора 35 %.
    Pct35,
    /// Плотность узора 37 %.
    Pct37,
    /// Плотность узора 40 %.
    Pct40,
    /// Плотность узора 45 %.
    Pct45,
    /// Плотность узора 50 %.
    Pct50,
    /// Плотность узора 55 %.
    Pct55,
    /// Плотность узора 60 %.
    Pct60,
    /// Плотность узора 62 %.
    Pct62,
    /// Плотность узора 65 %.
    Pct65,
    /// Плотность узора 70 %.
    Pct70,
    /// Плотность узора 75 %.
    Pct75,
    /// Плотность узора 80 %.
    Pct80,
    /// Плотность узора 85 %.
    Pct85,
    /// Плотность узора 87 %.
    Pct87,
    /// Плотность узора 90 %.
    Pct90,
    /// Плотность узора 95 %.
    Pct95,
    /// Значение, которого нет в `ST_Shd`.
    Other(String),
}

impl ShadingPattern {
    /// Все варианты без payload — перечень полон по `ST_Shd`; `Other` хранит
    /// значение, которого в стандарте нет, и в список не входит.
    pub const ALL: [Self; 38] = [
        Self::Nil,
        Self::Clear,
        Self::Solid,
        Self::HorzStripe,
        Self::VertStripe,
        Self::ReverseDiagStripe,
        Self::DiagStripe,
        Self::HorzCross,
        Self::DiagCross,
        Self::ThinHorzStripe,
        Self::ThinVertStripe,
        Self::ThinReverseDiagStripe,
        Self::ThinDiagStripe,
        Self::ThinHorzCross,
        Self::ThinDiagCross,
        Self::Pct5,
        Self::Pct10,
        Self::Pct12,
        Self::Pct15,
        Self::Pct20,
        Self::Pct25,
        Self::Pct30,
        Self::Pct35,
        Self::Pct37,
        Self::Pct40,
        Self::Pct45,
        Self::Pct50,
        Self::Pct55,
        Self::Pct60,
        Self::Pct62,
        Self::Pct65,
        Self::Pct70,
        Self::Pct75,
        Self::Pct80,
        Self::Pct85,
        Self::Pct87,
        Self::Pct90,
        Self::Pct95,
    ];
}

/// Границы абзаца (`w:pBdr`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ParagraphBorders {
    /// Верхняя граница (`w:top`).
    pub top: Option<Border>,
    /// Левая граница (`w:left`).
    pub left: Option<Border>,
    /// Нижняя граница (`w:bottom`).
    pub bottom: Option<Border>,
    /// Правая граница (`w:right`).
    pub right: Option<Border>,
    /// Граница между абзацами с одинаковыми свойствами (`w:between`).
    pub between: Option<Border>,
    /// Вертикальная черта слева от абзаца (`w:bar`).
    pub bar: Option<Border>,
}

/// Отступы абзаца (`w:ind`), в twips.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Ind {
    /// Отступ слева (`w:left`/`w:start`).
    pub left: Option<Twips>,
    /// Отступ справа (`w:right`/`w:end`).
    pub right: Option<Twips>,
    /// Отступ первой строки (`w:firstLine`).
    pub first_line: Option<Twips>,
    /// Выступ первой строки, влево (`w:hanging`).
    pub hanging: Option<Twips>,
}

/// Ссылка на нумерацию (`w:numPr`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NumPr {
    /// Уровень списка 0..=8 (`w:ilvl`).
    pub ilvl: Option<u8>,
    /// Идентификатор нумерации `w:num` (`w:numId`).
    pub num_id: Option<NumId>,
}

/// `w:spacing` внутри `w:pPr` — интервалы абзаца.
///
/// Отдельный тип, а не общий с [`CharacterSpacing`]: в `w:pPr` и `w:rPr` этот элемент
/// означает совершенно разное (план §3.2 называл одним именем два разных XML-типа).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ParagraphSpacing {
    /// Интервал перед абзацем (`w:before`), в twips.
    pub before: Option<Twips>,
    /// Интервал после абзаца (`w:after`), в twips.
    pub after: Option<Twips>,
    /// Высота строки (`w:line`); единицы зависят от `w:lineRule`.
    pub line: Option<LineSpacing>,
    /// Как понимать `w:line` (`w:lineRule`).
    pub line_rule: Option<LineSpacingRule>,
    /// Интервал перед абзацем в строках (`w:beforeLines`), в сотых долях строки.
    pub before_lines: Option<u32>,
    /// Интервал после абзаца в строках (`w:afterLines`), в сотых долях строки.
    pub after_lines: Option<u32>,
    /// Интервал перед абзацем задаёт приложение (`w:beforeAutospacing`).
    pub before_autospacing: bool,
    /// Интервал после абзаца задаёт приложение (`w:afterAutospacing`).
    pub after_autospacing: bool,
}

/// Значение `w:line`: при `Auto` — 240-е доли строки, при `Exact`/`AtLeast` — twips.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(transparent)]
pub struct LineSpacing(i32);

impl LineSpacing {
    /// Значение `w:line` как есть.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Значение `w:line` как есть.
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl From<i32> for LineSpacing {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

/// Как понимать `w:line` (`w:lineRule`, `ST_LineSpacingRule`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LineSpacingRule {
    /// Множитель строки: значение `w:line` — 240-е доли (240 = одна строка).
    Auto,
    /// Точная высота строки в twips.
    Exact,
    /// Высота строки не меньше значения в twips.
    AtLeast,
    /// Значение, которого нет в `ST_LineSpacingRule`.
    Other(String),
}

/// `w:spacing` внутри `w:rPr` — межбуквенный интервал (`w:val`, `ST_SignedTwipsMeasure`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CharacterSpacing {
    /// Межбуквенный интервал в twips; отрицательное значение — уплотнение.
    pub value: Option<Twips>,
}

/// Выравнивание содержимого ячейки по вертикали (`w:tcPr/w:vAlign`, `ST_VerticalJc`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CellVAlign {
    /// По верхнему краю.
    Top,
    /// По центру.
    Center,
    /// По нижнему краю.
    Bottom,
}

/// Вертикальное смещение знака (`w:rPr/w:vertAlign`, `ST_VerticalAlignRun`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VertAlign {
    /// Обычное положение на базовой линии.
    Baseline,
    /// Верхний индекс.
    Superscript,
    /// Нижний индекс.
    Subscript,
}

/// Сырые свойства абзаца (`w:pPr`) — как в XML, без каскада.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RawPPr {
    /// Стиль абзаца (`w:pStyle`), ещё не разыменованный.
    pub style: Option<StyleId>,
    /// Ссылка на нумерацию (`w:numPr`).
    pub num_pr: Option<NumPr>,
    /// Интервалы абзаца (`w:spacing`).
    pub spacing: Option<ParagraphSpacing>,
    /// Отступы (`w:ind`).
    pub ind: Option<Ind>,
    /// Выравнивание (`w:jc`).
    pub jc: Option<Justification>,
    /// Не отрывать абзац от следующего (`w:keepNext`).
    pub keep_next: Option<Toggle>,
    /// Не разрывать абзац между страницами (`w:keepLines`).
    pub keep_lines: Option<Toggle>,
    /// Начать абзац с новой страницы (`w:pageBreakBefore`).
    pub page_break_before: Option<Toggle>,
    /// Не оставлять одну строку в начале или конце страницы (`w:widowControl`).
    pub widow_control: Option<Toggle>,
    /// Уровень структуры документа 0..=9 (`w:outlineLvl`); в заголовок абзац превращает стиль.
    pub outline_lvl: Option<u8>,
    /// Границы абзаца (`w:pBdr`).
    pub p_bdr: Option<ParagraphBorders>,
    /// Заливка абзаца (`w:shd`).
    pub shd: Option<Shading>,
    /// Позиции табуляции (`w:tabs`), в порядке следования в XML.
    pub tabs: Vec<TabStop>,
    /// Свойства знака абзаца (`w:rPr`).
    pub r_pr: Option<RawRPr>,
    /// Свойства секции (`w:sectPr`); внутри `w:pPr` означает конец секции (ADR-0013 §2).
    pub sect_pr: Option<SectionProperties>,
    /// Нераспознанные дочерние элементы: (локальное имя, XML элемента).
    ///
    /// Элемент сохраняется целиком, а не отбрасывается: раскладка и отладка должны увидеть
    /// неподдержанное свойство, а не молча его потерять (ADR-0014 §3).
    pub unknown: Vec<(String, String)>,
}

/// Сырые свойства знака (`w:rPr`) — как в XML, без каскада.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RawRPr {
    /// Знаковый стиль (`w:rStyle`), ещё не разыменованный.
    pub style: Option<StyleId>,
    /// Шрифты (`w:rFonts`).
    pub r_fonts: Option<RFonts>,
    /// Полужирный (`w:b`).
    pub b: Option<Toggle>,
    /// Курсив (`w:i`).
    pub i: Option<Toggle>,
    /// Все прописные (`w:caps`).
    pub caps: Option<Toggle>,
    /// Капитель (`w:smallCaps`).
    pub small_caps: Option<Toggle>,
    /// Зачёркнутый (`w:strike`).
    pub strike: Option<Toggle>,
    /// Зачёркнутый двойной линией (`w:dstrike`).
    pub dstrike: Option<Toggle>,
    /// Скрытый текст (`w:vanish`).
    pub vanish: Option<Toggle>,
    /// Контурный (`w:outline`).
    pub outline: Option<Toggle>,
    /// С тенью (`w:shadow`).
    pub shadow: Option<Toggle>,
    /// Рельефный (`w:emboss`).
    pub emboss: Option<Toggle>,
    /// Вдавленный (`w:imprint`).
    pub imprint: Option<Toggle>,
    /// Цвет текста (`w:color`).
    pub color: Option<Color>,
    /// Размер шрифта в полупунктах (`w:sz`): 24 — это 12 pt.
    pub sz: Option<HalfPoint>,
    /// Размер шрифта для языков со сложной раскладкой (`w:szCs`).
    pub sz_cs: Option<HalfPoint>,
    /// Выделение цветом (`w:highlight`).
    pub highlight: Option<Highlight>,
    /// Подчёркивание (`w:u`).
    pub u: Option<Underline>,
    /// Вертикальное смещение (`w:vertAlign`).
    pub vert_align: Option<VertAlign>,
    /// Межбуквенный интервал (`w:spacing`).
    pub spacing: Option<CharacterSpacing>,
    /// Смещение от базовой линии в полупунктах (`w:position`).
    pub position: Option<HalfPoint>,
    /// Нераспознанные дочерние элементы: (локальное имя, XML элемента).
    ///
    /// Элемент сохраняется целиком, а не отбрасывается: раскладка и отладка должны увидеть
    /// неподдержанное свойство, а не молча его потерять (ADR-0014 §3).
    pub unknown: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::Toggle;

    #[test]
    fn toggle_from_val_recognizes_all_values() {
        assert_eq!(Toggle::from_val(None), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("1")), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("true")), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("on")), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("0")), Some(Toggle::Off));
        assert_eq!(Toggle::from_val(Some("false")), Some(Toggle::Off));
        assert_eq!(Toggle::from_val(Some("off")), Some(Toggle::Off));
        assert_eq!(Toggle::from_val(Some("inherit")), Some(Toggle::Inherit));
        assert_eq!(Toggle::default(), Toggle::On);
    }

    #[test]
    fn toggle_from_val_ignores_case_and_surrounding_spaces() {
        assert_eq!(Toggle::from_val(Some("True")), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some(" ON ")), Some(Toggle::On));
        assert_eq!(Toggle::from_val(Some("False")), Some(Toggle::Off));
        assert_eq!(Toggle::from_val(Some("Inherit")), Some(Toggle::Inherit));
    }

    #[test]
    fn toggle_from_val_rejects_unknown_values() {
        assert_eq!(Toggle::from_val(Some("yes")), None);
        assert_eq!(Toggle::from_val(Some("2")), None);
        assert_eq!(Toggle::from_val(Some("")), None);
    }
}
