//! Абзац, inline-содержимое и run — строительные блоки тела документа.

use serde::{Deserialize, Serialize};

use doc_converter_core::NodeId;

use super::drawing::InlineOrAnchor;
use super::numbering::NumId;
use super::raw::{RawPPr, RawRPr, StyleId};
use super::section::SectionProperties;

/// Абзац: свойства, содержимое и, если абзац закрывает секцию, её `w:sectPr`.
///
/// `style_ref` и `numbering_ref` дублируют то, что уже лежит в `ppr`: раскладке они нужны
/// чаще прочих свойств, а какой стиль победит — всё равно решает каскад (ADR-0013 §1).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    /// Идентификатор узла в модели.
    pub id: NodeId,
    /// Сырые свойства абзаца (`w:pPr`).
    pub ppr: RawPPr,
    /// Свойства знака абзаца (`w:rPr` внутри `w:pPr`); к runs не применяются (ADR-0013 §2).
    pub mark_rpr: RawRPr,
    /// Содержимое абзаца по порядку.
    pub runs: Vec<Inline>,
    /// Идентификатор стиля абзаца (`w:pStyle`) — копия `ppr.style`.
    pub style_ref: Option<StyleId>,
    /// Нумерация абзаца (`w:numId`) — копия `ppr.num_pr.num_id`.
    pub numbering_ref: Option<NumId>,
    /// `w:sectPr` внутри `w:pPr`: конец секции, а не её начало.
    pub section_break: Option<SectionProperties>,
}

/// Inline-содержимое абзаца.
///
/// Именно enum, а не «run с флагами» (требование `DoD` спринта): у ссылки, закладки и поля своё
/// содержимое и своя вложенность, а run с флагами пришлось бы проверять вручную.
// Модель — замороженный контракт: `Box` изменил бы вид вариантов у всех потребителей,
// а `Run` и `InlineOrAnchor` заведомо тяжелее остальных.
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Inline {
    /// Run с форматированием и содержимым (`w:r`).
    Run(Run),
    /// Гиперссылка (`w:hyperlink`).
    Hyperlink(Hyperlink),
    /// Начало закладки (`w:bookmarkStart`).
    Bookmark(Bookmark),
    /// Поле — простое или составное.
    Field(Field),
    /// Разрыв строки, страницы или колонки (`w:br`).
    Break(BreakKind),
    /// Табуляция (`w:tab`).
    Tab,
    /// Символ из шрифта-символов (`w:sym`), а не обычный текст.
    Symbol {
        /// Имя шрифта-символов (`w:font`, например `Wingdings`).
        font: String,
        /// Код символа в этом шрифте (`w:char`).
        char: char,
    },
    /// Рисунок: встроенный или плавающий (`w:drawing`).
    Drawing(InlineOrAnchor),
    /// Неподдержанный элемент: (id, XML) — сохраняется для отладки (ADR-0014 §3).
    Unknown {
        /// Идентификатор узла в модели.
        id: NodeId,
        /// Исходный XML элемента целиком.
        xml: String,
    },
}

/// Run (`w:r`): свойства знака и содержимое.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Run {
    /// Идентификатор узла в модели.
    pub id: NodeId,
    /// Сырые свойства знака (`w:rPr`).
    pub rpr: RawRPr,
    /// Идентификатор знакового стиля (`w:rStyle`) — копия `rpr.style`.
    pub style_ref: Option<StyleId>,
    /// Содержимое run'а по порядку.
    pub content: Vec<RunContent>,
}

/// Элемент содержимого run'а.
// Модель — замороженный контракт, см. обоснование у [`Inline`].
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunContent {
    /// Текст (`w:t`); его форматирование берётся из `rPr` run'а.
    Text(String),
    /// Табуляция (`w:tab`).
    Tab,
    /// Разрыв строки, страницы или колонки (`w:br`).
    Break(BreakKind),
    /// Символ из шрифта-символов (`w:sym`).
    Symbol {
        /// Имя шрифта-символов (`w:font`, например `Wingdings`).
        font: String,
        /// Код символа в этом шрифте (`w:char`).
        char: char,
    },
    /// Рисунок: встроенный или плавающий (`w:drawing`).
    Drawing(InlineOrAnchor),
    /// Неподдержанный элемент: (id, XML).
    Unknown {
        /// Идентификатор узла в модели.
        id: NodeId,
        /// Исходный XML элемента целиком.
        xml: String,
    },
}

/// Вид разрыва (`w:br w:type`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    /// Разрыв строки; так же читается `w:br` без `w:type`.
    Line,
    /// Разрыв страницы.
    Page,
    /// Разрыв колонки.
    Column,
    /// Вид разрыва, которого нет в модели; значение атрибута сохраняется как есть.
    Unsupported(String),
}

/// Гиперссылка (`w:hyperlink`): её runs наследуют цель ссылки.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Hyperlink {
    /// Идентификатор узла в модели.
    pub id: NodeId,
    /// Идентификатор отношения с целью ссылки (`r:id`).
    pub rel_id: Option<String>,
    /// Закладка внутри документа (`w:anchor`).
    pub anchor: Option<String>,
    /// Подсказка при наведении (`w:tooltip`).
    pub tooltip: Option<String>,
    /// Адрес цели, разрешённый через relationships; `None`, пока связь не разрешена.
    pub target: Option<String>,
    /// Ссылка ведёт за пределы документа: задана `r:id`, а не `w:anchor`.
    pub external: bool,
    /// Содержимое ссылки.
    pub runs: Vec<Inline>,
}

/// Закладка: создаётся из `w:bookmarkStart`.
///
/// `w:bookmarkEnd` без начала даёт предупреждение `OrphanBookmark` и в модель не попадает.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bookmark {
    /// Идентификатор узла в модели.
    pub id: NodeId,
    /// Имя закладки (`w:name`); на него ссылается `w:hyperlink w:anchor`.
    pub name: String,
    /// Числовой идентификатор пары `w:bookmarkStart`/`w:bookmarkEnd` (`w:id`).
    pub bookmark_id: i64,
}

/// Вид поля.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Простое поле `w:fldSimple`: инструкция в атрибуте `w:instr`.
    Simple,
    /// Составное поле: `w:fldChar` от `begin` до `end`; `separate` делит инструкцию и результат.
    Complex,
}

/// Поле — простое (`w:fldSimple`) или составное (`w:fldChar`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Field {
    /// Идентификатор узла в модели.
    pub id: NodeId,
    /// Простое поле или составное.
    pub kind: FieldKind,
    /// Инструкция поля — текст `w:instrText`, например `PAGE`.
    pub instruction: String,
    /// Результат поля: то, что видно в документе.
    pub result: Vec<Inline>,
    /// `w:dirty`: результат устарел и требует пересчёта.
    pub dirty: bool,
}
