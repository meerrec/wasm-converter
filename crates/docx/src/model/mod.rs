//! Нормализованная модель документа DOCX.
//!
//! Типы повторяют `WordprocessingML` (`w:p`, `w:tbl`, `w:sectPr`), но каскад
//! стилей на разборе не разрешается: сырые свойства абзаца и знака достаются
//! потребителю ровно в том виде, в каком были в XML (ADR-0013). Наполняют
//! модель по частям парсеры слайсов S6–S12.

pub mod annotation;
pub mod drawing;
pub mod inline;
pub mod numbering;
pub mod raw;
pub mod section;
pub mod settings;
pub mod style;
pub mod table;
pub mod validate;

pub use annotation::{Comment, Footnote, NoteKind};
pub use drawing::{
    AlignH, AlignV, Anchor, Extent, InlineImage, InlineOrAnchor, PositionH, PositionV, RelFromH,
    RelFromV, WrapKind,
};
pub use inline::{
    Bookmark, BreakKind, Field, FieldKind, Hyperlink, Inline, Paragraph, Run, RunContent,
};
pub use numbering::{
    AbstractNum, AbstractNumId, LevelSuffix, Lvl, LvlOverride, MultiLevelType, Num, NumFmt, NumId,
    NumberingTable,
};
pub use raw::{
    Border, BorderStyle, CellVAlign, CharacterSpacing, Color, FontHint, HalfPoint, Highlight, Ind,
    Justification, LineSpacing, LineSpacingRule, NumPr, ParagraphBorders, ParagraphSpacing, RFonts,
    RawPPr, RawRPr, Shading, ShadingPattern, StyleId, TabLeader, TabStop, TabStopKind, Toggle,
    Twips, Underline, VertAlign,
};
pub use section::{
    ColumnDef, Columns, HeaderFooter, Margins, Orientation, PageSize, PartRef, Section,
    SectionProperties, SectionType,
};
pub use settings::{
    CharacterSpacingControl, CompatSettings, EndnotePos, EndnotePr, FootnotePos, FootnotePr,
    NumRestart, Settings,
};
pub use style::{
    CharacterStyle, ConditionalFormat, DefaultStyleIds, DocDefaults, NumberingStyle,
    ParagraphStyle, StyleTable, TableStyle, TableStyleCondition,
};
pub use table::{
    Cell, CellBorders, CellMargins, CellWidth, GridCol, HeightRule, Row, RowHeight, Table,
    TableBorders, TableLayout, TableLook, TableWidth, VMerge,
};

use std::collections::BTreeMap;

use doc_converter_core::rels::{RelMap, Relationship};
use doc_converter_core::{NodeId, ParseWarning, WarningKind, Warnings};
use serde::{Deserialize, Serialize};

/// Разобранный документ DOCX.
///
/// Собирается один раз и дальше только читается; ссылки между частями (стили,
/// нумерация, колонтитулы) разрешены на разборе, а восстановимые нарушения
/// собраны в `warnings`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Document {
    /// Идентификатор корневого узла документа.
    pub id: NodeId,
    /// Тело документа: блоки и секции.
    pub body: Body,
    /// Таблица стилей из `word/styles.xml`.
    pub styles: StyleTable,
    /// Абстрактные нумерации и их экземпляры из `word/numbering.xml`.
    pub numbering: NumberingTable,
    /// Параметры документа из `word/settings.xml`.
    pub settings: Settings,
    /// Свойства пакета из `docProps/core.xml` и `docProps/app.xml`.
    pub metadata: Metadata,
    /// Relationships главной части (`word/_rels/document.xml.rels`).
    pub rels: Relationships,
    /// Сноски из `word/footnotes.xml`.
    pub footnotes: Vec<Footnote>,
    /// Концевые сноски из `word/endnotes.xml`.
    pub endnotes: Vec<Footnote>,
    /// Комментарии из `word/comments.xml`.
    pub comments: Vec<Comment>,
    /// Колонтитулы по имени части (`word/header1.xml`) — так на них ссылается `w:headerReference`.
    pub headers: BTreeMap<String, HeaderFooter>,
    /// Нижние колонтитулы по имени части (`word/footer1.xml`).
    pub footers: BTreeMap<String, HeaderFooter>,
    /// Восстановимые нарушения разбора в порядке появления.
    pub warnings: Vec<ParseWarning>,
}

impl Document {
    /// Предупреждения разбора в порядке появления.
    #[must_use]
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }

    /// Предупреждения одного вида — выборка для отчёта или диагностики.
    #[must_use]
    pub fn warnings_by_kind(&self, kind: WarningKind) -> Vec<&ParseWarning> {
        self.warnings.iter().filter(|w| w.kind == kind).collect()
    }

    /// Предупреждений больше `Warnings::UI_ALERT_THRESHOLD` — пора показать их
    /// пользователю (ADR-0016 §6), документ при этом разобран.
    #[must_use]
    pub fn has_fatal_warnings(&self) -> bool {
        self.warnings.len() > Warnings::UI_ALERT_THRESHOLD
    }
}

