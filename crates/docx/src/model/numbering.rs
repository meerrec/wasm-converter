//! Нумерация `word/numbering.xml`: абстрактные схемы, конкретные списки и уровни.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::raw::{Justification, RawPPr, RawRPr, StyleId};

/// Идентификатор абстрактной схемы нумерации (`w:abstractNumId`).
///
/// Ключ `BTreeMap`: `Ord` нужен для детерминированного обхода таблицы нумерации (ADR-0019 §7).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct AbstractNumId(
    /// Значение `w:abstractNumId`; уникально в пределах `word/numbering.xml`.
    pub u32,
);

impl AbstractNumId {
    /// Создаёт идентификатор из числа.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Возвращает число.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Идентификатор нумерации (`w:numId`): ссылка абзаца на конкретный список.
///
/// Ключ `BTreeMap`: `Ord` нужен для детерминированного обхода таблицы нумерации (ADR-0019 §7).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct NumId(
    /// Значение `w:numId`; `0` в `WordprocessingML` означает «нумерация снята».
    pub u32,
);

impl NumId {
    /// Создаёт идентификатор из числа.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Возвращает число.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Таблица нумерации: схемы и ссылающиеся на них списки.
///
/// `BTreeMap`, а не `HashMap`: детерминированный JSON для snapshot-тестов (ADR-0019 §7).
/// Пустая таблица осмысленна: документ без `word/numbering.xml` — обычное дело.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NumberingTable {
    /// Абстрактные схемы уровней (`w:abstractNum`), ключ — `w:abstractNumId`.
    pub abstract_nums: BTreeMap<AbstractNumId, AbstractNum>,
    /// Конкретные списки (`w:num`), ключ — `w:numId`.
    pub nums: BTreeMap<NumId, Num>,
}

/// Абстрактная схема нумерации (`w:abstractNum`): уровни, общие для нескольких списков.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AbstractNum {
    /// Идентификатор схемы (`w:abstractNumId`).
    pub id: AbstractNumId,
    /// Вид многоуровневости (`w:multiLevelType`).
    pub multi_level_type: MultiLevelType,
    /// Уровни схемы, ключ — `w:ilvl` (0–8).
    pub levels: BTreeMap<u8, Lvl>,
    /// Ссылка на связанный стиль нумерации (`w:numStyleLink`).
    pub num_style_link: Option<StyleId>,
    /// Ссылка на стиль абзаца, для которого схема задана (`w:styleLink`).
    pub style_link: Option<StyleId>,
    /// Идентификатор набора настроек (`w:nsid`), по нему Word находит схему после правок.
    pub nsid: Option<String>,
    /// Идентификатор шаблона (`w:tmpl`) для сопоставления копий документа.
    pub tmpl: Option<String>,
}

/// Конкретный список (`w:num`): ссылка на схему плюс переопределения уровней.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Num {
    /// Идентификатор списка (`w:numId`).
    pub id: NumId,
    /// Схема, на которую ссылается список (`w:abstractNumId`).
    pub abstract_id: AbstractNumId,
    /// Переопределения уровней (`w:lvlOverride`), ключ — `w:ilvl`.
    pub overrides: BTreeMap<u8, LvlOverride>,
    /// Идентификатор картинки-маркера (`w:lvlPicBulletId`).
    pub picture_bullet_id: Option<u32>,
}

/// Переопределение уровня в конкретном списке (`w:lvlOverride`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LvlOverride {
    /// Новый начальный номер (`w:startOverride`).
    pub start_override: Option<u32>,
    /// Полная замена уровня (`w:lvl`) вместо уровня из схемы.
    pub lvl: Option<Lvl>,
}

/// Уровень нумерации (`w:lvl`) схемы или переопределения.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lvl {
    /// Номер уровня (`w:ilvl`, 0–8); дублирует ключ в `BTreeMap` и остаётся для сериализации.
    pub ilvl: u8,
    /// Начальный номер (`w:start`, по умолчанию 1).
    pub start: u32,
    /// Формат номера (`w:numFmt`).
    pub num_fmt: NumFmt,
    /// Шаблон текста маркера (`w:lvlText`), например `%1.` — подстановка номера уровня 1.
    pub lvl_text: String,
    /// Выравнивание маркера (`w:lvlJc`).
    pub lvl_jc: Option<Justification>,
    /// Символ после маркера (`w:suff`).
    pub suff: LevelSuffix,
    /// Свойства абзаца уровня.
    pub ppr: RawPPr,
    /// Свойства знака маркера.
    pub rpr: RawRPr,
    /// Уровень, после которого нумерация начинается заново (`w:lvlRestart`).
    pub restart: Option<u8>,
    /// Стиль абзаца, привязанный к уровню (`w:pStyle`).
    pub pstyle: Option<StyleId>,
    /// `w:isLgl`: номер уровня выводится в десятичном виде независимо от `num_fmt`.
    pub is_lgl: bool,
    /// Идентификатор картинки-маркера (`w:lvlPicBulletId`).
    pub pic_bullet_id: Option<u32>,
    /// `w:tentative`: уровень не применялся в документе и может быть изменён Word.
    pub tentative: bool,
}

