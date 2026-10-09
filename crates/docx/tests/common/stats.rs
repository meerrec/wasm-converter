//! Обход разобранной модели DOCX: счётчики узлов и оценка памяти.
//!
//! Модуль лежит в `tests/common/` и подключается через `#[path]`, как
//! `tests/common/package.rs` у xlsx: общих тестовых крейтов в workspace нет, а
//! держать обходчик рядом с замеряющим тестом дешевле, чем дублировать его.
//!
//! Считаются три вещи:
//!
//! * **счётчики** — абзацы, run'ы, ячейки, строки, таблицы, гиперссылки, поля,
//!   закладки, рисунки и неподдержанные элементы, плюс суммарная длина текста;
//! * **`sized`** — Σ `size_of::<T>()` по слотам, лежащим в куче по значению
//!   (`Vec<BlockItem>`, `Vec<Inline>`, `Vec<RunContent>`, `Box<RawRPr>`,
//!   значения `BTreeMap`). Слот считается один раз — по типу, который в нём
//!   лежит; вложенный в вариант узел (`Run` внутри `Inline`, `Paragraph` внутри
//!   `BlockItem`) отдельно не считается, его байты уже в размере слота;
//! * **`heap`** — оценка динамической памяти: `String::capacity()` у строк и
//!   `Vec::capacity() * size_of::<T>()` у векторов.
//!
//! `heap` — именно **оценка**, а не замер аллокатора: накладные расходы
//! аллокатора (заголовок блока, округление до класса размера) и выравнивание не
//! учтены, а `capacity` бывает заметно больше длины. Годится она для сравнения
//! фикстур между собой и для ловли регрессий порядка — «абзац стал вдвое
//! тяжелее», — а не как абсолютная величина.
//!
//! Глубже узлов обход не идёт: внутренние коллекции сырых свойств
//! (`RawPPr::tabs`, `RawPPr::unknown`, `RawRPr::unknown`) не обходятся, хотя сами
//! `RawPPr`/`RawRPr` посчитаны.

#![allow(dead_code)]

use std::mem::size_of;

use doc_converter_core::rels::Relationship;
use doc_converter_core::{ParseWarning, WarningLocation};
use doc_converter_docx::{
    AbstractNum, Anchor, BlockItem, Body, Bookmark, BreakKind, Cell, CellBorders, CharacterStyle,
    Comment, Document, Field, Footnote, GridCol, HeaderFooter, Hyperlink, Inline, InlineImage,
    InlineOrAnchor, Lvl, LvlOverride, Metadata, Num, NumberingStyle, NumberingTable, Paragraph,
    ParagraphStyle, PartRef, RawPPr, RawRPr, Row, Run, RunContent, Section, SectionProperties,
    Settings, StyleId, StyleTable, Table, TableStyle,
};

/// Счётчики узлов модели — по полю на тип узла.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Абзацы (`Paragraph`) любой вложенности, включая абзацы ячеек.
    pub paragraphs: usize,
    /// Run'ы (`Run`).
    pub runs: usize,
    /// Ячейки таблиц (`Cell`).
    pub cells: usize,
    /// Строки таблиц (`Row`).
    pub rows: usize,
    /// Таблицы (`Table`), включая вложенные в ячейки.
    pub tables: usize,
    /// Гиперссылки (`Hyperlink`).
    pub hyperlinks: usize,
    /// Поля, простые и составные (`Field`).
    pub fields: usize,
    /// Начала закладок (`Bookmark`).
    pub bookmarks: usize,
    /// Рисунки: `RunContent::Drawing` и `Inline::Drawing` вместе — это один узел модели.
    pub drawings: usize,
    /// Неподдержанные элементы: `BlockItem::Unknown`, `Inline::Unknown`, `RunContent::Unknown`.
    pub unknown: usize,
    /// Суммарная длина текста (`RunContent::Text`) в байтах UTF-8.
    pub text_bytes: usize,
}

/// Байты одной группы узлов: тело, сноски и служебные таблицы считаются порознь,
/// иначе вес колонтитулов растворился бы в весе тела.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Group {
    /// Счётчики узлов группы.
    pub nodes: Counters,
    /// Σ `size_of::<T>()` по слотам, лежащим в куче по значению.
    pub sized: usize,
    /// Оценка динамической памяти: ёмкости строк и векторов.
    pub heap: usize,
}

