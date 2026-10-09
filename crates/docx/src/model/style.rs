//! Таблица стилей `word/styles.xml`: стили абзаца, знака, таблицы и нумерации.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::raw::{RawPPr, RawRPr, StyleId};

/// Все стили документа плюс умолчания.
///
/// Таблицы — `BTreeMap`, а не `HashMap`: порядок обхода детерминирован, JSON стабилен для
/// snapshot-тестов (ADR-0019 §7).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct StyleTable {
    /// Свойства по умолчанию (`w:docDefaults`).
    pub doc_defaults: DocDefaults,
    /// Стили абзаца (`w:style w:type="paragraph"`).
    pub paragraph: BTreeMap<StyleId, ParagraphStyle>,
    /// Стили знака (`w:type="character"`).
    pub character: BTreeMap<StyleId, CharacterStyle>,
    /// Стили таблиц (`w:type="table"`).
    pub table: BTreeMap<StyleId, TableStyle>,
    /// Стили нумерации (`w:type="numbering"`).
    pub numbering: BTreeMap<StyleId, NumberingStyle>,
    /// Стили по умолчанию для каждого вида (`w:default="1"`).
    pub defaults: DefaultStyleIds,
}

/// Свойства по умолчанию из `w:docDefaults` — низ каскада стилей.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DocDefaults {
    /// Свойства знака по умолчанию (`w:rPrDefault`).
    pub r_pr: RawRPr,
    /// Свойства абзаца по умолчанию (`w:pPrDefault`).
    pub p_pr: RawPPr,
}

/// Стиль абзаца.
///
/// Стили адресуются по [`StyleId`], `NodeId` им не присваивается (ADR-0019 §3), поэтому поле `id`
/// здесь — именно идентификатор стиля, а не узел модели.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ParagraphStyle {
    /// Идентификатор стиля (`w:styleId`).
    pub id: StyleId,
    /// Отображаемое имя (`w:name w:val`).
    pub name: Option<String>,
    /// Стиль-основа (`w:basedOn`) — с него продолжается каскад (ADR-0013 §3).
    pub based_on: Option<StyleId>,
    /// Стиль следующего абзаца по Enter (`w:next`); в цепочке `basedOn` не участвует.
    pub next: Option<StyleId>,
    /// Связанный стиль другого вида (`w:link`).
    pub link: Option<StyleId>,
    /// `w:default="1"`: стиль по умолчанию для абзацев.
    pub is_default: bool,
    /// `w:semiHidden`/`w:hidden`: стиль не показывается в списке Word.
    pub hidden: bool,
    /// `w:customStyle`: стиль создан пользователем, а не поставляется с шаблоном.
    pub custom: bool,
    /// Синонимы имени (`w:aliases`).
    pub aliases: Vec<String>,
    /// Свойства абзаца стиля.
    pub ppr: RawPPr,
    /// Свойства знака стиля.
    pub rpr: RawRPr,
}

/// Стиль знака (`w:style w:type="character"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CharacterStyle {
    /// Идентификатор стиля (`w:styleId`).
    pub id: StyleId,
    /// Отображаемое имя (`w:name w:val`).
    pub name: Option<String>,
    /// Стиль-основа (`w:basedOn`).
    pub based_on: Option<StyleId>,
    /// Связанный стиль абзаца (`w:link`).
    pub link: Option<StyleId>,
    /// `w:default="1"`: стиль знака по умолчанию.
    pub is_default: bool,
    /// `w:semiHidden`/`w:hidden`.
    pub hidden: bool,
    /// `w:customStyle`.
    pub custom: bool,
    /// Синонимы имени (`w:aliases`).
    pub aliases: Vec<String>,
    /// Свойства знака стиля.
    pub rpr: RawRPr,
}

/// Стиль таблицы (`w:style w:type="table"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TableStyle {
    /// Идентификатор стиля (`w:styleId`).
    pub id: StyleId,
    /// Отображаемое имя (`w:name w:val`).
    pub name: Option<String>,
    /// Стиль-основа (`w:basedOn`).
    pub based_on: Option<StyleId>,
    /// `w:default="1"`: стиль таблиц по умолчанию.
    pub is_default: bool,
    /// `w:semiHidden`/`w:hidden`.
    pub hidden: bool,
    /// `w:customStyle`.
    pub custom: bool,
    /// Синонимы имени (`w:aliases`).
    pub aliases: Vec<String>,
    /// Свойства абзаца, применяемые к ячейкам стиля.
    pub ppr: RawPPr,
    /// Свойства знака, применяемые к тексту ячеек стиля.
    pub rpr: RawRPr,
    /// Условные форматы для строк, столбцов, полос и угловых ячеек (`w:tblStylePr`).
    pub conditional: Vec<ConditionalFormat>,
}