/// Символ после номера списка (`w:suff`, `ST_LevelSuffix`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LevelSuffix {
    /// Табуляция (умолчание `WordprocessingML`).
    Tab,
    /// Пробел.
    Space,
    /// Ничего: текст идёт сразу за номером.
    Nothing,
}

/// Вид многоуровневости схемы (`w:multiLevelType`, `ST_MultiLevelType`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MultiLevelType {
    /// Один уровень.
    SingleLevel,
    /// Многоуровневый список, уровни заданы независимо.
    Multilevel,
    /// Гибридный многоуровневый список.
    HybridMultilevel,
}

/// Формат номера списка (`w:numFmt`, `ST_NumberFormat` ECMA-376).
///
/// Перечень повторяет стандарт целиком: сокращать его нельзя, иначе round-trip потеряет формат.
/// Значения представлены так, как их записывает Word, `Other` хранит всё незнакомое.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NumFmt {
    /// Арабские числа: 1, 2, 3.
    Decimal,
    /// Римские числа в верхнем регистре: I, II, III.
    UpperRoman,
    /// Римские числа в нижнем регистре.
    LowerRoman,
    /// Латинские буквы в верхнем регистре: A, B, C.
    UpperLetter,
    /// Латинские буквы в нижнем регистре.
    LowerLetter,
    /// Порядковые числительные: 1st, 2nd, 3rd.
    Ordinal,
    /// Количественные числительные словами: one, two.
    CardinalText,
    /// Порядковые числительные словами: first, second.
    OrdinalText,
    /// Шестнадцатеричные числа.
    Hex,
    /// Нумерация по правилам Chicago Manual of Style: звёздочки, крестики, буквы.
    Chicago,
    /// Идеографы-цифры: китайские и японские цифры.
    IdeographDigital,
    /// Японская счётная нумерация: ити, ни, сан.
    JapaneseCounting,
    /// Японская слоговая нумерация айуэо в порядке годзюон.
    Aiueo,
    /// Японская слоговая нумерация ироха.
    Iroha,
    /// Арабские цифры полной ширины (восточноазиатская ширина).
    DecimalFullWidth,
    /// Арабские цифры половинной ширины.
    DecimalHalfWidth,
    /// Японская юридическая нумерация.
    JapaneseLegal,
    /// Японские цифры с разрядом десять тысяч.
    JapaneseDigitalTenThousand,
    /// Арабские цифры в кружке: ①, ②, ③.
    DecimalEnclosedCircle,
    /// Арабские цифры полной ширины: вариант 2.
    DecimalFullWidth2,
    /// Слоговая нумерация айуэо полной ширины.
    AiueoFullWidth,
    /// Слоговая нумерация ироха полной ширины.
    IrohaFullWidth,
    /// Арабские цифры с ведущим нулём: 01, 02.
    DecimalZero,
    /// Маркер списка: символ берётся из `w:lvlText`, а не из счётчика.
    Bullet,
    /// Корейские буквы канада: 가, 나, 다.
    Ganada,
    /// Корейские начальные согласные.
    Chosung,
    /// Арабская цифра с точкой: 1., 2.
    DecimalEnclosedFullstop,
    /// Арабская цифра в скобках: (1), (2).
    DecimalEnclosedParen,
    /// Арабская цифра в кружке в китайской традиции.
    DecimalEnclosedCircleChinese,
    /// Идеограф в кружке.
    IdeographEnclosedCircle,
    /// Традиционные идеографы небесных стволов: 甲, 乙.
    IdeographTraditional,
    /// Идеографы двенадцати зодиакальных животных.
    IdeographZodiac,
    /// Традиционные идеографы зодиака.
    IdeographZodiacTraditional,
    /// Тайваньская счётная нумерация.
    TaiwaneseCounting,
    /// Традиционные идеографы для юридических документов.
    IdeographLegalTraditional,
    /// Тайваньская счётная нумерация с разрядом тысяч.
    TaiwaneseCountingThousand,
    /// Тайваньские цифры-идеографы.
    TaiwaneseDigital,
    /// Китайская счётная нумерация.
    ChineseCounting,
    /// Упрощённые китайские идеографы для юридических документов.
    ChineseLegalSimplified,
    /// Китайская счётная нумерация с разрядом тысяч.
    ChineseCountingThousand,
    /// Корейские цифры-идеографы.
    KoreanDigital,
    /// Корейская счётная нумерация.
    KoreanCounting,
    /// Корейские идеографы для юридических документов.
    KoreanLegal,
    /// Корейские цифры-идеографы: вариант 2.
    KoreanDigital2,
    /// Вьетнамская счётная нумерация.
    VietnameseCounting,
    /// Русские числительные словами в нижнем регистре: один, два.
    RussianLower,
    /// Русские числительные словами в верхнем регистре.
    RussianUpper,
    /// Нумерация отсутствует: маркер не выводится.
    None,
    /// Число в тире: - 1 -, - 2 -.
    NumberInDash,
    /// Еврейские буквы-числительные.
    Hebrew1,
    /// Еврейские буквы-числительные: вариант 2.
    Hebrew2,
    /// Буквы арабского алфавита.
    ArabicAlpha,
    /// Арабская абджадия: числовые значения букв.
    ArabicAbjad,
    /// Гласные деванагари.
    HindiVowels,
    /// Согласные деванагари.
    HindiConsonants,
    /// Цифры деванагари.
    HindiNumbers,
    /// Счётная нумерация на деванагари.
    HindiCounting,
    /// Буквы тайского алфавита.
    ThaiLetters,
    /// Тайские цифры.
    ThaiNumbers,
    /// Тайская счётная нумерация.
    ThaiCounting,
    /// Числительные словами по-тайски с денежной единицей бат.
    BahtText,
    /// Числительные словами с денежной единицей доллар.
    DollarText,
    /// Пользовательский формат, заданный шаблоном `w:lvlText`.
    Custom,
    /// Значение, неизвестное парсеру; строка сохраняется как есть.
    Other(String),
}