impl Group {
    /// Прибавляет другую группу — агрегат по категории или по всему корпусу.
    pub fn merge(&mut self, other: &Self) {
        let (mine, theirs) = (&mut self.nodes, &other.nodes);
        mine.paragraphs += theirs.paragraphs;
        mine.runs += theirs.runs;
        mine.cells += theirs.cells;
        mine.rows += theirs.rows;
        mine.tables += theirs.tables;
        mine.hyperlinks += theirs.hyperlinks;
        mine.fields += theirs.fields;
        mine.bookmarks += theirs.bookmarks;
        mine.drawings += theirs.drawings;
        mine.unknown += theirs.unknown;
        mine.text_bytes += theirs.text_bytes;
        self.sized += other.sized;
        self.heap += other.heap;
    }

    /// Слот, лежащий по значению: элемент `Vec`, значение `Box` или `BTreeMap`.
    fn node<T>(&mut self) {
        self.sized += size_of::<T>();
    }

    /// Вектор: элементы лежат в куче, поэтому важна ёмкость, а не длина.
    // `&Vec<T>`, а не срез: у среза ёмкости нет, а оценке нужна именно она.
    #[allow(clippy::ptr_arg)]
    fn vec<T>(&mut self, items: &Vec<T>) {
        self.heap += items.capacity() * size_of::<T>();
    }

    /// Строка: заголовок `String` уже посчитан вместе со своим узлом, здесь — текст.
    // `&String`, а не `&str`: нужна ёмкость, у среза её нет.
    #[allow(clippy::ptr_arg)]
    fn string(&mut self, text: &String) {
        self.heap += text.capacity();
    }

    /// Необязательное строковое поле.
    fn opt_string(&mut self, text: &Option<String>) {
        if let Some(text) = text {
            self.string(text);
        }
    }

    /// Ссылка на стиль.
    ///
    /// У `StyleId` поле приватное, `capacity()` не достать — берём длину: у
    /// идентификаторов из XML она почти всегда равна ёмкости.
    fn style_ref(&mut self, id: &StyleId) {
        self.heap += id.as_str().len();
    }

    /// Необязательная ссылка на стиль.
    fn opt_style_ref(&mut self, id: &Option<StyleId>) {
        if let Some(id) = id {
            self.style_ref(id);
        }
    }
}

/// Статистика по частям документа: тело, сноски, комментарии, колонтитулы и
/// служебные таблицы считаются порознь — они тоже часть документа, и молчание о
/// них исказило бы картину.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Блоки тела документа и его секции.
    pub body: Group,
    /// Сноски (`word/footnotes.xml`).
    pub footnotes: Group,
    /// Концевые сноски (`word/endnotes.xml`).
    pub endnotes: Group,
    /// Комментарии (`word/comments.xml`).
    pub comments: Group,
    /// Колонтитулы (`word/header*.xml`).
    pub headers: Group,
    /// Нижние колонтитулы (`word/footer*.xml`).
    pub footers: Group,
    /// Таблица стилей (`word/styles.xml`).
    pub styles: Group,
    /// Таблица нумерации (`word/numbering.xml`).
    pub numbering: Group,
    /// Остальное: метаданные, связи, параметры документа, предупреждения разбора.
    pub rest: Group,
}

impl Stats {
    /// Обходит документ целиком.
    #[must_use]
    pub fn of(document: &Document) -> Self {
        let mut stats = Self::default();

        body_stats(&document.body, &mut stats.body);

        stats.footnotes.vec(&document.footnotes);
        for note in &document.footnotes {
            footnote(note, &mut stats.footnotes);
        }
        stats.endnotes.vec(&document.endnotes);
        for note in &document.endnotes {
            footnote(note, &mut stats.endnotes);
        }
        stats.comments.vec(&document.comments);
        for comment in &document.comments {
            comment_stats(comment, &mut stats.comments);
        }
        for (part, header) in &document.headers {
            header_footer(part, header, &mut stats.headers);
        }
        for (part, footer) in &document.footers {
            header_footer(part, footer, &mut stats.footers);
        }

        style_stats(&document.styles, &mut stats.styles);
        numbering_stats(&document.numbering, &mut stats.numbering);
        rest(document, &mut stats.rest);
        stats
    }

    /// Прибавляет статистику другого документа — агрегат по категории или корпусу.
    pub fn merge(&mut self, other: &Self) {
        for (mine, theirs) in self.fields_mut().into_iter().zip(other.fields()) {
            mine.merge(theirs);
        }
    }