/// Условный формат табличного стиля (`w:tblStylePr`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConditionalFormat {
    /// Область применения условия.
    pub kind: TableStyleCondition,
    /// Свойства абзаца для этой области.
    pub ppr: RawPPr,
    /// Свойства знака для этой области.
    pub rpr: RawRPr,
}

/// Условие применения формата табличного стиля (`w:type`, `ST_TblStyleOverrideType`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TableStyleCondition {
    /// Вся таблица целиком.
    WholeTable,
    /// Первая строка.
    FirstRow,
    /// Последняя строка.
    LastRow,
    /// Первый столбец.
    FirstCol,
    /// Последний столбец.
    LastCol,
    /// Полоса нечётных столбцов.
    Band1Vert,
    /// Полоса чётных столбцов.
    Band2Vert,
    /// Полоса нечётных строк.
    Band1Horz,
    /// Полоса чётных строк.
    Band2Horz,
    /// Северо-восточная угловая ячейка.
    NeCell,
    /// Северо-западная угловая ячейка.
    NwCell,
    /// Юго-восточная угловая ячейка.
    SeCell,
    /// Юго-западная угловая ячейка.
    SwCell,
    /// Значение, неизвестное парсеру; строка сохраняется как есть.
    Other(String),
}

impl TableStyleCondition {
    /// Все варианты без payload — перечень полон, `Other` вне списка.
    ///
    /// Нужен round-trip-тесту: он прогоняет через serde каждое условие.
    pub const ALL: [Self; 13] = [
        Self::WholeTable,
        Self::FirstRow,
        Self::LastRow,
        Self::FirstCol,
        Self::LastCol,
        Self::Band1Vert,
        Self::Band2Vert,
        Self::Band1Horz,
        Self::Band2Horz,
        Self::NeCell,
        Self::NwCell,
        Self::SeCell,
        Self::SwCell,
    ];
}

/// Стиль нумерации (`w:style w:type="numbering"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NumberingStyle {
    /// Идентификатор стиля (`w:styleId`).
    pub id: StyleId,
    /// Отображаемое имя (`w:name w:val`).
    pub name: Option<String>,
    /// Стиль-основа (`w:basedOn`).
    pub based_on: Option<StyleId>,
    /// `w:default="1"`: стиль нумерации по умолчанию.
    pub is_default: bool,
    /// `w:semiHidden`/`w:hidden`.
    pub hidden: bool,
    /// `w:customStyle`.
    pub custom: bool,
    /// Синонимы имени (`w:aliases`).
    pub aliases: Vec<String>,
    /// Свойства абзаца стиля.
    pub ppr: RawPPr,
    /// Свойства знака стиля.
    pub rpr: RawRPr,
}

/// Идентификаторы стилей, помеченных `w:default="1"`, по видам стилей.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DefaultStyleIds {
    /// Стиль абзаца по умолчанию.
    pub paragraph: Option<StyleId>,
    /// Стиль знака по умолчанию.
    pub character: Option<StyleId>,
    /// Стиль таблицы по умолчанию.
    pub table: Option<StyleId>,
    /// Стиль нумерации по умолчанию.
    pub numbering: Option<StyleId>,
}

#[cfg(test)]
mod tests {
    use super::TableStyleCondition;

    /// Полнота `ALL` важна round-trip-тесту: пропущенный вариант не пройдёт через serde.
    #[test]
    fn all_table_style_conditions_are_distinct_and_complete() {
        let all = TableStyleCondition::ALL;
        assert_eq!(all.len(), 13);
        for (index, variant) in all.iter().enumerate() {
            for other in all.iter().skip(index + 1) {
                assert_ne!(
                    variant, other,
                    "дубликат в TableStyleCondition::ALL: {variant:?}"
                );
            }
        }
    }
}
