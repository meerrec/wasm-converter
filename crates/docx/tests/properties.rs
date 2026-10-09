//! Property-тесты разбора DOCX (`proptest`): проверяются свойства формата, а не
//! отдельные примеры.
//!
//! Ожидания выведены из спецификации, а не из поведения парсера, — иначе тест
//! был бы тавтологией. Откуда взято каждое:
//!
//! * уникальность и детерминированность [`NodeId`] — ADR-0019 §2: идентификатор
//!   выдаётся в порядке обхода XML, по одному на узел;
//! * независимость обхода от раскладки ZIP — ECMA-376 Part 2 §10: OPC-пакет это
//!   множество частей, порядок записей в архиве смысла не несёт, а посторонняя
//!   часть не часть документа;
//! * тринстейт `Toggle` — ECMA-376 Part 1 §17.17.4 (`ST_OnOff`) и §17.7.1
//!   (`w:b`): `on`/`true`/`1` включают свойство, `off`/`false`/`0` выключают,
//!   элемент без `w:val` включён, а третий вариант `Inherit` («наследуется»)
//!   описан доккомментарием [`Toggle`] и в модели значим как отдельное
//!   состояние, а не как bool;
//! * `basedOn` — ECMA-376 Part 1 §17.7.4.4: основа стиля обязана существовать,
//!   а цикл по `basedOn` не определён; разбор обязан пережить и то и другое
//!   предупреждением, а не отказом или зависанием.
//!
//! Пакет собирается в памяти через dev-зависимость `zip`, разбор идёт публичным
//! входом [`parse_docx`] — литералы `Paragraph`/`Run`/`Document` не нужны и не
//! используются: модель обязана получиться из XML.

use std::collections::BTreeSet;
use std::io::{Cursor, Write};
use std::sync::mpsc;
use std::time::Duration;

use doc_converter_core::{NodeId, WarningKind, ZipLimits};
use doc_converter_docx::{
    parse_docx, BlockItem, Body, Document, Inline, InlineOrAnchor, Run, RunContent, Toggle,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Сборка пакета
// ---------------------------------------------------------------------------

/// Минимальный набор частей: content types, связи пакета, главная часть и её
/// (пустые) связи. Без `[Content_Types].xml` разбор отказывает, без связей
/// главной части предупреждает `MissingRels` и мешает тестам, где
/// предупреждения считаются поимённо.
const CONTENT_TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const EMPTY_RELS: &str =
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;

/// Пространство имён `WordprocessingML`: Strict разбор отвергает, как и Word.
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// ZIP из готовых частей в заданном порядке; `deflate` — способ упаковки
/// записей, ещё одно измерение раскладки архива.
fn zip(parts: &[(String, Vec<u8>)], deflate: bool) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = if deflate {
            zip::write::SimpleFileOptions::default()
        } else {
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
        };
        for (name, data) in parts {
            writer.start_file(name.as_str(), options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    buf
}

/// Части пакета без `word/styles.xml` (он не нужен свойствам обхода) либо с ним:
/// таблица стилей находится по стандартному пути, связь для неё не обязательна.
fn package_parts(document: &str, styles: Option<&str>) -> Vec<(String, Vec<u8>)> {
    let mut parts = vec![
        (
            "[Content_Types].xml".to_owned(),
            CONTENT_TYPES.as_bytes().to_vec(),
        ),
        ("_rels/.rels".to_owned(), ROOT_RELS.as_bytes().to_vec()),
        ("word/document.xml".to_owned(), document.as_bytes().to_vec()),
        (
            "word/_rels/document.xml.rels".to_owned(),
            EMPTY_RELS.as_bytes().to_vec(),
        ),
    ];
    if let Some(styles) = styles {
        parts.push(("word/styles.xml".to_owned(), styles.as_bytes().to_vec()));
    }
    parts
}

fn package_bytes(document: &str, styles: Option<&str>) -> Vec<u8> {
    zip(&package_parts(document, styles), true)
}

fn document_xml(body: &str) -> String {
    format!(r#"<w:document xmlns:w="{W_NS}"><w:body>{body}</w:body></w:document>"#)
}

fn parse(bytes: &[u8]) -> Document {
    parse_docx(bytes, ZipLimits::default()).expect("сгенерированный пакет разбирается")
}

/// Разбор в отдельном потоке с бюджетом времени: `basedOn` обходится циклом, и
/// вечный цикл обязан проявиться паникой, а не зависанием всего прогона.
fn parse_within_budget(bytes: &[u8], budget: Duration) -> Document {
    let owned = bytes.to_vec();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(parse_docx(&owned, ZipLimits::default()));
    });
    match receiver.recv_timeout(budget) {
        Ok(result) => result.expect("сгенерированный пакет разбирается"),
        Err(_) => panic!("разбор не завершился за {budget:?}"),
    }
}

// ---------------------------------------------------------------------------
// Обход модели
// ---------------------------------------------------------------------------

/// Идентификаторы всех узлов документа в порядке обхода XML.
fn node_ids(document: &Document) -> Vec<NodeId> {
    let mut ids = vec![document.id, document.body.id];
    block_ids(&document.body.items, &mut ids);
    for section in &document.body.sections {
        ids.push(section.id);
    }
    for note in document.footnotes.iter().chain(&document.endnotes) {
        ids.push(note.id);
        block_ids(&note.body, &mut ids);
    }
    for comment in &document.comments {
        ids.push(comment.id);
        block_ids(&comment.body, &mut ids);
    }
    for part in document.headers.values().chain(document.footers.values()) {
        ids.push(part.id);
        ids.push(part.body.id);
        block_ids(&part.body.items, &mut ids);
    }
    ids
}

fn block_ids(items: &[BlockItem], out: &mut Vec<NodeId>) {
    for item in items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                out.push(paragraph.id);
                inline_ids(&paragraph.runs, out);
            }
            BlockItem::Table(table) => {
                out.push(table.id);
                for row in &table.rows {
                    out.push(row.id);
                    for cell in &row.cells {
                        out.push(cell.id);
                        block_ids(&cell.items, out);
                    }
                }
            }
            // У `w:sectPr` собственного идентификатора нет: `SectionProperties` —
            // не узел обхода, в отличие от `Section` из `Body::sections`.
            BlockItem::SectPr(_) => {}
            BlockItem::Unknown { id, .. } => out.push(*id),
        }
    }
}