    /// Строки отчёта в порядке печати: часть документа и её байты.
    #[must_use]
    pub fn rows(&self) -> [(&'static str, Group); 9] {
        [
            ("body", self.body),
            ("footnotes", self.footnotes),
            ("endnotes", self.endnotes),
            ("comments", self.comments),
            ("headers", self.headers),
            ("footers", self.footers),
            ("styles", self.styles),
            ("numbering", self.numbering),
            ("rest", self.rest),
        ]
    }

    /// Итог по всему документу — сумма всех частей.
    #[must_use]
    pub fn total(&self) -> Group {
        let mut total = Group::default();
        for (_, group) in self.rows() {
            total.merge(&group);
        }
        total
    }

    /// Части документа в порядке [`Stats::rows`].
    fn fields(&self) -> [&Group; 9] {
        [
            &self.body,
            &self.footnotes,
            &self.endnotes,
            &self.comments,
            &self.headers,
            &self.footers,
            &self.styles,
            &self.numbering,
            &self.rest,
        ]
    }

    /// Те же части на запись — для [`Stats::merge`].
    fn fields_mut(&mut self) -> [&mut Group; 9] {
        [
            &mut self.body,
            &mut self.footnotes,
            &mut self.endnotes,
            &mut self.comments,
            &mut self.headers,
            &mut self.footers,
            &mut self.styles,
            &mut self.numbering,
            &mut self.rest,
        ]
    }
}

// ---------------------------------------------------------------------------
// Обход модели
// ---------------------------------------------------------------------------

/// Тело: блоки и секции.
fn body_stats(body: &Body, g: &mut Group) {
    g.node::<Body>();
    block_stats(&body.items, g);
    g.vec(&body.sections);
    for section in &body.sections {
        g.node::<Section>();
        section_parts(&section.properties, g);
        for part in [
            &section.header_default,
            &section.header_first,
            &section.header_even,
            &section.footer_default,
            &section.footer_first,
            &section.footer_even,
        ]
        .into_iter()
        .flatten()
        {
            part_ref(part, g);
        }
    }
}

/// Блоки: абзацы, таблицы и следы неподдержанного. Таблица заходит в ячейки, а те
/// снова в блоки — вложенность любая, поэтому обход рекурсивный.
// `&Vec<T>`, а не срез: нужна ёмкость вектора, у среза её нет.
#[allow(clippy::ptr_arg)]
fn block_stats(items: &Vec<BlockItem>, g: &mut Group) {
    g.vec(items);
    for item in items {
        match item {
            BlockItem::Paragraph(paragraph) => paragraph_stats(paragraph, g),
            BlockItem::Table(table) => table_stats(table, g),
            BlockItem::SectPr(_) => g.node::<BlockItem>(),
            BlockItem::Unknown { xml, .. } => {
                g.nodes.unknown += 1;
                g.node::<BlockItem>();
                g.string(xml);
            }
        }
    }
}

/// Абзац: свойства, содержимое и разрыв секции.
fn paragraph_stats(paragraph: &Paragraph, g: &mut Group) {
    g.nodes.paragraphs += 1;
    g.node::<Paragraph>();
    g.node::<RawPPr>(); // `Box<RawPPr>`: по значению лежит только указатель
    g.node::<RawRPr>(); // `Box<RawRPr>`
    if paragraph.section_break.is_some() {
        g.node::<SectionProperties>(); // `Box<SectionProperties>`
    }
    g.opt_style_ref(&paragraph.style_ref);
    inline_stats(&paragraph.runs, g);
}

/// Inline-содержимое абзаца: run'ы, ссылки, закладки, поля и рисунки.
// `&Vec<T>`, а не срез: нужна ёмкость вектора.
#[allow(clippy::ptr_arg)]
fn inline_stats(items: &Vec<Inline>, g: &mut Group) {
    g.vec(items);
    for item in items {
        g.node::<Inline>();
        match item {
            Inline::Run(run) => run_stats(run, g),
            Inline::Hyperlink(link) => {
                g.nodes.hyperlinks += 1;
                g.node::<Hyperlink>();
                for text in [&link.rel_id, &link.anchor, &link.tooltip, &link.target]
                    .into_iter()
                    .flatten()
                {
                    g.string(text);
                }
                inline_stats(&link.runs, g);
            }
            Inline::Bookmark(bookmark) => {
                g.nodes.bookmarks += 1;
                g.node::<Bookmark>();
                g.string(&bookmark.name);
            }
            Inline::Field(field) => field_stats(field, g),
            Inline::Break(kind) => break_kind(kind, g),
            Inline::Tab => {}
            Inline::Symbol { font, .. } => g.string(font),
            Inline::Drawing(drawing) => drawing_stats(drawing, g),
            Inline::Unknown { xml, .. } => {
                g.nodes.unknown += 1;
                g.string(xml);
            }
        }
    }
}

/// Run: свойства знака и содержимое.
fn run_stats(run: &Run, g: &mut Group) {
    g.nodes.runs += 1;
    g.node::<Run>();
    g.node::<RawRPr>(); // `Box<RawRPr>`
    g.opt_style_ref(&run.style_ref);
    g.vec(&run.content);
    for item in &run.content {
        g.node::<RunContent>();
        match item {
            RunContent::Text(text) => {
                g.string(text);
                g.nodes.text_bytes += text.len();
            }
            RunContent::Tab => {}
            RunContent::Break(kind) => break_kind(kind, g),
            RunContent::Symbol { font, .. } => g.string(font),
            RunContent::Drawing(drawing) => drawing_stats(drawing, g),
            RunContent::Unknown { xml, .. } => {
                g.nodes.unknown += 1;
                g.string(xml);
            }
        }
    }
}

/// Поле: инструкция и результат, в котором снова inline-содержимое.
fn field_stats(field: &Field, g: &mut Group) {
    g.nodes.fields += 1;
    g.node::<Field>();
    g.string(&field.instruction);
    inline_stats(&field.result, g);
}

/// Разрыв: у неизвестного вида в модели сохраняется его значение строкой.
fn break_kind(kind: &BreakKind, g: &mut Group) {
    if let BreakKind::Unsupported(value) = kind {
        g.string(value);
    }
}

/// Таблица: сетка, строки и ячейки; в ячейках снова блоки.
fn table_stats(table: &Table, g: &mut Group) {
    g.nodes.tables += 1;
    g.node::<Table>();
    g.opt_style_ref(&table.style_ref);
    g.vec(&table.grid);
    for _ in &table.grid {
        g.node::<GridCol>();
    }
    g.vec(&table.rows);
    for row in &table.rows {
        g.nodes.rows += 1;
        g.node::<Row>();
        g.vec(&row.cells);
        for cell in &row.cells {
            g.nodes.cells += 1;
            g.node::<Cell>();
            g.node::<CellBorders>(); // `Box<CellBorders>`
            block_stats(&cell.items, g);
        }
    }
}

/// Рисунок: встроенный или плавающий; у обоих — картинка со ссылкой на часть пакета.
fn drawing_stats(drawing: &InlineOrAnchor, g: &mut Group) {
    g.nodes.drawings += 1;
    g.node::<InlineOrAnchor>();
    if let Some(image) = &drawing.inline {
        image_stats(image, g);
    }
    if let Some(anchor) = &drawing.anchor {
        g.node::<Anchor>();
        image_stats(&anchor.image, g);
    }
}

/// Картинка: ссылка на relationship и разрешённая цель.
fn image_stats(image: &InlineImage, g: &mut Group) {
    g.node::<InlineImage>();
    g.string(&image.rel_id);
    for text in [&image.part, &image.name, &image.description]
        .into_iter()
        .flatten()
    {
        g.string(text);
    }
}

/// Сноска: та же модель блоков, что и в теле.
fn footnote(note: &Footnote, g: &mut Group) {
    g.node::<Footnote>();
    block_stats(&note.body, g);
}

/// Комментарий: тело из тех же блоков.
fn comment_stats(comment: &Comment, g: &mut Group) {
    g.node::<Comment>();
    for text in [&comment.author, &comment.initials, &comment.date]
        .into_iter()
        .flatten()
    {
        g.string(text);
    }
    block_stats(&comment.body, g);
}

/// Колонтитул: имя части и тело.
fn header_footer(part: &String, header: &HeaderFooter, g: &mut Group) {
    g.node::<HeaderFooter>();
    g.string(part);
    g.string(&header.part);
    body_stats(&header.body, g);
}

/// Свойства секции: сам `SectionProperties` считает вызывающий — он лежит то в
/// `Box`, то в слоте `BlockItem`, то внутри `Section`, — здесь только их куча.
fn section_parts(properties: &SectionProperties, g: &mut Group) {
    g.vec(&properties.unknown);
    for (name, xml) in &properties.unknown {
        g.string(name);
        g.string(xml);
    }
    for part in [
        &properties.header_default,
        &properties.header_first,
        &properties.header_even,
        &properties.footer_default,
        &properties.footer_first,
        &properties.footer_even,
    ]
    .into_iter()
    .flatten()
    {
        part_ref(part, g);
    }
}

/// Ссылка на колонтитул: два имени части строками.
fn part_ref(part: &PartRef, g: &mut Group) {
    g.string(&part.rel_id);
    g.string(&part.target);
}

/// Таблица стилей.
///
/// Обход поверхностный: узлы стилей и строки-идентификаторы посчитаны, а
/// вложенные в `RawPPr`/`RawRPr` коллекции — нет.
fn style_stats(table: &StyleTable, g: &mut Group) {
    g.node::<StyleTable>();
    for (id, style) in &table.paragraph {
        g.node::<StyleId>();
        g.style_ref(id);
        g.node::<ParagraphStyle>();
        g.opt_string(&style.name);
        alias_stats(&style.aliases, g);
    }
    for (id, style) in &table.character {
        g.node::<StyleId>();
        g.style_ref(id);
        g.node::<CharacterStyle>();
        g.opt_string(&style.name);
        alias_stats(&style.aliases, g);
    }
    for (id, style) in &table.table {
        g.node::<StyleId>();
        g.style_ref(id);
        g.node::<TableStyle>();
        g.opt_string(&style.name);
        alias_stats(&style.aliases, g);
    }
    for id in table.numbering.keys() {
        g.node::<StyleId>();
        g.style_ref(id);
        g.node::<NumberingStyle>();
    }
    for id in [
        &table.defaults.paragraph,
        &table.defaults.character,
        &table.defaults.table,
        &table.defaults.numbering,
    ]
    .into_iter()
    .flatten()
    {
        g.style_ref(id);
    }
}

/// Имена-синонимы стиля: слот `String` и её текст.
fn alias_stats(aliases: &Vec<String>, g: &mut Group) {
    g.vec(aliases);
    for alias in aliases {
        g.node::<String>();
        g.string(alias);
    }
}

/// Таблица нумерации: схемы и ссылающиеся на них списки.
fn numbering_stats(table: &NumberingTable, g: &mut Group) {
    g.node::<NumberingTable>();
    for abstract_num in table.abstract_nums.values() {
        g.node::<AbstractNum>();
        g.opt_string(&abstract_num.nsid);
        g.opt_string(&abstract_num.tmpl);
        g.opt_style_ref(&abstract_num.num_style_link);
        g.opt_style_ref(&abstract_num.style_link);
        for level in abstract_num.levels.values() {
            level_stats(level, g);
        }
    }
    for num in table.nums.values() {
        g.node::<Num>();
        for over in num.overrides.values() {
            g.node::<LvlOverride>();
            if let Some(level) = &over.lvl {
                level_stats(level, g);
            }
        }
    }
}

/// Уровень нумерации: текст метки и ссылка на стиль абзаца.
fn level_stats(level: &Lvl, g: &mut Group) {
    g.node::<Lvl>();
    g.string(&level.lvl_text);
    g.opt_style_ref(&level.pstyle);
}

/// Остальное: метаданные, связи, параметры и предупреждения разбора.
fn rest(document: &Document, g: &mut Group) {
    let meta = &document.metadata;
    g.node::<Metadata>();
    for text in [
        &meta.title,
        &meta.subject,
        &meta.creator,
        &meta.keywords,
        &meta.description,
        &meta.last_modified_by,
        &meta.category,
        &meta.application,
        &meta.created,
        &meta.modified,
    ]
    .into_iter()
    .flatten()
    {
        g.string(text);
    }

    g.node::<Settings>();

    for (rid, rel) in &document.rels.items {
        g.node::<Relationship>();
        g.string(rid);
        for text in [&rel.id, &rel.rel_type, &rel.target]
            .into_iter()
            .chain(rel.target_mode.iter())
        {
            g.string(text);
        }
    }

    g.vec(&document.warnings);
    for warning in &document.warnings {
        g.node::<ParseWarning>();
        g.string(&warning.message);
        if let Some(location) = &warning.location {
            g.node::<WarningLocation>();
            g.string(&location.part);
            g.opt_string(&location.xml_path);
        }
    }
}