/// Тело документа (`w:body`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Body {
    /// Идентификатор узла тела.
    pub id: NodeId,
    /// Абзацы и таблицы в порядке следования.
    pub items: Vec<BlockItem>,
    /// Секции документа: концы секций из абзацев и завершающий `w:sectPr` тела.
    pub sections: Vec<Section>,
}

/// Блок тела документа.
// Модель — замороженный контракт: `Box` изменил бы вид варианта у всех потребителей,
// а `Paragraph` заведомо тяжелее `SectPr` и `Table`.
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BlockItem {
    /// Абзац.
    Paragraph(Paragraph),
    /// Таблица.
    Table(Table),
    /// Конец секции на уровне тела (`w:sectPr`).
    SectPr(SectionProperties),
    /// Неподдержанный блок: (id, XML) — сохраняется для отладки (ADR-0014 §3).
    Unknown {
        /// Идентификатор узла.
        id: NodeId,
        /// XML элемента целиком.
        xml: String,
    },
}

/// Свойства пакета (`docProps/core.xml` и `docProps/app.xml`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    /// Заголовок (`dc:title`).
    pub title: Option<String>,
    /// Тема (`dc:subject`).
    pub subject: Option<String>,
    /// Автор (`dc:creator`).
    pub creator: Option<String>,
    /// Ключевые слова (`cp:keywords`).
    pub keywords: Option<String>,
    /// Описание (`dc:description`).
    pub description: Option<String>,
    /// Кто изменял последним (`cp:lastModifiedBy`).
    pub last_modified_by: Option<String>,
    /// Категория (`cp:category`).
    pub category: Option<String>,
    /// Создавшее приложение (`Application` из `docProps/app.xml`).
    pub application: Option<String>,
    /// Время создания — строка ISO-8601 как в XML (`dcterms:created`).
    pub created: Option<String>,
    /// Время изменения — строка ISO-8601 как в XML (`dcterms:modified`).
    pub modified: Option<String>,
    /// Номер ревизии (`cp:revision`).
    pub revision: Option<u32>,
}

/// Relationships части, ключ — `rId`.
///
/// `core::rels::Relationship` не реализует `PartialEq`, а модель обязана:
/// сравнение написано вручную (см. `impl PartialEq` ниже).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Relationships {
    /// Отношения по идентификатору `rId`.
    pub items: BTreeMap<String, Relationship>,
}

impl Relationships {
    /// Отношение по идентификатору `rId`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.get(id)
    }

    /// Отношения, тип которых оканчивается на `suffix` (например, `/image`).
    ///
    /// Сравнение по суффиксу, а не по полному URI: полный тип версионно-зависим
    /// (`http://schemas.openxmlformats.org/officeDocument/2006/relationships/image`).
    /// Порядок обхода — порядок `BTreeMap`, то есть детерминированный.
    ///
    /// `#[must_use]` не ставится: `impl Iterator` и без него уже помечен
    /// `#[must_use]`, и атрибут ловит `clippy::double_must_use`.
    pub fn by_type(&self, suffix: &str) -> impl Iterator<Item = (&str, &Relationship)> + '_ {
        // Суффикс копируется в `String`: замыкание живёт внутри возвращаемого
        // итератора, и заимствованный суффикс связал бы его время жизни со
        // временем жизни результата — тогда `by_type("…")` не позвать с
        // временной строкой.
        let suffix = suffix.to_owned();
        self.items
            .iter()
            .filter(move |(_, rel)| rel.rel_type.ends_with(&suffix))
            .map(|(id, rel)| (id.as_str(), rel))
    }

    /// Собирает карту связей из разобранной `_rels`-части.
    #[must_use]
    pub fn from_map(map: RelMap) -> Self {
        Self {
            items: map.items.into_iter().collect(),
        }
    }
}

impl PartialEq for Relationships {
    fn eq(&self, other: &Self) -> bool {
        // `Relationship` приходит из чужого крейта и `PartialEq` не выводит,
        // поэтому сравниваем значимые поля вручную; ключ `rId` уникален в
        // каждой карте, так что равенства по ключам достаточно.
        self.items.len() == other.items.len()
            && self.items.iter().all(|(id, rel)| {
                other.items.get(id).is_some_and(|other| {
                    (
                        rel.id.as_str(),
                        rel.rel_type.as_str(),
                        rel.target.as_str(),
                        rel.target_mode.as_deref(),
                    ) == (
                        other.id.as_str(),
                        other.rel_type.as_str(),
                        other.target.as_str(),
                        other.target_mode.as_deref(),
                    )
                })
            })
    }
}