fn inline_ids(items: &[Inline], out: &mut Vec<NodeId>) {
    for item in items {
        match item {
            Inline::Run(run) => {
                out.push(run.id);
                for content in &run.content {
                    if let RunContent::Drawing(drawing) = content {
                        drawing_ids(drawing, out);
                    }
                    if let RunContent::Unknown { id, .. } = content {
                        out.push(*id);
                    }
                }
            }
            Inline::Hyperlink(link) => {
                out.push(link.id);
                inline_ids(&link.runs, out);
            }
            Inline::Bookmark(bookmark) => out.push(bookmark.id),
            Inline::Field(field) => {
                out.push(field.id);
                inline_ids(&field.result, out);
            }
            Inline::Drawing(drawing) => drawing_ids(drawing, out),
            Inline::Unknown { id, .. } => out.push(*id),
            Inline::Break(_) | Inline::Tab | Inline::Symbol { .. } => {}
        }
    }
}

fn drawing_ids(drawing: &InlineOrAnchor, out: &mut Vec<NodeId>) {
    out.push(drawing.id);
    if let Some(image) = &drawing.inline {
        out.push(image.id);
    }
    if let Some(anchor) = &drawing.anchor {
        out.push(anchor.id);
        out.push(anchor.image.id);
    }
}

/// Нормализованный текст тела: ровно то, что увидит потребитель модели.
fn body_text(body: &Body) -> String {
    let mut out = String::new();
    blocks_text(&body.items, &mut out);
    out
}

fn blocks_text(items: &[BlockItem], out: &mut String) {
    for item in items {
        match item {
            BlockItem::Paragraph(paragraph) => {
                inlines_text(&paragraph.runs, out);
                out.push('\n');
            }
            BlockItem::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        blocks_text(&cell.items, out);
                    }
                }
            }
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }
}

