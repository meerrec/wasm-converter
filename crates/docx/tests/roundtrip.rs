//! Lossless round-trip модели через JSON: документ собирается программно, сериализуется и
//! разбирается обратно — `assert_eq!` ловит поле или вариант enum, потерянные при сериализации.
//!
//! Сравнение идёт по структуре, а не по строкам JSON: `skip_serializing_if` с неверным
//! условием или `untagged`-неоднозначность проходят мимо строк и видны только здесь.
//!
//! Второй источник документа — фикстуры: `every_fixture_survives_a_json_round_trip`
//! гоняет круг по всему, что разбирается, и ловит поля, которых не выставляет
//! ни один программно собранный документ.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::path::{Path, PathBuf};

use doc_converter_core::rels::{RelMap, Relationship};
use doc_converter_core::{NodeId, ParseWarning, WarningKind};
use doc_converter_docx::{
    AbstractNum, AbstractNumId, AlignH, AlignV, Anchor, BlockItem, Body, Bookmark, Border,
    BorderStyle, BreakKind, Cell, CellBorders, CellMargins, CellVAlign, CellWidth,
    CharacterSpacing, CharacterSpacingControl, CharacterStyle, Color, ColumnDef, Columns, Comment,
    CompatSettings, ConditionalFormat, DefaultStyleIds, DocDefaults, Document, EndnotePos,
    EndnotePr, Extent, Field, FieldKind, FontHint, Footnote, FootnotePos, FootnotePr, GridCol,
    HalfPoint, HeaderFooter, HeightRule, Highlight, Hyperlink, Ind, Inline, InlineImage,
    InlineOrAnchor, Justification, LevelSuffix, LineSpacing, LineSpacingRule, Lvl, LvlOverride,
    Margins, Metadata, MultiLevelType, NoteKind, Num, NumFmt, NumId, NumPr, NumRestart,
    NumberingStyle, NumberingTable, Orientation, PageSize, Paragraph, ParagraphBorders,
    ParagraphSpacing, ParagraphStyle, PartRef, PositionH, PositionV, RFonts, RawPPr, RawRPr,
    RelFromH, RelFromV, Relationships, Row, RowHeight, Run, RunContent, Section, SectionProperties,
    SectionType, Settings, Shading, ShadingPattern, StyleId, StyleTable, TabLeader, TabStop,
    TabStopKind, Table, TableBorders, TableLayout, TableLook, TableStyle, TableStyleCondition,
    TableWidth, Toggle, Twips, Underline, VMerge, VertAlign, WrapKind,
};
// Парсер наружу открывается одним входом `open`: вариант с `ZipLimits` — внутренний.
use doc_converter_docx::open;
// `RawTblPr` — поле `TableStyle`/`ConditionalFormat`, но в поимённый реэкспорт
// корня (`lib.rs`) пока не попал: он добавлен слайсом S8 поверх модели.
use doc_converter_docx::model::raw::RawTblPr;
use serde::de::DeserializeOwned;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Проверка и общие куски документа
// ---------------------------------------------------------------------------

fn id(value: u64) -> NodeId {
    NodeId::new(value)
}

/// JSON-круг одного значения с проверкой потерь.
fn round_trip<T>(value: T) -> T
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_string(&value).unwrap();
    let decoded: T = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, value);
    decoded
}

fn round_trip_all<T, I>(values: I)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
    I: IntoIterator<Item = T>,
{
    for value in values {
        round_trip(value);
    }
}

/// Круг целого документа; повторная сериализация обязана дать ту же строку —
/// так проверяется ещё и устойчивость формы JSON (порядок ключей `BTreeMap`).
fn roundtrip_document(document: &Document) {
    let json = serde_json::to_string(document).unwrap();
    let decoded: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, *document);
    assert_eq!(serde_json::to_string(&decoded).unwrap(), json);
}

/// Документ с пустыми таблицами: тесты переопределяют только свой раздел.
fn base_document() -> Document {
    Document {
        id: NodeId::ROOT,
        body: Body {
            id: id(1),
            items: Vec::new(),
            sections: Vec::new(),
        },
        styles: StyleTable::default(),
        numbering: NumberingTable::default(),
        settings: Settings::default(),
        metadata: Metadata::default(),
        rels: Relationships::default(),
        footnotes: Vec::new(),
        endnotes: Vec::new(),
        comments: Vec::new(),
        headers: BTreeMap::new(),
        footers: BTreeMap::new(),
        warnings: Vec::new(),
    }
}

/// Короткий абзац без свойств — содержимое ячеек и колонтитулов.
fn simple_paragraph(node: u64, text: &str) -> Paragraph {
    Paragraph {
        id: id(node),
        ppr: Box::new(RawPPr::default()),
        mark_rpr: Box::new(RawRPr::default()),
        runs: vec![Inline::Run(Run {
            id: id(node + 1),
            rpr: Box::new(RawRPr::default()),
            style_ref: None,
            content: vec![RunContent::Text(text.to_owned())],
        })],
        style_ref: None,
        numbering_ref: None,
        section_break: None,
    }
}

