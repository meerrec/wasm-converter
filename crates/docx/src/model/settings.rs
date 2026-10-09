//! Настройки документа `word/settings.xml`: сноски, совместимость и интервалы.

use serde::{Deserialize, Serialize};

use super::numbering::NumFmt;
use super::raw::Twips;

/// Настройки документа; отсутствие `word/settings.xml` даёт значение по умолчанию.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Шаг табуляции по умолчанию (`w:defaultTabStop`).
    pub default_tab_stop: Option<Twips>,
    /// `w:evenAndOddHeaders`: у чётных и нечётных страниц разные колонтитулы.
    pub even_and_odd_headers: bool,
    /// Параметры сносок (`w:footnotePr`).
    pub footnote_pr: FootnotePr,
    /// Параметры концевых сносок (`w:endnotePr`).
    pub endnote_pr: EndnotePr,
    /// Флаги совместимости (`w:compat`).
    pub compat: CompatSettings,
    /// Правила сжатия интервалов (`w:characterSpacingControl`).
    pub character_spacing_control: CharacterSpacingControl,
}

/// Нумерация и положение сносок (`w:footnotePr`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FootnotePr {
    /// Положение сносок (`w:pos`).
    pub pos: Option<FootnotePos>,
    /// Формат номера сноски (`w:numFmt`).
    pub num_fmt: Option<NumFmt>,
    /// Номер первой сноски (`w:numStart`).
    pub num_start: Option<u32>,
    /// Перезапуск нумерации (`w:numRestart`).
    pub num_restart: Option<NumRestart>,
}

/// Нумерация и положение концевых сносок (`w:endnotePr`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EndnotePr {
    /// Положение концевых сносок (`w:pos`).
    pub pos: Option<EndnotePos>,
    /// Формат номера концевой сноски (`w:numFmt`).
    pub num_fmt: Option<NumFmt>,
    /// Номер первой концевой сноски (`w:numStart`).
    pub num_start: Option<u32>,
    /// Перезапуск нумерации (`w:numRestart`).
    pub num_restart: Option<NumRestart>,
}

/// Положение сносок (`w:pos`, `ST_FtnPos`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FootnotePos {
    /// Внизу страницы (умолчание `WordprocessingML`).
    PageBottom,
    /// Сразу под текстом.
    BeneathText,
    /// В конце секции.
    SectionEnd,
    /// В конце документа.
    DocEnd,
    /// Значение, неизвестное парсеру; строка сохраняется как есть.
    Other(String),
}

/// Положение концевых сносок (`w:pos`, `ST_EdnPos`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EndnotePos {
    /// В конце секции (умолчание `WordprocessingML`).
    SectionEnd,
    /// В конце документа.
    DocEnd,
    /// Значение, неизвестное парсеру; строка сохраняется как есть.
    Other(String),
}

/// Перезапуск нумерации (`w:numRestart`, `ST_RestartNumber`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NumRestart {
    /// Нумерация продолжается через весь документ.
    Continuous,
    /// Нумерация начинается заново в каждой секции.
    EachSection,
    /// Нумерация начинается заново на каждой странице.
    EachPage,
    /// Значение, неизвестное парсеру; строка сохраняется как есть.
    Other(String),
}

/// Правила сжатия пунктуации при выключке (`w:characterSpacingControl`, `ST_CharacterSpacing`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CharacterSpacingControl {
    /// Пунктуация не сжимается — умолчание `WordprocessingML`.
    #[default]
    DoNotCompress,
    /// Сжимать знаки пунктуации.
    CompressPunctuation,
    /// Сжимать пунктуацию и японскую кану.
    CompressPunctuationAndJapaneseKana,
    /// Значение, неизвестное парсеру; строка сохраняется как есть.
    Other(String),
}

/// Значимые флаги совместимости `w:compat`; остальные остаются в `unknown`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CompatSettings {
    /// `w:doNotExpandShiftReturn`: мягкий перенос не растягивает строку выключкой.
    pub do_not_expand_shift_return: bool,
    /// `w:doNotUseHTMLParagraphAutoSpacing`: интервалы абзацев считаются по правилам Word.
    pub do_not_use_html_paragraph_auto_spacing: bool,
    /// Ширина таблиц не подгоняется под содержимое автоматически.
    pub do_not_autofit_tables: bool,
    /// `w:doNotBreakWrappedTables`: обёрнутая таблица не разрывается между страницами.
    pub do_not_break_wrapped_tables: bool,
    /// Ячейки с интервалами не выравниваются по вертикали.
    pub do_not_vert_align_in_cell_wi: bool,
    /// `w:doNotUseEastAsianBreakRules`: восточноазиатские правила переноса не применяются.
    pub do_not_use_east_asian_break: bool,
    /// `w:useSingleBorderforContiguousCells`: общая граница соседних ячеек рисуется один раз.
    pub use_single_border_for_contiguous_cells: bool,
    /// Флаги `w:compat` перекрывают свойства табличного стиля.
    pub compat_setting_override_table_style: bool,
    /// Нераспознанные флаги: (локальное имя, XML элемента).
    pub unknown: Vec<(String, String)>,
}