fn inlines_text(items: &[Inline], out: &mut String) {
    for item in items {
        match item {
            Inline::Run(run) => {
                for content in &run.content {
                    if let RunContent::Text(text) = content {
                        out.push_str(text);
                    }
                }
            }
            Inline::Hyperlink(link) => inlines_text(&link.runs, out),
            Inline::Field(field) => inlines_text(&field.result, out),
            Inline::Bookmark(_)
            | Inline::Break(_)
            | Inline::Tab
            | Inline::Symbol { .. }
            | Inline::Drawing(_)
            | Inline::Unknown { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Генератор тела документа
// ---------------------------------------------------------------------------

/// Блок тела: абзац или таблица; таблица вкладывается в ячейки, поэтому обход
/// проверяется и на вложенности.
#[derive(Debug, Clone)]
enum Block {
    Paragraph(Vec<InlineSpec>),
    Table(Vec<Row>),
}

type Row = Vec<Cell>;
type Cell = Vec<Block>;

/// Inline-элемент сгенерированного абзаца; имя не `Inline`, чтобы не спорить с
/// моделью (`doc_converter_docx::Inline`).
#[derive(Debug, Clone)]
enum InlineSpec {
    Run {
        bold: bool,
        italic: bool,
        text: String,
    },
    /// Внутренняя ссылка (`w:anchor`): внешняя требовала бы связи в rels.
    Link(Vec<String>),
    /// Простое поле `w:fldSimple`.
    Field(Vec<String>),
    /// Пара `w:bookmarkStart`/`w:bookmarkEnd` — иначе разбор предупредит
    /// `OrphanBookmark`.
    Bookmark(u8),
    Tab,
    Break,
}

fn text_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9]{0,6}".prop_map(String::from)
}

fn run_strategy() -> impl Strategy<Value = InlineSpec> {
    (any::<bool>(), any::<bool>(), text_strategy())
        .prop_map(|(bold, italic, text)| InlineSpec::Run { bold, italic, text })
}

fn inline_list_strategy() -> impl Strategy<Value = Vec<InlineSpec>> {
    prop::collection::vec(
        prop_oneof![
            6 => run_strategy(),
            1 => prop::collection::vec(text_strategy(), 0..2).prop_map(InlineSpec::Link),
            1 => prop::collection::vec(text_strategy(), 0..2).prop_map(InlineSpec::Field),
            1 => any::<u8>().prop_map(InlineSpec::Bookmark),
            1 => Just(InlineSpec::Tab),
            1 => Just(InlineSpec::Break),
        ],
        0..4,
    )
}

fn blocks_strategy() -> impl Strategy<Value = Vec<Block>> {
    let leaf = inline_list_strategy().prop_map(Block::Paragraph);
    let block = leaf.prop_recursive(3, 24, 3, |child| {
        let cell = prop::collection::vec(child, 0..3);
        let row = prop::collection::vec(cell, 0..3);
        prop::collection::vec(row, 0..3).prop_map(Block::Table)
    });
    // Хотя бы один блок: иначе обход проверялся бы на пустом теле.
    prop::collection::vec(block, 1..4)
}

/// XML-тело документа: блоки и, иногда, завершающий `w:sectPr` — он проверяет
/// ветку секций, у которых собственный идентификатор есть.
fn body_xml_strategy() -> impl Strategy<Value = String> {
    (blocks_strategy(), any::<bool>()).prop_map(|(blocks, sect_pr)| {
        let mut body = render_blocks(&blocks);
        if sect_pr {
            body.push_str("<w:sectPr/>");
        }
        body
    })
}

fn render_blocks(blocks: &[Block]) -> String {
    blocks.iter().map(render_block).collect()
}

fn render_block(block: &Block) -> String {
    match block {
        Block::Paragraph(inlines) => {
            let inlines: String = inlines.iter().map(render_inline).collect();
            format!("<w:p>{inlines}</w:p>")
        }
        Block::Table(rows) => {
            let mut xml = String::from("<w:tbl>");
            for row in rows {
                xml.push_str("<w:tr>");
                for cell in row {
                    xml.push_str("<w:tc>");
                    xml.push_str(&render_blocks(cell));
                    xml.push_str("</w:tc>");
                }
                xml.push_str("</w:tr>");
            }
            xml.push_str("</w:tbl>");
            xml
        }
    }
}

fn render_inline(inline: &InlineSpec) -> String {
    match inline {
        InlineSpec::Run { bold, italic, text } => {
            let bold = if *bold {
                "<w:b/>"
            } else {
                r#"<w:b w:val="0"/>"#
            };
            let italic = if *italic {
                "<w:i/>"
            } else {
                r#"<w:i w:val="0"/>"#
            };
            format!(r#"<w:r><w:rPr>{bold}{italic}</w:rPr><w:t>{text}</w:t></w:r>"#)
        }
        InlineSpec::Link(runs) => format!(
            r#"<w:hyperlink w:anchor="mark">{}</w:hyperlink>"#,
            render_plain_runs(runs)
        ),
        InlineSpec::Field(runs) => format!(
            r#"<w:fldSimple w:instr="PAGE">{}</w:fldSimple>"#,
            render_plain_runs(runs)
        ),
        InlineSpec::Bookmark(id) => {
            format!(r#"<w:bookmarkStart w:id="{id}" w:name="bm{id}"/><w:bookmarkEnd w:id="{id}"/>"#)
        }
        InlineSpec::Tab => "<w:r><w:tab/></w:r>".to_owned(),
        InlineSpec::Break => "<w:r><w:br/></w:r>".to_owned(),
    }
}

fn render_plain_runs(texts: &[String]) -> String {
    texts
        .iter()
        .map(|text| format!("<w:r><w:t>{text}</w:t></w:r>"))
        .collect()
}

// ---------------------------------------------------------------------------
// Генератор графа `basedOn`
// ---------------------------------------------------------------------------

/// Дефект, заложенный генератором, — ровно один на компоненту графа, поэтому
/// ожидаемый набор предупреждений известен заранее.
#[derive(Debug, Clone, Copy)]
enum Defect {
    None,
    Dangling,
    Cycle,
    Both,
}

/// Стиль-основа: нет, существующий стиль или отсутствующий стиль.
#[derive(Debug, Clone)]
enum Base {
    None,
    Style(usize),
    Missing(String),
}

fn based_on_graph() -> impl Strategy<Value = (Defect, Vec<Base>)> {
    prop::sample::select(vec![
        Defect::None,
        Defect::Dangling,
        Defect::Cycle,
        Defect::Both,
    ])
    .prop_flat_map(|defect| {
        // Больше 20 стилей не нужно: цепочка обязана оставаться короче предела
        // обхода (`MAX_BASED_ON_DEPTH`), иначе к дефекту добавится `DeepNesting`.
        let count = if matches!(defect, Defect::Both) {
            2..=20usize
        } else {
            1..=20usize
        };
        (Just(defect), count, prop::collection::vec(any::<u8>(), 20))
            .prop_map(move |(defect, count, keys)| (defect, graph(defect, count, &keys)))
    })
}

/// Строит граф `count` стилей с единственным дефектом заданного вида.
fn graph(defect: Defect, count: usize, keys: &[u8]) -> Vec<Base> {
    let order = permutation(count, keys);
    let mut bases = vec![Base::None; count];
    match defect {
        // Лес без дефектов: основа — стиль с меньшим номером, цикл невозможен.
        Defect::None => {
            for index in 0..count {
                if index > 0 && keys[index] % 2 == 1 {
                    bases[index] = Base::Style(usize::from(keys[index]) % index);
                }
            }
        }
        // Цепочка, последнее звено которой ссылается на отсутствующий стиль.
        Defect::Dangling => {
            link_chain(&mut bases, &order, Base::Missing("Ghost".to_owned()));
        }
        // Замкнутая в кольцо цепочка: цикл длиной `count`.
        Defect::Cycle => {
            link_chain(&mut bases, &order, Base::Style(order[0]));
        }
        // Две независимые компоненты: обрыв и цикл.
        Defect::Both => {
            let (path, cycle) = order.split_at(count / 2);
            link_chain(&mut bases, path, Base::Missing("Ghost".to_owned()));
            link_chain(&mut bases, cycle, Base::Style(cycle[0]));
        }
    }
    bases
}

/// Замыкает перестановку в цепочку `order[0] → order[1] → … → last_base`.
fn link_chain(bases: &mut [Base], order: &[usize], last: Base) {
    for (index, &style) in order.iter().enumerate() {
        bases[style] = if index + 1 < order.len() {
            Base::Style(order[index + 1])
        } else {
            last.clone()
        };
    }
}

/// Детерминированная перестановка `0..count` по ключам генератора.
fn permutation(count: usize, keys: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&index| (keys.get(index).copied().unwrap_or(0), index));
    order
}

fn style_id(index: usize) -> String {
    format!("S{index:02}")
}

fn styles_xml(bases: &[Base]) -> String {
    let mut body = String::new();
    for (index, base) in bases.iter().enumerate() {
        let id = style_id(index);
        match base {
            Base::None => {
                body.push_str(&format!(r#"<w:style w:type="paragraph" w:styleId="{id}"/>"#));
            }
            Base::Style(other) => body.push_str(&format!(
                r#"<w:style w:type="paragraph" w:styleId="{id}"><w:basedOn w:val="{}"/></w:style>"#,
                style_id(*other)
            )),
            Base::Missing(name) => body.push_str(&format!(
                r#"<w:style w:type="paragraph" w:styleId="{id}"><w:basedOn w:val="{name}"/></w:style>"#
            )),
        }
    }
    format!(r#"<w:styles xmlns:w="{W_NS}">{body}</w:styles>"#)
}

/// Ожидаемые предупреждения: цикл и обрыв предупреждаются по одному разу на
/// компоненту (`warnings` в модели — список видов, поэтому сравнение
/// мультимножеством, порядок зависит от порядка обхода).
fn expected_warnings(defect: Defect) -> Vec<WarningKind> {
    match defect {
        Defect::None => Vec::new(),
        Defect::Dangling => vec![WarningKind::MissingStyleRef],
        Defect::Cycle => vec![WarningKind::CyclicBasedOn],
        Defect::Both => vec![WarningKind::MissingStyleRef, WarningKind::CyclicBasedOn],
    }
}

// ---------------------------------------------------------------------------
// Генератор значений `w:val` тумблера
// ---------------------------------------------------------------------------

/// Значение `w:val`: атрибута нет, известное спецификации значение или
/// незнакомое (сюда же пустое — это не `ST_OnOff`).
#[derive(Debug, Clone)]
enum ToggleVal {
    Absent,
    Known(&'static str),
    Unknown(&'static str),
}

/// Таблица «значение `w:val` → ожидание» из ECMA-376 Part 1 §17.17.4
/// (`ST_OnOff`): `on`/`true`/`1` — включено, `off`/`false`/`0` — выключено.
/// Третий вариант, `Inherit`, — состояние «наследуется от стиля»: оно описано
/// доккомментарием [`Toggle`] и потому не сводится к bool.
const ON_VALUES: [&str; 3] = ["on", "true", "1"];
const OFF_VALUES: [&str; 3] = ["off", "false", "0"];
const INHERIT_VALUES: [&str; 1] = ["inherit"];
/// Не `ST_OnOff`: пустая строка и слова, которых нет в перечислении.
const UNKNOWN_VALUES: [&str; 4] = ["", "yes", "2", "bogus"];

fn toggle_value() -> impl Strategy<Value = ToggleVal> {
    prop_oneof![
        1 => Just(ToggleVal::Absent),
        6 => prop::sample::select(
            ON_VALUES
                .iter()
                .chain(&OFF_VALUES)
                .chain(&INHERIT_VALUES)
                .copied()
                .collect::<Vec<_>>()
        )
        .prop_map(ToggleVal::Known),
        2 => prop::sample::select(UNKNOWN_VALUES.to_vec()).prop_map(ToggleVal::Unknown),
    ]
}

fn toggle_xml(element: &str, value: &ToggleVal) -> String {
    match value {
        ToggleVal::Absent => format!("<w:{element}/>"),
        ToggleVal::Known(raw) | ToggleVal::Unknown(raw) => {
            format!(r#"<w:{element} w:val="{raw}"/>"#)
        }
    }
}

/// Ожидание по таблице: элемент без `w:val` включён (свойство задаёт сам факт
/// элемента), незнакомое значение разбор обязан пометить `InvalidAttribute` и
/// всё равно счесть свойство включённым.
fn toggle_expectation(value: &ToggleVal) -> (Toggle, bool) {
    match value {
        ToggleVal::Absent => (Toggle::On, false),
        ToggleVal::Known(raw) if ON_VALUES.contains(raw) => (Toggle::On, false),
        ToggleVal::Known(raw) if OFF_VALUES.contains(raw) => (Toggle::Off, false),
        ToggleVal::Known(raw) if INHERIT_VALUES.contains(raw) => (Toggle::Inherit, false),
        ToggleVal::Known(raw) => panic!("`{raw}` нет в таблице ожиданий"),
        ToggleVal::Unknown(_) => (Toggle::On, true),
    }
}

fn first_run(document: &Document) -> &Run {
    document
        .body
        .items
        .iter()
        .find_map(|item| match item {
            BlockItem::Paragraph(paragraph) => {
                paragraph.runs.iter().find_map(|inline| match inline {
                    Inline::Run(run) => Some(run),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("сгенерированный документ содержит знак")
}

// ---------------------------------------------------------------------------
// Свойства
// ---------------------------------------------------------------------------

proptest! {
    // Число прогонов на свойство: 256 — умолчание proptest, зафиксировано явно,
    // чтобы вывод отчёта не зависел от версии.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// 1а. Идентификаторы узлов уникальны в пределах документа: у каждого узла
    /// свой `NodeId` (ADR-0019 §2).
    #[test]
    fn node_ids_are_unique(body in body_xml_strategy()) {
        let document = parse(&package_bytes(&document_xml(&body), None));
        let ids = node_ids(&document);
        prop_assert!(ids.len() >= 3, "обход потерял узлы: {:?}", ids);
        let unique: BTreeSet<NodeId> = ids.iter().copied().collect();
        prop_assert_eq!(unique.len(), ids.len(), "идентификатор выдан дважды: {:?}", ids);
    }

    /// 1б. Нумерация детерминирована: те же байты — тот же обход и тот же текст.
    #[test]
    fn node_ids_are_deterministic(body in body_xml_strategy()) {
        let bytes = package_bytes(&document_xml(&body), None);
        let first = parse(&bytes);
        let second = parse(&bytes);
        prop_assert_eq!(node_ids(&second), node_ids(&first));
        prop_assert_eq!(body_text(&second.body), body_text(&first.body));
    }

    /// 2. Обход — свойство пакета, а не ZIP-раскладки: перестановка записей,
    /// иной способ упаковки и посторонняя часть `word/extra.xml` не меняют ни
    /// последовательность идентификаторов, ни текст.
    #[test]
    fn traversal_is_independent_of_zip_layout(
        body in body_xml_strategy(),
        extra in body_xml_strategy(),
        keys in prop::collection::vec(any::<u8>(), 8),
    ) {
        let mut parts = package_parts(&document_xml(&body), None);
        let reference = parse(&zip(&parts, true));
        let reference_ids = node_ids(&reference);
        prop_assert!(reference_ids.len() >= 3, "обход потерял узлы: {:?}", reference_ids);

        parts.push(("word/extra.xml".to_owned(), document_xml(&extra).into_bytes()));
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by_key(|&index| (keys.get(index).copied().unwrap_or(0), index));
        let shuffled: Vec<(String, Vec<u8>)> =
            order.iter().map(|&index| parts[index].clone()).collect();
        let reordered = parse(&zip(&shuffled, false));

        prop_assert_eq!(node_ids(&reordered), reference_ids);
        prop_assert_eq!(body_text(&reordered.body), body_text(&reference.body));
    }

    /// 3. Тринстейт `w:b`/`w:i`: разбор идёт по таблице ECMA-376 §17.17.4, а
    /// незнакомое значение — предупреждение `InvalidAttribute` и включённое
    /// свойство, а не отказ (свойство задаёт присутствие элемента).
    #[test]
    fn toggle_values_follow_the_spec_table(b in toggle_value(), i in toggle_value()) {
        let body = format!(
            "<w:p><w:r><w:rPr>{}{}</w:rPr><w:t>x</w:t></w:r></w:p>",
            toggle_xml("b", &b),
            toggle_xml("i", &i),
        );
        let document = parse(&package_bytes(&document_xml(&body), None));

        let (expected_b, warns_b) = toggle_expectation(&b);
        let (expected_i, warns_i) = toggle_expectation(&i);
        let run = first_run(&document);
        prop_assert_eq!(run.rpr.b, Some(expected_b));
        prop_assert_eq!(run.rpr.i, Some(expected_i));

        let warnings: Vec<_> = document.warnings().iter().collect();
        prop_assert_eq!(
            warnings.len(),
            usize::from(warns_b) + usize::from(warns_i),
            "посторонние предупреждения: {:?}",
            warnings,
        );
        for warning in &warnings {
            prop_assert_eq!(warning.kind, WarningKind::InvalidAttribute);
        }
        for (element, warns) in [("w:b", warns_b), ("w:i", warns_i)] {
            let count = warnings
                .iter()
                .filter(|warning| warning.message.contains(element))
                .count();
            prop_assert_eq!(
                count,
                usize::from(warns),
                "предупреждений о `{}`: {:?}",
                element,
                warnings,
            );
        }
    }

    /// 4. Случайный граф `basedOn` с циклами и ссылками на отсутствующие стили
    /// разбирается за отведённое время и даёт ровно по одному предупреждению на
    /// компоненту: цикл не зацикливает обход, обрыв не теряется.
    #[test]
    fn based_on_graph_terminates_with_expected_warnings(
        (defect, bases) in based_on_graph(),
    ) {
        let document = parse_within_budget(
            &package_bytes(&document_xml("<w:p/>"), Some(&styles_xml(&bases))),
            Duration::from_secs(10),
        );
        prop_assert_eq!(document.styles.paragraph.len(), bases.len());

        let mut actual: Vec<WarningKind> =
            document.warnings().iter().map(|warning| warning.kind).collect();
        let mut expected = expected_warnings(defect);
        actual.sort_by_key(|kind| kind.as_str());
        expected.sort_by_key(|kind| kind.as_str());
        prop_assert_eq!(actual, expected);
    }

    /// 5. Круг `Model → JSON → Model` без потерь на сгенерированных
    /// документах: та же модель и та же строка JSON после обратного разбора
    /// (форма стабильна, порядок ключей `BTreeMap` не плавает).
    #[test]
    fn json_round_trip_is_lossless(
        body in body_xml_strategy(),
        styles in prop::option::of(based_on_graph().prop_map(|(_, bases)| styles_xml(&bases))),
    ) {
        let document = parse(&package_bytes(&document_xml(&body), styles.as_deref()));
        let json = serde_json::to_string(&document).unwrap();
        let decoded: Document = serde_json::from_str(&json).expect("JSON разбирается обратно");
        prop_assert_eq!(&decoded, &document);
        prop_assert_eq!(serde_json::to_string(&decoded).unwrap(), json);
    }
}

/// Таблица §17.17.4 целиком, без сэмплирования: гарантия, что покрыты все
/// строки, а не только выпавшие генератору.
#[test]
fn the_spec_table_is_covered_exhaustively() {
    let cases: Vec<(ToggleVal, Toggle, bool)> = vec![
        (ToggleVal::Absent, Toggle::On, false),
        (ToggleVal::Known("on"), Toggle::On, false),
        (ToggleVal::Known("true"), Toggle::On, false),
        (ToggleVal::Known("1"), Toggle::On, false),
        (ToggleVal::Known("off"), Toggle::Off, false),
        (ToggleVal::Known("false"), Toggle::Off, false),
        (ToggleVal::Known("0"), Toggle::Off, false),
        (ToggleVal::Known("inherit"), Toggle::Inherit, false),
        (ToggleVal::Unknown(""), Toggle::On, true),
        (ToggleVal::Unknown("bogus"), Toggle::On, true),
    ];

    for (value, expected, warns) in cases {
        let body = format!(
            "<w:p><w:r><w:rPr>{}</w:rPr><w:t>x</w:t></w:r></w:p>",
            toggle_xml("b", &value),
        );
        let document = parse(&package_bytes(&document_xml(&body), None));
        assert_eq!(first_run(&document).rpr.b, Some(expected), "{value:?}");
        assert_eq!(document.warnings().len(), usize::from(warns), "{value:?}");
    }
}

/// Снисходительность сверх спецификации: `from_val` регистронезависим и режет
/// пробелы по краям (доккомментарий `Toggle`), хотя `ST_OnOff` пишется строчными
/// и пробелов не допускает. Тест фиксирует это как осознанное решение парсера.
#[test]
fn toggle_values_are_trimmed_and_case_insensitive() {
    for (raw, expected) in [
        ("ON", Toggle::On),
        ("True", Toggle::On),
        (" OFF ", Toggle::Off),
        (" Inherit ", Toggle::Inherit),
    ] {
        let body =
            format!(r#"<w:p><w:r><w:rPr><w:b w:val="{raw}"/></w:rPr><w:t>x</w:t></w:r></w:p>"#,);
        let document = parse(&package_bytes(&document_xml(&body), None));
        assert_eq!(first_run(&document).rpr.b, Some(expected), "{raw}");
        assert!(
            document.warnings().is_empty(),
            "{raw}: {:?}",
            document.warnings()
        );
    }
}