fn body_with_text(node: u64, text: &str) -> Body {
    Body {
        id: id(node),
        items: vec![BlockItem::Paragraph(simple_paragraph(node + 1, text))],
        sections: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Полные значения модели
// ---------------------------------------------------------------------------

fn border(val: BorderStyle) -> Border {
    Border {
        val,
        sz: Some(8),
        space: Some(1),
        color: Some(Color::Rgb(0x0033_66CC)),
    }
}

fn full_rpr() -> RawRPr {
    RawRPr {
        style: Some(StyleId::from("Emphasis")),
        r_fonts: Some(RFonts {
            ascii: Some("Calibri".to_owned()),
            h_ansi: Some("Arial".to_owned()),
            east_asia: Some("MS Mincho".to_owned()),
            cs: Some("Times New Roman".to_owned()),
            hint: Some(FontHint::EastAsia),
        }),
        b: Some(Toggle::On),
        i: Some(Toggle::Off),
        caps: Some(Toggle::Inherit),
        small_caps: Some(Toggle::On),
        strike: Some(Toggle::Off),
        dstrike: Some(Toggle::On),
        vanish: Some(Toggle::Off),
        outline: Some(Toggle::On),
        shadow: Some(Toggle::Off),
        emboss: Some(Toggle::On),
        imprint: Some(Toggle::Off),
        color: Some(Color::Rgb(0x0033_66CC)),
        sz: Some(HalfPoint::new(24)),
        sz_cs: Some(HalfPoint::new(22)),
        highlight: Some(Highlight::Yellow),
        u: Some(Underline::WavyDouble),
        vert_align: Some(VertAlign::Superscript),
        spacing: Some(CharacterSpacing {
            value: Some(Twips::new(-12)),
        }),
        position: Some(HalfPoint::new(6)),
        unknown: vec![("kern".to_owned(), "<w:kern w:val=\"14\"/>".to_owned())],
    }
}

fn full_ppr() -> RawPPr {
    RawPPr {
        style: Some(StyleId::from("Heading1")),
        num_pr: Some(NumPr {
            ilvl: Some(2),
            num_id: Some(NumId::new(7)),
        }),
        spacing: Some(ParagraphSpacing {
            before: Some(Twips::new(240)),
            after: Some(Twips::new(120)),
            line: Some(LineSpacing::new(360)),
            line_rule: Some(LineSpacingRule::Auto),
            before_lines: Some(2),
            after_lines: Some(1),
            before_autospacing: true,
            after_autospacing: false,
        }),
        ind: Some(Ind {
            left: Some(Twips::new(720)),
            right: Some(Twips::new(0)),
            first_line: Some(Twips::new(360)),
            hanging: Some(Twips::new(180)),
        }),
        jc: Some(Justification::Both),
        keep_next: Some(Toggle::On),
        keep_lines: Some(Toggle::Off),
        page_break_before: Some(Toggle::On),
        widow_control: Some(Toggle::Inherit),
        outline_lvl: Some(1),
        p_bdr: Some(ParagraphBorders {
            top: Some(border(BorderStyle::Double)),
            left: Some(border(BorderStyle::Dashed)),
            bottom: Some(border(BorderStyle::Wave)),
            right: Some(border(BorderStyle::Inset)),
            between: Some(border(BorderStyle::Dotted)),
            bar: Some(border(BorderStyle::Nil)),
        }),
        shd: Some(Shading {
            val: ShadingPattern::Pct20,
            color: Some(Color::Auto),
            fill: Some(Color::Rgb(0x00FF_FF00)),
        }),
        tabs: vec![TabStop {
            val: Twips::new(720),
            kind: TabStopKind::Center,
            leader: TabLeader::Dot,
        }],
        r_pr: Some(full_rpr()),
        sect_pr: Some(full_section_properties()),
        unknown: vec![(
            "textAlignment".to_owned(),
            "<w:textAlignment w:val=\"auto\"/>".to_owned(),
        )],
    }
}

fn full_margins() -> Margins {
    Margins {
        top: Twips::new(1_440),
        right: Twips::new(1_080),
        bottom: Twips::new(1_440),
        left: Twips::new(1_080),
        header: Some(Twips::new(708)),
        footer: Some(Twips::new(708)),
        gutter: Some(Twips::new(0)),
    }
}

fn full_columns() -> Columns {
    Columns {
        count: 3,
        space: Twips::new(425),
        equal_width: false,
        separator: true,
        defs: vec![
            ColumnDef {
                width: Twips::new(2_500),
                space: Twips::new(425),
            },
            ColumnDef {
                width: Twips::new(3_000),
                space: Twips::new(0),
            },
        ],
    }
}

fn part_ref(rel_id: &str, target: &str) -> PartRef {
    PartRef {
        rel_id: rel_id.to_owned(),
        target: target.to_owned(),
        external: false,
    }
}

fn full_section_properties() -> SectionProperties {
    SectionProperties {
        page_size: PageSize {
            width: Twips::new(11_906),
            height: Twips::new(16_838),
        },
        orientation: Orientation::Landscape,
        margins: full_margins(),
        columns: full_columns(),
        title_pg: true,
        header_default: Some(part_ref("rIdHeaderDefault", "word/header1.xml")),
        header_first: Some(part_ref("rIdHeaderFirst", "word/header2.xml")),
        header_even: Some(PartRef {
            external: true,
            ..part_ref("rIdHeaderEven", "word/header3.xml")
        }),
        footer_default: Some(part_ref("rIdFooterDefault", "word/footer1.xml")),
        footer_first: Some(part_ref("rIdFooterFirst", "word/footer2.xml")),
        footer_even: Some(part_ref("rIdFooterEven", "word/footer3.xml")),
        section_type: Some(SectionType::Continuous),
        unknown: vec![(
            "docGrid".to_owned(),
            "<w:docGrid w:linePitch=\"360\"/>".to_owned(),
        )],
    }
}

fn full_section() -> Section {
    Section {
        id: id(70),
        properties: full_section_properties(),
        header_default: Some(part_ref("rIdHeaderDefault", "word/header1.xml")),
        header_first: Some(part_ref("rIdHeaderFirst", "word/header2.xml")),
        header_even: None,
        footer_default: Some(part_ref("rIdFooterDefault", "word/footer1.xml")),
        footer_first: None,
        footer_even: Some(part_ref("rIdFooterEven", "word/footer3.xml")),
        title_pg: true,
        page_size: PageSize {
            width: Twips::new(11_906),
            height: Twips::new(16_838),
        },
        orientation: Orientation::Landscape,
        margins: full_margins(),
        columns: full_columns(),
    }
}

// ---------------------------------------------------------------------------
// Inline-содержимое
// ---------------------------------------------------------------------------

fn inline_image() -> InlineImage {
    InlineImage {
        id: id(40),
        rel_id: "rIdImage1".to_owned(),
        part: Some("word/media/image1.png".to_owned()),
        name: Some("Picture 1".to_owned()),
        description: Some("Схема".to_owned()),
        extent: Extent {
            cx: 914_400,
            cy: 457_200,
        },
    }
}

fn anchored_drawing() -> InlineOrAnchor {
    InlineOrAnchor {
        id: id(41),
        inline: None,
        anchor: Some(Anchor {
            id: id(42),
            extent: Extent {
                cx: 1_828_800,
                cy: 914_400,
            },
            horizontal: PositionH {
                relative_from: RelFromH::Column,
                align: Some(AlignH::Outside),
                offset: Some(-12_700),
                percent: None,
            },
            vertical: PositionV {
                relative_from: RelFromV::Paragraph,
                align: Some(AlignV::Center),
                offset: None,
                percent: Some(-5_000),
            },
            wrap: WrapKind::TopAndBottom,
            behind_text: true,
            image: InlineImage {
                id: id(43),
                ..inline_image()
            },
        }),
    }
}

/// Второе заполнение `InlineOrAnchor` — только `inline`, без `anchor`.
fn inline_drawing() -> InlineOrAnchor {
    InlineOrAnchor {
        id: id(44),
        inline: Some(InlineImage {
            part: None,
            ..inline_image()
        }),
        anchor: None,
    }
}

fn full_run() -> Run {
    Run {
        id: id(11),
        rpr: Box::new(full_rpr()),
        style_ref: Some(StyleId::from("Strong")),
        content: vec![
            RunContent::Text("Привет".to_owned()),
            RunContent::Tab,
            RunContent::Break(BreakKind::Line),
            RunContent::Symbol {
                font: "Symbol".to_owned(),
                char: '\u{00B7}',
            },
            RunContent::Drawing(inline_drawing()),
            RunContent::Unknown {
                id: id(19),
                xml: "<w:noBreakHyphen/>".to_owned(),
            },
        ],
    }
}

fn full_hyperlink() -> Hyperlink {
    Hyperlink {
        id: id(22),
        rel_id: Some("rIdLink1".to_owned()),
        anchor: Some("section-2".to_owned()),
        tooltip: Some("Наверх".to_owned()),
        target: Some("https://example.com/".to_owned()),
        external: true,
        runs: vec![Inline::Run(full_run())],
    }
}

fn full_field() -> Field {
    Field {
        id: id(23),
        kind: FieldKind::Complex,
        instruction: "PAGE \\* MERGEFORMAT".to_owned(),
        result: vec![Inline::Run(full_run())],
        dirty: true,
    }
}

fn all_inlines() -> Vec<Inline> {
    vec![
        Inline::Run(full_run()),
        Inline::Hyperlink(full_hyperlink()),
        Inline::Bookmark(Bookmark {
            id: id(21),
            name: "mark".to_owned(),
            bookmark_id: -42,
        }),
        Inline::Field(full_field()),
        Inline::Break(BreakKind::Page),
        Inline::Tab,
        Inline::Symbol {
            font: "Wingdings".to_owned(),
            char: '\u{2705}',
        },
        Inline::Drawing(anchored_drawing()),
        Inline::Unknown {
            id: id(29),
            xml: "<w:customXml/>".to_owned(),
        },
    ]
}

fn full_paragraph() -> Paragraph {
    Paragraph {
        id: id(10),
        ppr: Box::new(full_ppr()),
        mark_rpr: Box::new(full_rpr()),
        runs: all_inlines(),
        style_ref: Some(StyleId::from("Heading1")),
        numbering_ref: Some(NumId::new(7)),
        section_break: Some(Box::new(full_section_properties())),
    }
}

// ---------------------------------------------------------------------------
// Таблица
// ---------------------------------------------------------------------------

fn full_cell() -> Cell {
    Cell {
        id: id(52),
        grid_span: 2,
        v_merge: Some(VMerge::Restart),
        width: Some(CellWidth::Dxa(Twips::new(4_000))),
        margins: CellMargins {
            top: Some(Twips::new(0)),
            left: Some(Twips::new(108)),
            bottom: Some(Twips::new(0)),
            right: Some(Twips::new(108)),
        },
        v_align: CellVAlign::Bottom,
        borders: CellBorders {
            top: Some(border(BorderStyle::Single)),
            left: Some(border(BorderStyle::Thick)),
            bottom: Some(border(BorderStyle::Outset)),
            right: Some(border(BorderStyle::ThinThickSmallGap)),
        },
        shading: Some(Shading {
            val: ShadingPattern::Solid,
            color: Some(Color::None),
            fill: Some(Color::Rgb(0x00EE_EEEE)),
        }),
        items: vec![BlockItem::Paragraph(simple_paragraph(53, "в ячейке"))],
    }
}

fn full_row() -> Row {
    Row {
        id: id(51),
        cells: vec![
            full_cell(),
            Cell {
                v_merge: Some(VMerge::Continue),
                width: Some(CellWidth::Nil),
                ..full_cell()
            },
        ],
        height: Some(RowHeight {
            value: Twips::new(480),
            rule: HeightRule::AtLeast,
        }),
        cant_split: true,
        header: false,
    }
}

fn full_table() -> Table {
    Table {
        id: id(50),
        style_ref: Some(StyleId::from("TableGrid")),
        grid: vec![
            GridCol {
                width: Twips::new(2_000),
            },
            GridCol {
                width: Twips::new(3_000),
            },
        ],
        rows: vec![full_row()],
        layout: TableLayout::Fixed,
        width: Some(TableWidth::Pct(33.5)),
        borders: TableBorders {
            top: Some(border(BorderStyle::Single)),
            left: Some(border(BorderStyle::Double)),
            bottom: Some(border(BorderStyle::DashSmallGap)),
            right: Some(border(BorderStyle::ThreeDEngrave)),
            inside_h: Some(border(BorderStyle::Dotted)),
            inside_v: Some(border(BorderStyle::DashDotStroked)),
        },
        look: TableLook {
            first_row: true,
            last_row: false,
            first_column: true,
            last_column: false,
            no_h_band: true,
            no_v_band: false,
        },
        jc: Some(Justification::Center),
        indent: Some(Twips::new(-108)),
        cell_margins: CellMargins {
            top: Some(Twips::new(0)),
            left: Some(Twips::new(108)),
            bottom: Some(Twips::new(0)),
            right: Some(Twips::new(108)),
        },
    }
}

fn all_block_items() -> Vec<BlockItem> {
    vec![
        BlockItem::Paragraph(full_paragraph()),
        BlockItem::Table(full_table()),
        BlockItem::SectPr(full_section_properties()),
        BlockItem::Unknown {
            id: id(30),
            xml: "<w:sdt/>".to_owned(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Стили и нумерация
// ---------------------------------------------------------------------------

fn full_paragraph_style() -> ParagraphStyle {
    ParagraphStyle {
        id: StyleId::from("Heading1"),
        name: Some("heading 1".to_owned()),
        based_on: Some(StyleId::from("Normal")),
        next: Some(StyleId::from("Normal")),
        link: Some(StyleId::from("Heading1Char")),
        is_default: false,
        hidden: true,
        custom: true,
        aliases: vec!["h1".to_owned(), "Заголовок 1".to_owned()],
        ppr: full_ppr(),
        rpr: full_rpr(),
    }
}

fn full_character_style() -> CharacterStyle {
    CharacterStyle {
        id: StyleId::from("Strong"),
        name: Some("Strong".to_owned()),
        based_on: Some(StyleId::from("DefaultParagraphFont")),
        link: Some(StyleId::from("Heading1")),
        is_default: false,
        hidden: false,
        custom: true,
        aliases: vec!["Жирный".to_owned()],
        rpr: full_rpr(),
    }
}

/// Свойства таблицы условного формата и стиля (`w:tblPr`).
fn full_tbl_pr() -> RawTblPr {
    RawTblPr {
        borders: TableBorders {
            top: Some(border(BorderStyle::Single)),
            left: Some(border(BorderStyle::Double)),
            bottom: Some(border(BorderStyle::DashSmallGap)),
            right: Some(border(BorderStyle::ThreeDEngrave)),
            inside_h: Some(border(BorderStyle::Dotted)),
            inside_v: Some(border(BorderStyle::DashDotStroked)),
        },
        cell_margins: CellMargins {
            top: Some(Twips::new(0)),
            left: Some(Twips::new(108)),
            bottom: Some(Twips::new(0)),
            right: Some(Twips::new(108)),
        },
        width: Some(TableWidth::Pct(33.5)),
        look: Some(TableLook {
            first_row: true,
            last_row: false,
            first_column: true,
            last_column: false,
            no_h_band: true,
            no_v_band: false,
        }),
        jc: Some(Justification::Center),
        indent: Some(Twips::new(-108)),
        unknown: vec![(
            "tblStyleRowBandSize".to_owned(),
            "<w:tblStyleRowBandSize w:val=\"2\"/>".to_owned(),
        )],
    }
}

/// Все условия `ST_TblStyleOverrideType` плюс неизвестное значение.
fn conditional_formats() -> Vec<ConditionalFormat> {
    let mut formats: Vec<ConditionalFormat> = TableStyleCondition::ALL
        .into_iter()
        .map(|kind| ConditionalFormat {
            kind,
            ppr: full_ppr(),
            rpr: full_rpr(),
            tbl_pr: full_tbl_pr(),
        })
        .collect();
    formats.push(ConditionalFormat {
        kind: TableStyleCondition::Other("band3Horz".to_owned()),
        ppr: RawPPr::default(),
        rpr: RawRPr::default(),
        tbl_pr: RawTblPr::default(),
    });
    formats
}

fn full_table_style() -> TableStyle {
    TableStyle {
        id: StyleId::from("TableGrid"),
        name: Some("Table Grid".to_owned()),
        based_on: Some(StyleId::from("TableNormal")),
        is_default: true,
        hidden: false,
        custom: true,
        aliases: vec!["Сетка".to_owned()],
        ppr: full_ppr(),
        rpr: full_rpr(),
        tbl_pr: full_tbl_pr(),
        conditional: conditional_formats(),
    }
}

fn full_numbering_style() -> NumberingStyle {
    NumberingStyle {
        id: StyleId::from("ListParagraph"),
        name: Some("List Paragraph".to_owned()),
        based_on: Some(StyleId::from("Normal")),
        is_default: false,
        hidden: true,
        custom: true,
        aliases: vec!["Список".to_owned()],
        ppr: full_ppr(),
        rpr: full_rpr(),
    }
}

fn full_styles() -> StyleTable {
    StyleTable {
        doc_defaults: DocDefaults {
            r_pr: full_rpr(),
            p_pr: full_ppr(),
        },
        paragraph: BTreeMap::from([(StyleId::from("Heading1"), full_paragraph_style())]),
        character: BTreeMap::from([(StyleId::from("Strong"), full_character_style())]),
        table: BTreeMap::from([(StyleId::from("TableGrid"), full_table_style())]),
        numbering: BTreeMap::from([(StyleId::from("ListParagraph"), full_numbering_style())]),
        defaults: DefaultStyleIds {
            paragraph: Some(StyleId::from("Normal")),
            character: Some(StyleId::from("DefaultParagraphFont")),
            table: Some(StyleId::from("TableNormal")),
            numbering: Some(StyleId::from("NoList")),
        },
    }
}

fn full_lvl(ilvl: u8) -> Lvl {
    Lvl {
        ilvl,
        start: 3,
        num_fmt: NumFmt::UpperRoman,
        lvl_text: format!("%{}.", u32::from(ilvl) + 1),
        lvl_jc: Some(Justification::Left),
        suff: LevelSuffix::Tab,
        ppr: full_ppr(),
        rpr: full_rpr(),
        restart: Some(2),
        pstyle: Some(StyleId::from("ListParagraph")),
        is_lgl: true,
        pic_bullet_id: Some(5),
        tentative: false,
    }
}

fn full_abstract_num() -> AbstractNum {
    AbstractNum {
        id: AbstractNumId::new(0),
        multi_level_type: MultiLevelType::HybridMultilevel,
        levels: BTreeMap::from([(0, full_lvl(0)), (1, full_lvl(1))]),
        num_style_link: Some(StyleId::from("ListNumber")),
        style_link: Some(StyleId::from("ListParagraph")),
        nsid: Some("1D5B2C3A".to_owned()),
        tmpl: Some("A1B2C3D4".to_owned()),
    }
}

fn full_num() -> Num {
    Num {
        id: NumId::new(1),
        abstract_id: AbstractNumId::new(0),
        overrides: BTreeMap::from([
            (
                0,
                LvlOverride {
                    start_override: Some(5),
                    lvl: None,
                },
            ),
            (
                1,
                LvlOverride {
                    start_override: None,
                    lvl: Some(full_lvl(1)),
                },
            ),
        ]),
        picture_bullet_id: Some(42),
    }
}

fn full_numbering() -> NumberingTable {
    NumberingTable {
        abstract_nums: BTreeMap::from([(AbstractNumId::new(0), full_abstract_num())]),
        nums: BTreeMap::from([(NumId::new(1), full_num())]),
    }
}

// ---------------------------------------------------------------------------
// Settings, метаданные, связи, примечания
// ---------------------------------------------------------------------------

fn full_settings() -> Settings {
    Settings {
        default_tab_stop: Some(Twips::new(708)),
        even_and_odd_headers: true,
        footnote_pr: FootnotePr {
            pos: Some(FootnotePos::BeneathText),
            num_fmt: Some(NumFmt::LowerRoman),
            num_start: Some(1),
            num_restart: Some(NumRestart::EachPage),
        },
        endnote_pr: EndnotePr {
            pos: Some(EndnotePos::DocEnd),
            num_fmt: Some(NumFmt::LowerLetter),
            num_start: Some(2),
            num_restart: Some(NumRestart::EachSection),
        },
        compat: CompatSettings {
            do_not_expand_shift_return: true,
            do_not_use_html_paragraph_auto_spacing: true,
            do_not_autofit_tables: false,
            do_not_break_wrapped_tables: true,
            do_not_vert_align_in_cell_wi: true,
            do_not_use_east_asian_break: false,
            use_single_border_for_contiguous_cells: true,
            compat_setting_override_table_style: true,
            unknown: vec![(
                "doNotHyphenateCaps".to_owned(),
                "<w:doNotHyphenateCaps/>".to_owned(),
            )],
        },
        character_spacing_control: CharacterSpacingControl::CompressPunctuationAndJapaneseKana,
    }
}

fn full_metadata() -> Metadata {
    Metadata {
        title: Some("Отчёт".to_owned()),
        subject: Some("Тема".to_owned()),
        creator: Some("Автор".to_owned()),
        keywords: Some("docx, тест".to_owned()),
        description: Some("Описание".to_owned()),
        last_modified_by: Some("Редактор".to_owned()),
        category: Some("Категория".to_owned()),
        application: Some("Microsoft Word".to_owned()),
        created: Some("2026-10-05T10:00:00Z".to_owned()),
        modified: Some("2026-10-06T11:30:00Z".to_owned()),
        revision: Some(7),
    }
}

fn relationship(id: &str, rel_type: &str, target: &str, target_mode: Option<&str>) -> Relationship {
    Relationship {
        id: id.to_owned(),
        rel_type: rel_type.to_owned(),
        target: target.to_owned(),
        target_mode: target_mode.map(str::to_owned),
    }
}

fn full_relationships() -> Relationships {
    Relationships::from_map(RelMap {
        items: HashMap::from([
            (
                "rId1".to_owned(),
                relationship(
                    "rId1",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
                    "word/document.xml",
                    None,
                ),
            ),
            (
                "rId2".to_owned(),
                relationship(
                    "rId2",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
                    "word/styles.xml",
                    None,
                ),
            ),
            (
                "rId3".to_owned(),
                relationship(
                    "rId3",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
                    "media/image1.png",
                    Some("External"),
                ),
            ),
        ]),
    })
}

fn full_warnings() -> Vec<ParseWarning> {
    vec![
        ParseWarning::at(
            WarningKind::UnknownElement,
            "element is not supported",
            "word/document.xml",
        )
        .with_node(id(60))
        .with_xml_path("w:document/w:body/w:p[3]/w:pPr/w:rPr"),
        ParseWarning::new(
            WarningKind::OrphanBookmark,
            "bookmarkEnd without bookmarkStart",
        ),
    ]
}

fn full_header() -> HeaderFooter {
    HeaderFooter {
        id: id(80),
        part: "word/header1.xml".to_owned(),
        body: body_with_text(81, "Колонтитул"),
    }
}

fn full_footer() -> HeaderFooter {
    HeaderFooter {
        id: id(82),
        part: "word/footer1.xml".to_owned(),
        body: body_with_text(83, "Подвал"),
    }
}

fn full_footnotes() -> Vec<Footnote> {
    vec![
        Footnote {
            id: id(90),
            note_id: 1,
            kind: NoteKind::Footnote,
            body: vec![BlockItem::Paragraph(simple_paragraph(91, "Сноска"))],
            mark_rpr: full_rpr(),
        },
        Footnote {
            id: id(92),
            note_id: -1,
            kind: NoteKind::Separator,
            body: Vec::new(),
            mark_rpr: RawRPr::default(),
        },
    ]
}

fn full_endnotes() -> Vec<Footnote> {
    vec![Footnote {
        id: id(93),
        note_id: 2,
        kind: NoteKind::Endnote,
        body: vec![BlockItem::Paragraph(simple_paragraph(
            94,
            "Концевая сноска",
        ))],
        mark_rpr: RawRPr::default(),
    }]
}

fn full_comments() -> Vec<Comment> {
    vec![Comment {
        id: id(95),
        comment_id: 3,
        author: Some("Рецензент".to_owned()),
        initials: Some("РЦ".to_owned()),
        date: Some("2026-10-05T10:00:00Z".to_owned()),
        body: vec![BlockItem::Paragraph(simple_paragraph(96, "Комментарий"))],
    }]
}

// ---------------------------------------------------------------------------
// Перечни вариантов enum'ов
// ---------------------------------------------------------------------------

fn all_underlines() -> Vec<Underline> {
    vec![
        Underline::Single,
        Underline::Words,
        Underline::Double,
        Underline::Thick,
        Underline::Dotted,
        Underline::DottedHeavy,
        Underline::Dash,
        Underline::DashedHeavy,
        Underline::DashLong,
        Underline::DashLongHeavy,
        Underline::DotDash,
        Underline::DashDotHeavy,
        Underline::DotDotDash,
        Underline::DashDotDotHeavy,
        Underline::Wave,
        Underline::WavyHeavy,
        Underline::WavyDouble,
        Underline::None,
        Underline::Other("dashDotDotHeavy".to_owned()),
    ]
}

fn all_highlights() -> Vec<Highlight> {
    vec![
        Highlight::Black,
        Highlight::Blue,
        Highlight::Cyan,
        Highlight::Green,
        Highlight::Magenta,
        Highlight::Red,
        Highlight::Yellow,
        Highlight::White,
        Highlight::DarkBlue,
        Highlight::DarkCyan,
        Highlight::DarkGreen,
        Highlight::DarkMagenta,
        Highlight::DarkRed,
        Highlight::DarkYellow,
        Highlight::DarkGray,
        Highlight::LightGray,
        Highlight::None,
        Highlight::Other("darkYellow".to_owned()),
    ]
}

fn all_font_hints() -> Vec<FontHint> {
    vec![
        FontHint::Default,
        FontHint::EastAsia,
        FontHint::Cs,
        FontHint::Other("hAnsi".to_owned()),
    ]
}

fn all_justifications() -> Vec<Justification> {
    vec![
        Justification::Left,
        Justification::Center,
        Justification::Right,
        Justification::Both,
        Justification::Distribute,
        Justification::Start,
        Justification::End,
        Justification::Other("thaiDistribute".to_owned()),
    ]
}

fn all_tab_stop_kinds() -> Vec<TabStopKind> {
    vec![
        TabStopKind::Bar,
        TabStopKind::Center,
        TabStopKind::Clear,
        TabStopKind::Decimal,
        TabStopKind::End,
        TabStopKind::Num,
        TabStopKind::Start,
        TabStopKind::Left,
        TabStopKind::Right,
        TabStopKind::Other("clearAll".to_owned()),
    ]
}

fn all_tab_leaders() -> Vec<TabLeader> {
    vec![
        TabLeader::None,
        TabLeader::Dot,
        TabLeader::Hyphen,
        TabLeader::MiddleDot,
        TabLeader::Heavy,
        TabLeader::Underscore,
        TabLeader::Other("dash".to_owned()),
    ]
}

fn all_line_spacing_rules() -> Vec<LineSpacingRule> {
    vec![
        LineSpacingRule::Auto,
        LineSpacingRule::Exact,
        LineSpacingRule::AtLeast,
        LineSpacingRule::Other("auto".to_owned()),
    ]
}

fn all_break_kinds() -> Vec<BreakKind> {
    vec![
        BreakKind::Line,
        BreakKind::Page,
        BreakKind::Column,
        BreakKind::Unsupported("textWrapping".to_owned()),
    ]
}

fn all_wrap_kinds() -> Vec<WrapKind> {
    vec![
        WrapKind::None,
        WrapKind::Square,
        WrapKind::Tight,
        WrapKind::Through,
        WrapKind::TopAndBottom,
    ]
}

fn all_rel_from_h() -> Vec<RelFromH> {
    vec![
        RelFromH::Margin,
        RelFromH::Page,
        RelFromH::Column,
        RelFromH::Character,
        RelFromH::LeftMargin,
        RelFromH::RightMargin,
        RelFromH::InsideMargin,
        RelFromH::OutsideMargin,
        RelFromH::Other("page".to_owned()),
    ]
}

fn all_rel_from_v() -> Vec<RelFromV> {
    vec![
        RelFromV::Margin,
        RelFromV::Page,
        RelFromV::Paragraph,
        RelFromV::Line,
        RelFromV::TopMargin,
        RelFromV::BottomMargin,
        RelFromV::InsideMargin,
        RelFromV::OutsideMargin,
        RelFromV::Other("line".to_owned()),
    ]
}

fn all_align_h() -> Vec<AlignH> {
    vec![
        AlignH::Left,
        AlignH::Center,
        AlignH::Right,
        AlignH::Inside,
        AlignH::Outside,
        AlignH::Other("center".to_owned()),
    ]
}

fn all_align_v() -> Vec<AlignV> {
    vec![
        AlignV::Top,
        AlignV::Center,
        AlignV::Bottom,
        AlignV::Inside,
        AlignV::Outside,
        AlignV::Other("bottom".to_owned()),
    ]
}

fn all_level_suffixes() -> Vec<LevelSuffix> {
    vec![LevelSuffix::Tab, LevelSuffix::Space, LevelSuffix::Nothing]
}

fn all_multi_level_types() -> Vec<MultiLevelType> {
    vec![
        MultiLevelType::SingleLevel,
        MultiLevelType::Multilevel,
        MultiLevelType::HybridMultilevel,
    ]
}

fn all_section_types() -> Vec<SectionType> {
    vec![
        SectionType::NextPage,
        SectionType::Continuous,
        SectionType::EvenPage,
        SectionType::OddPage,
    ]
}

fn all_note_kinds() -> Vec<NoteKind> {
    vec![
        NoteKind::Footnote,
        NoteKind::Endnote,
        NoteKind::Separator,
        NoteKind::ContinuationSeparator,
        NoteKind::ContinuationNotice,
    ]
}

fn all_footnote_pos() -> Vec<FootnotePos> {
    vec![
        FootnotePos::PageBottom,
        FootnotePos::BeneathText,
        FootnotePos::SectionEnd,
        FootnotePos::DocEnd,
        FootnotePos::Other("bottom".to_owned()),
    ]
}

fn all_endnote_pos() -> Vec<EndnotePos> {
    vec![
        EndnotePos::SectionEnd,
        EndnotePos::DocEnd,
        EndnotePos::Other("bottom".to_owned()),
    ]
}

fn all_num_restarts() -> Vec<NumRestart> {
    vec![
        NumRestart::Continuous,
        NumRestart::EachSection,
        NumRestart::EachPage,
        NumRestart::Other("eachSection".to_owned()),
    ]
}

fn all_character_spacing_controls() -> Vec<CharacterSpacingControl> {
    vec![
        CharacterSpacingControl::DoNotCompress,
        CharacterSpacingControl::CompressPunctuation,
        CharacterSpacingControl::CompressPunctuationAndJapaneseKana,
        CharacterSpacingControl::Other("compressPunctuation".to_owned()),
    ]
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

#[test]
fn raw_properties_and_primitives() {
    round_trip(Twips::new(-360));
    round_trip(HalfPoint::new(24));
    round_trip(LineSpacing::new(360));
    round_trip(StyleId::from("Heading1"));
    round_trip_all([Toggle::On, Toggle::Off, Toggle::Inherit]);
    round_trip_all([Color::Rgb(0x0033_66CC), Color::Auto, Color::None]);
    round_trip_all(all_underlines());
    round_trip_all(all_highlights());
    round_trip_all(all_font_hints());
    round_trip_all(all_justifications());
    round_trip_all(all_tab_stop_kinds());
    round_trip_all(all_tab_leaders());
    round_trip_all(all_line_spacing_rules());
    round_trip_all([CellVAlign::Top, CellVAlign::Center, CellVAlign::Bottom]);
    round_trip_all([
        VertAlign::Baseline,
        VertAlign::Superscript,
        VertAlign::Subscript,
    ]);
    round_trip_all(BorderStyle::ALL);
    round_trip(BorderStyle::Other("cliff".to_owned()));
    round_trip_all(ShadingPattern::ALL);
    round_trip(ShadingPattern::Other("diagCross".to_owned()));

    round_trip(full_rpr());
    round_trip(full_ppr());

    let mut document = base_document();
    document.body.items = vec![BlockItem::Paragraph(full_paragraph())];
    roundtrip_document(&document);
}

#[test]
fn inline_and_block_items() {
    round_trip_all(all_inlines());
    round_trip_all(all_break_kinds());
    round_trip_all([FieldKind::Simple, FieldKind::Complex]);
    round_trip(full_run());
    round_trip(full_hyperlink());
    round_trip(full_field());
    round_trip(full_paragraph());

    let mut document = base_document();
    document.body.items = all_block_items();
    document.body.sections = vec![full_section()];
    roundtrip_document(&document);
}

#[test]
fn table_and_drawing() {
    round_trip_all([
        TableWidth::Auto,
        TableWidth::Dxa(Twips::new(5_000)),
        TableWidth::Pct(33.5),
    ]);
    round_trip_all([
        CellWidth::Dxa(Twips::new(2_000)),
        CellWidth::Pct(66.75),
        CellWidth::Nil,
    ]);
    round_trip_all([TableLayout::Fixed, TableLayout::Autofit]);
    round_trip_all([VMerge::Continue, VMerge::Restart]);
    round_trip_all([HeightRule::Auto, HeightRule::AtLeast, HeightRule::Exact]);
    round_trip_all(all_wrap_kinds());
    round_trip_all(all_rel_from_h());
    round_trip_all(all_rel_from_v());
    round_trip_all(all_align_h());
    round_trip_all(all_align_v());
    round_trip(PositionH {
        relative_from: RelFromH::Margin,
        align: Some(AlignH::Inside),
        offset: Some(12_700),
        percent: None,
    });
    round_trip(PositionV {
        relative_from: RelFromV::Line,
        align: None,
        offset: None,
        percent: Some(2_500),
    });
    round_trip(anchored_drawing());
    round_trip(inline_drawing());

    let mut document = base_document();
    document.body.items = vec![
        BlockItem::Table(full_table()),
        BlockItem::Paragraph(full_paragraph()),
    ];
    roundtrip_document(&document);
}

#[test]
fn styles_and_numbering() {
    round_trip_all(TableStyleCondition::ALL);
    round_trip(TableStyleCondition::Other("band3Vert".to_owned()));
    round_trip_all(NumFmt::ALL);
    round_trip(NumFmt::Other("custom".to_owned()));
    round_trip_all(all_level_suffixes());
    round_trip_all(all_multi_level_types());
    round_trip(AbstractNumId::new(3));
    round_trip(NumId::new(9));
    round_trip(full_lvl(4));
    round_trip(full_abstract_num());
    round_trip(full_num());
    round_trip(full_styles());
    round_trip(full_numbering());

    let mut document = base_document();
    document.styles = full_styles();
    document.numbering = full_numbering();
    roundtrip_document(&document);
}

#[test]
fn settings_metadata_and_rels() {
    round_trip_all(all_footnote_pos());
    round_trip_all(all_endnote_pos());
    round_trip_all(all_num_restarts());
    round_trip_all(all_character_spacing_controls());
    round_trip(full_settings());
    round_trip(full_metadata());
    round_trip(full_relationships());
    round_trip(full_header());
    round_trip(full_footer());
    round_trip_all(full_warnings());

    let mut document = base_document();
    document.settings = full_settings();
    document.metadata = full_metadata();
    document.rels = full_relationships();
    document.headers = BTreeMap::from([("word/header1.xml".to_owned(), full_header())]);
    document.footers = BTreeMap::from([("word/footer1.xml".to_owned(), full_footer())]);
    document.warnings = full_warnings();
    roundtrip_document(&document);
}

#[test]
fn annotations_and_sections() {
    round_trip_all(all_note_kinds());
    round_trip_all(all_section_types());
    round_trip_all([Orientation::Portrait, Orientation::Landscape]);
    round_trip(full_section_properties());
    round_trip(full_section());
    round_trip_all(full_footnotes());
    round_trip_all(full_endnotes());
    round_trip_all(full_comments());

    let mut document = base_document();
    document.body = Body {
        id: id(1),
        items: all_block_items(),
        sections: vec![full_section()],
    };
    document.footnotes = full_footnotes();
    document.endnotes = full_endnotes();
    document.comments = full_comments();
    roundtrip_document(&document);
}

// ---------------------------------------------------------------------------
// Обход фикстур
// ---------------------------------------------------------------------------

/// Каталог фикстур DOCX.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/docx")
}

/// Все `.docx` под каталогом фикстур, по возрастанию пути: обход рекурсивный,
/// потому что фикстуры разложены по категориям (`broken/`, `simple/`, …).
fn fixture_paths() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("docx") {
                out.push(path);
            }
        }
    }

    let mut paths = Vec::new();
    walk(&fixtures_dir(), &mut paths);
    paths
}

#[test]
fn every_fixture_survives_a_json_round_trip() {
    let paths = fixture_paths();
    assert!(!paths.is_empty(), "фикстуры не найдены");

    let mut round_tripped = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for path in &paths {
        let name = path
            .strip_prefix(fixtures_dir())
            .unwrap_or(path)
            .display()
            .to_string();
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let document = match open(bytes) {
            Ok(document) => document,
            // Фатальные фикстуры проверяются в `warnings.rs`: здесь важно
            // только то, что дошло до модели.
            Err(_) => {
                skipped.push(name);
                continue;
            }
        };
        let json = serde_json::to_string(&document).unwrap();
        let decoded: Document = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("{name}: JSON не разбирается обратно: {e}"));
        assert_eq!(decoded, document, "{name}: JSON round-trip потерял данные");
        round_tripped += 1;
    }

    println!(
        "round-trip прошло {round_tripped} фикстур из {} (пропущено {}: {})",
        paths.len(),
        skipped.len(),
        skipped.join(", ")
    );
}