impl NumFmt {
    /// Разбирает значение `w:numFmt` (`ST_NumberFormat`).
    ///
    /// Написание — имя варианта со строчной первой буквой (`upperRoman`); незнакомое
    /// значение сохраняется в [`NumFmt::Other`]. Единственная таблица соответствия
    /// в крейте: и `numbering.xml`, и `settings.xml` разбирают через неё.
    #[must_use]
    pub fn from_ooxml(value: &str) -> Self {
        match value.trim() {
            "decimal" => Self::Decimal,
            "upperRoman" => Self::UpperRoman,
            "lowerRoman" => Self::LowerRoman,
            "upperLetter" => Self::UpperLetter,
            "lowerLetter" => Self::LowerLetter,
            "ordinal" => Self::Ordinal,
            "cardinalText" => Self::CardinalText,
            "ordinalText" => Self::OrdinalText,
            "hex" => Self::Hex,
            "chicago" => Self::Chicago,
            "ideographDigital" => Self::IdeographDigital,
            "japaneseCounting" => Self::JapaneseCounting,
            "aiueo" => Self::Aiueo,
            "iroha" => Self::Iroha,
            "decimalFullWidth" => Self::DecimalFullWidth,
            "decimalHalfWidth" => Self::DecimalHalfWidth,
            "japaneseLegal" => Self::JapaneseLegal,
            "japaneseDigitalTenThousand" => Self::JapaneseDigitalTenThousand,
            "decimalEnclosedCircle" => Self::DecimalEnclosedCircle,
            "decimalFullWidth2" => Self::DecimalFullWidth2,
            "aiueoFullWidth" => Self::AiueoFullWidth,
            "irohaFullWidth" => Self::IrohaFullWidth,
            "decimalZero" => Self::DecimalZero,
            "bullet" => Self::Bullet,
            "ganada" => Self::Ganada,
            "chosung" => Self::Chosung,
            "decimalEnclosedFullstop" => Self::DecimalEnclosedFullstop,
            "decimalEnclosedParen" => Self::DecimalEnclosedParen,
            "decimalEnclosedCircleChinese" => Self::DecimalEnclosedCircleChinese,
            "ideographEnclosedCircle" => Self::IdeographEnclosedCircle,
            "ideographTraditional" => Self::IdeographTraditional,
            "ideographZodiac" => Self::IdeographZodiac,
            "ideographZodiacTraditional" => Self::IdeographZodiacTraditional,
            "taiwaneseCounting" => Self::TaiwaneseCounting,
            "ideographLegalTraditional" => Self::IdeographLegalTraditional,
            "taiwaneseCountingThousand" => Self::TaiwaneseCountingThousand,
            "taiwaneseDigital" => Self::TaiwaneseDigital,
            "chineseCounting" => Self::ChineseCounting,
            "chineseLegalSimplified" => Self::ChineseLegalSimplified,
            "chineseCountingThousand" => Self::ChineseCountingThousand,
            "koreanDigital" => Self::KoreanDigital,
            "koreanCounting" => Self::KoreanCounting,
            "koreanLegal" => Self::KoreanLegal,
            "koreanDigital2" => Self::KoreanDigital2,
            "vietnameseCounting" => Self::VietnameseCounting,
            "russianLower" => Self::RussianLower,
            "russianUpper" => Self::RussianUpper,
            "none" => Self::None,
            "numberInDash" => Self::NumberInDash,
            "hebrew1" => Self::Hebrew1,
            "hebrew2" => Self::Hebrew2,
            "arabicAlpha" => Self::ArabicAlpha,
            "arabicAbjad" => Self::ArabicAbjad,
            "hindiVowels" => Self::HindiVowels,
            "hindiConsonants" => Self::HindiConsonants,
            "hindiNumbers" => Self::HindiNumbers,
            "hindiCounting" => Self::HindiCounting,
            "thaiLetters" => Self::ThaiLetters,
            "thaiNumbers" => Self::ThaiNumbers,
            "thaiCounting" => Self::ThaiCounting,
            "bahtText" => Self::BahtText,
            "dollarText" => Self::DollarText,
            "custom" => Self::Custom,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Все варианты без payload — перечень полон, `Other` вне списка.
    ///
    /// Нужен round-trip-тесту: он прогоняет через serde каждый формат.
    pub const ALL: [Self; 63] = [
        Self::Decimal,
        Self::UpperRoman,
        Self::LowerRoman,
        Self::UpperLetter,
        Self::LowerLetter,
        Self::Ordinal,
        Self::CardinalText,
        Self::OrdinalText,
        Self::Hex,
        Self::Chicago,
        Self::IdeographDigital,
        Self::JapaneseCounting,
        Self::Aiueo,
        Self::Iroha,
        Self::DecimalFullWidth,
        Self::DecimalHalfWidth,
        Self::JapaneseLegal,
        Self::JapaneseDigitalTenThousand,
        Self::DecimalEnclosedCircle,
        Self::DecimalFullWidth2,
        Self::AiueoFullWidth,
        Self::IrohaFullWidth,
        Self::DecimalZero,
        Self::Bullet,
        Self::Ganada,
        Self::Chosung,
        Self::DecimalEnclosedFullstop,
        Self::DecimalEnclosedParen,
        Self::DecimalEnclosedCircleChinese,
        Self::IdeographEnclosedCircle,
        Self::IdeographTraditional,
        Self::IdeographZodiac,
        Self::IdeographZodiacTraditional,
        Self::TaiwaneseCounting,
        Self::IdeographLegalTraditional,
        Self::TaiwaneseCountingThousand,
        Self::TaiwaneseDigital,
        Self::ChineseCounting,
        Self::ChineseLegalSimplified,
        Self::ChineseCountingThousand,
        Self::KoreanDigital,
        Self::KoreanCounting,
        Self::KoreanLegal,
        Self::KoreanDigital2,
        Self::VietnameseCounting,
        Self::RussianLower,
        Self::RussianUpper,
        Self::None,
        Self::NumberInDash,
        Self::Hebrew1,
        Self::Hebrew2,
        Self::ArabicAlpha,
        Self::ArabicAbjad,
        Self::HindiVowels,
        Self::HindiConsonants,
        Self::HindiNumbers,
        Self::HindiCounting,
        Self::ThaiLetters,
        Self::ThaiNumbers,
        Self::ThaiCounting,
        Self::BahtText,
        Self::DollarText,
        Self::Custom,
    ];
}

#[cfg(test)]
mod tests {
    use super::NumFmt;

    /// Полнота `ALL` важна round-trip-тесту: пропущенный вариант не пройдёт через serde.
    #[test]
    fn all_num_fmts_are_distinct_and_complete() {
        let all = NumFmt::ALL;
        assert_eq!(all.len(), 63);
        for (index, variant) in all.iter().enumerate() {
            for other in all.iter().skip(index + 1) {
                assert_ne!(variant, other, "дубликат в NumFmt::ALL: {variant:?}");
            }
        }
    }
}
