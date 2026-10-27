//! `content.xml`: тело текстового документа в модель `docx`.
//!
//! Разбор потоковый и однопроходный: `text:h`/`text:p` открывают абзац,
//! `text:span` — прогон, `text:a` — гиперссылку, `table:table` — таблицу,
//! `draw:frame`+`draw:image` — картинку. Пробелы в тексте значимы (`text:s`,
//! `xml:space`), поэтому читатель не обрезает текстовые узлы; фикстуры пишутся
//! генератором без межэлементных пробелов, и `preserving` отдаёт текст ровно
//! как он записан.
//!
//! Каскад знака упрощён: свойства `text:span` накладываются на знаки по
//! умолчанию из стиля абзаца (`style:text-properties` внутри
//! `style:family="paragraph"`). Стили ODT разрешаются на разборе и кладутся
//! прямо в [`RawRPr`]/[`RawPPr`] — таблица стилей DOCX остаётся пустой (см.
//! модуль `odt`).
//!
//! Политика ошибок — как у ODS/ODP: фатально только то, без чего модель
//! недостоверна (нет `content.xml`, битый XML); всё восстановимое — незнакомый
//! элемент, ненайденная картинка — копится в [`Document::warnings`] и не роняет
//! пакет.

use std::collections::BTreeMap;

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_core::{Archive, NodeId, NodeIdAllocator, ParseWarning, WarningKind};
use doc_converter_docx::{
    BlockItem, Body, BreakKind, Cell, CellBorders, CellMargins, CellVAlign, Color, Columns,
    Document, Extent, Field, FieldKind, GridCol, HalfPoint, Hyperlink, Ind, Inline, InlineImage,
    InlineOrAnchor, Margins, MediaPart, Metadata, NumberingTable, Orientation, PageSize, Paragraph,
    ParagraphSpacing, RFonts, RawPPr, RawRPr, Relationships, Row, Run, RunContent, RunEdits,
    Section, SectionProperties, Settings, Shading, ShadingPattern, StyleTable, Table, TableBorders,
    TableLayout, TableLook, Toggle, Twips, Underline,
};
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};

use crate::error::{OdfError, Result};
use crate::manifest::Manifest;
use crate::xml::{find, resolve_reference};

use super::styles::{PageLayoutDef, StyleBook, TextStyle};
use super::{
    attrs, bold_from_weight, color_rgb, italic_from_style, length_emu, length_pt, media_type,
    owned, underline_from_style,
};

/// Предел числа колонок, разворачиваемых одним `table:table-column` повтором.
const MAX_COLUMN_REPEAT: u32 = 1024;

/// Предел предупреждений: как в ODS/ODP, копятся до порога и не роняют пакет.
const MAX_WARNINGS: usize = 256;

/// Предел пробелов в одном `text:s`: враждебный файл не раздует модель.
const MAX_SPACES: u32 = 4096;

/// Разобрать `content.xml` в [`Document`].
///
/// Картинки читаются из архива здесь: `draw:image` ссылается на часть пакета
/// (`Pictures/…`), и байты должны попасть в [`MediaPart`] модели. Секции
/// (раскладки страниц) собираются из мастер-страниц: первая — мастер по
/// умолчанию, смена происходит абзацем со `style:master-page-name`.
///
/// # Errors
/// [`OdfError::Core`] — XML не разбирается или картинка-часть не читается;
/// [`OdfError::Malformed`] — атрибут элемента или текст не читаются.
pub(crate) fn parse(
    bytes: &[u8],
    part: &str,
    styles: &StyleBook,
    manifest: &Manifest,
    archive: &mut Archive,
    metadata: Metadata,
) -> Result<Document> {
    let mut reader = XmlReader::preserving(bytes, part.to_owned());
    let mut alloc = NodeIdAllocator::new();
    // Тело получает собственный узел модели, как у DOCX (`parse_frame`).
    let body_id = alloc.alloc();

    let mut parser = Parser {
        part,
        styles,
        manifest,
        archive,
        alloc,
        body_id,
        metadata,
        body_items: Vec::new(),
        sections: Vec::new(),
        media: Vec::new(),
        media_ids: BTreeMap::new(),
        warnings: Vec::new(),
        current_master: None,
        paragraph: None,
        hyperlink: None,
        run: None,
        frame: None,
        table: None,
        row: None,
        cell: None,
        base_text: TextStyle::default(),
        list_depth: 0,
    };
    parser.init_sections();

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }
    Ok(parser.finish())
}

/// Абзац, который ещё собирается: свойства и inline-содержимое.
struct PendingParagraph {
    ppr: RawPPr,
    runs: Vec<Inline>,
}

impl PendingParagraph {
    /// Собрать [`Paragraph`] с пустыми свойствами знака абзаца.
    fn finish(self, alloc: &mut NodeIdAllocator) -> Paragraph {
        Paragraph {
            id: alloc.alloc(),
            ppr: Box::new(self.ppr),
            mark_rpr: Box::new(RawRPr::default()),
            runs: self.runs,
            style_ref: None,
            numbering_ref: None,
            section_break: None,
        }
    }
}

/// Прогон, который ещё собирается: сырые свойства знака и содержимое.
struct PendingRun {
    rpr: RawRPr,
    content: Vec<RunContent>,
}

impl PendingRun {
    /// Пустой прогон в модель не попадает: `flush_run` его отбрасывает.
    fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    fn finish(self, alloc: &mut NodeIdAllocator) -> Run {
        Run {
            id: alloc.alloc(),
            rpr: Box::new(self.rpr),
            style_ref: None,
            content: self.content,
        }
    }
}

/// Гиперссылка, которая ещё собирается.
struct PendingHyperlink {
    href: String,
    runs: Vec<Inline>,
}

impl PendingHyperlink {
    fn finish(self, alloc: &mut NodeIdAllocator) -> Hyperlink {
        Hyperlink {
            id: alloc.alloc(),
            rel_id: None,
            anchor: None,
            tooltip: None,
            target: (!self.href.is_empty()).then_some(self.href),
            external: true,
            runs: self.runs,
        }
    }
}

/// Картинка `draw:frame`, у которой ещё нет `draw:image`.
struct PendingFrame {
    extent: Extent,
    href: Option<String>,
}

/// Таблица, которая ещё собирается.
struct PendingTable {
    grid: Vec<GridCol>,
    rows: Vec<Row>,
}

impl PendingTable {
    fn finish(self, alloc: &mut NodeIdAllocator) -> Table {
        Table {
            id: alloc.alloc(),
            style_ref: None,
            grid: self.grid,
            rows: self.rows,
            layout: TableLayout::Autofit,
            width: None,
            borders: TableBorders::default(),
            look: TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: CellMargins::default(),
            bidi_visual: false,
        }
    }
}

/// Строка таблицы, которая ещё собирается.
struct PendingRow {
    cells: Vec<Cell>,
}

impl PendingRow {
    fn finish(self, alloc: &mut NodeIdAllocator) -> Row {
        Row {
            id: alloc.alloc(),
            cells: self.cells,
            height: None,
            cant_split: false,
            header: false,
        }
    }
}

/// Ячейка таблицы, которая ещё собирается.
struct PendingCell {
    span: u32,
    fill: Option<Color>,
    items: Vec<BlockItem>,
}

impl PendingCell {
    fn finish(self, alloc: &mut NodeIdAllocator) -> Cell {
        Cell {
            id: alloc.alloc(),
            grid_span: self.span,
            v_merge: None,
            width: None,
            margins: CellMargins::default(),
            v_align: CellVAlign::Top,
            borders: Box::new(CellBorders::default()),
            shading: self.fill.map(|fill| Shading {
                val: ShadingPattern::Clear,
                color: None,
                fill: Some(fill),
            }),
            items: self.items,
        }
    }
}

/// Состояние разбора `content.xml`.
struct Parser<'a> {
    part: &'a str,
    styles: &'a StyleBook,
    manifest: &'a Manifest,
    archive: &'a mut Archive,
    alloc: NodeIdAllocator,
    body_id: NodeId,
    metadata: Metadata,

    body_items: Vec<BlockItem>,
    sections: Vec<Section>,
    media: Vec<MediaPart>,
    /// `xlink:href` картинки → её id в [`Self::media`] (дедупликация).
    media_ids: BTreeMap<String, u32>,
    warnings: Vec<ParseWarning>,

    /// Имя мастер-страницы текущей секции.
    current_master: Option<String>,

    paragraph: Option<PendingParagraph>,
    hyperlink: Option<PendingHyperlink>,
    run: Option<PendingRun>,
    frame: Option<PendingFrame>,
    table: Option<PendingTable>,
    row: Option<PendingRow>,
    cell: Option<PendingCell>,

    /// Знаки по умолчанию текущего абзаца (из его стиля).
    base_text: TextStyle,
    /// Глубина вложенных `text:list`.
    list_depth: u32,
}

impl Parser<'_> {
    fn handle(&mut self, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.start(&element, true),
            Event::Empty(element) => self.start(&element, false),
            Event::End(element) => {
                self.end(element.local_name().as_ref());
                Ok(())
            }
            Event::Text(text) => self.text(&text),
            Event::GeneralRef(reference) => self.reference(&reference),
            _ => Ok(()),
        }
    }

    /// Открыть или обработать элемент; `open` ложно для самозакрытого.
    fn start(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let name = element.local_name();
        match name.as_ref() {
            b"h" => self.open_paragraph(element, open, true),
            b"p" => self.open_paragraph(element, open, false),
            b"span" => self.open_span(element, open),
            b"a" => self.open_hyperlink(element, open),
            b"list" => {
                self.list_depth = self.list_depth.saturating_add(1);
                Ok(())
            }
            b"s" => self.space(element),
            b"tab" => {
                self.push_tab();
                Ok(())
            }
            b"line-break" => {
                self.push_line_break();
                Ok(())
            }
            b"page-number" => {
                self.page_number();
                Ok(())
            }
            b"table" => {
                self.open_table(element, open);
                Ok(())
            }
            b"table-column" => self.table_column(element),
            b"table-row" => {
                self.open_row(element, open);
                Ok(())
            }
            b"table-cell" => self.open_cell(element, open),
            // `covered-table-cell` — дырка в чужом объединении: значения и ячейки
            // у неё нет, поэтому она игнорируется, как прочие неизвестные.
            b"frame" => self.open_frame(element, open),
            b"image" => self.open_image(element),
            _ => Ok(()),
        }
    }

    /// Закрыть элемент.
    fn end(&mut self, name: &[u8]) {
        match name {
            b"p" | b"h" => self.close_paragraph(),
            b"span" => self.flush_run(),
            b"a" => self.close_hyperlink(),
            b"list" => self.list_depth = self.list_depth.saturating_sub(1),
            b"table" => self.close_table(),
            b"table-row" => self.close_row(),
            b"table-cell" => self.close_cell(),
            b"frame" => self.close_frame(),
            _ => {}
        }
    }

    /// Собрать начальную секцию из мастер-страницы по умолчанию.
    fn init_sections(&mut self) {
        let default = self.styles.default_master().map(str::to_owned);
        let layout = default
            .as_deref()
            .and_then(|master| self.styles.page_layout_for_master(master))
            .cloned()
            .unwrap_or_else(default_layout);
        self.current_master = default;
        self.sections
            .push(section_from_layout(&mut self.alloc, &layout));
    }

    /// `text:p`/`text:h`: новый абзац с разрешёнными свойствами стиля.
    fn open_paragraph(
        &mut self,
        element: &BytesStart<'_>,
        open: bool,
        heading: bool,
    ) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let style_name = owned(&attrs, "style-name");
        let style = self.styles.paragraph(style_name.as_deref());

        // Смена мастер-страницы — смена секции; она возможна только на уровне
        // тела, а не внутри ячейки таблицы (там вложенная геометрия не выразима).
        if self.cell.is_none() {
            self.switch_section_if_needed(&style);
        }

        let mut ppr = RawPPr {
            jc: style.align,
            ..RawPPr::default()
        };
        if style.break_before {
            ppr.page_break_before = Some(Toggle::On);
        }
        if style.margin_top.is_some() || style.margin_bottom.is_some() {
            ppr.spacing = Some(ParagraphSpacing {
                before: style.margin_top,
                after: style.margin_bottom,
                ..ParagraphSpacing::default()
            });
        }
        // Отступ слева: стиль абзаца, а для элемента списка — ещё и уровень.
        let left = style.margin_left.or_else(|| {
            (self.list_depth > 0)
                .then(|| Twips::new(720 * i32::try_from(self.list_depth.min(9)).unwrap_or(9)))
        });
        if left.is_some() || style.margin_right.is_some() {
            ppr.ind = Some(Ind {
                left,
                right: style.margin_right,
                ..Ind::default()
            });
        }
        if heading {
            ppr.outline_lvl = heading_level(&attrs, style_name.as_deref());
        }

        self.base_text = style.text;
        self.paragraph = Some(PendingParagraph {
            ppr,
            runs: Vec::new(),
        });
        self.hyperlink = None;
        self.run = Some(PendingRun {
            rpr: rpr_from_text(&self.base_text),
            content: Vec::new(),
        });

        if !open {
            self.close_paragraph();
        }
        Ok(())
    }

    /// `text:span`: прогон со свойствами поверх каскада абзаца.
    fn open_span(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        self.flush_run();
        let attrs = attrs(element, self.part)?;
        let mut text = self.styles.text(owned(&attrs, "style-name").as_deref());
        apply_direct_text(&attrs, &mut text);
        let merged = merge_text(&self.base_text, &text);
        self.run = Some(PendingRun {
            rpr: rpr_from_text(&merged),
            content: Vec::new(),
        });
        if !open {
            self.flush_run();
        }
        Ok(())
    }

    /// `text:a`: гиперссылка со своими прогонами.
    fn open_hyperlink(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        self.flush_run();
        let attrs = attrs(element, self.part)?;
        let href = owned(&attrs, "href").unwrap_or_default();
        let text = self.styles.text(owned(&attrs, "style-name").as_deref());
        let merged = merge_text(&self.base_text, &text);
        self.hyperlink = Some(PendingHyperlink {
            href,
            runs: Vec::new(),
        });
        self.run = Some(PendingRun {
            rpr: rpr_from_text(&merged),
            content: Vec::new(),
        });
        if !open {
            self.close_hyperlink();
        }
        Ok(())
    }

    /// `text:s`: значимые пробелы с повтором `text:c`.
    fn space(&mut self, element: &BytesStart<'_>) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        let count = find(&attrs, "c")
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .min(MAX_SPACES);
        self.push_text(&" ".repeat(count as usize));
        Ok(())
    }

    /// `table:table`: открыть таблицу (только на уровне тела).
    fn open_table(&mut self, _element: &BytesStart<'_>, open: bool) {
        if self.cell.is_some() {
            self.warn("nested `table:table` is not supported and is skipped");
            return;
        }
        self.close_table();
        self.table = Some(PendingTable {
            grid: Vec::new(),
            rows: Vec::new(),
        });
        if !open {
            self.close_table();
        }
    }

    /// `table:table-column`: ширина столбца (с повтором).
    fn table_column(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(table) = self.table.as_mut() else {
            return Ok(());
        };
        let attrs = attrs(element, self.part)?;
        let style_name = owned(&attrs, "style-name");
        let repeat = find(&attrs, "number-columns-repeated")
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .min(MAX_COLUMN_REPEAT);
        let width = self
            .styles
            .column_width(style_name.as_deref())
            .unwrap_or(Twips::new(0));
        for _ in 0..repeat {
            table.grid.push(GridCol { width });
        }
        Ok(())
    }

    /// `table:table-row`: открыть строку.
    fn open_row(&mut self, _element: &BytesStart<'_>, open: bool) {
        if self.table.is_none() {
            return;
        }
        self.close_row();
        self.row = Some(PendingRow { cells: Vec::new() });
        if !open {
            self.close_row();
        }
    }

    /// `table:table-cell`: открыть ячейку (`covered-table-cell` уже отброшен).
    fn open_cell(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.row.is_none() {
            return Ok(());
        }
        self.close_cell();
        let attrs = attrs(element, self.part)?;
        let style_name = owned(&attrs, "style-name");
        let span = find(&attrs, "number-columns-spanned")
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .max(1);
        let fill = self.styles.cell_fill(style_name.as_deref());
        self.cell = Some(PendingCell {
            span,
            fill,
            items: Vec::new(),
        });
        if !open {
            self.close_cell();
        }
        Ok(())
    }

    /// `draw:frame`: размер картинки; `draw:image` добавит ссылку на часть.
    fn open_frame(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        self.flush_run();
        let attrs = attrs(element, self.part)?;
        let extent = Extent {
            cx: find(&attrs, "width").and_then(length_emu).unwrap_or(0),
            cy: find(&attrs, "height").and_then(length_emu).unwrap_or(0),
        };
        self.frame = Some(PendingFrame { extent, href: None });
        if !open {
            self.close_frame();
        }
        Ok(())
    }

    /// `draw:image`: ссылка на часть пакета с картинкой.
    fn open_image(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let href = owned(&attrs, "href");
        match self.frame.as_mut() {
            Some(frame) => {
                frame.href = href;
                if frame.href.is_none() {
                    self.warn("`draw:image` has no `xlink:href`");
                }
            }
            None => self.warn("`draw:image` outside `draw:frame`"),
        }
        Ok(())
    }

    /// `text:page-number` → поле `PAGE`, которое раскладка DOCX подставляет.
    fn page_number(&mut self) {
        if self.paragraph.is_none() {
            return;
        }
        self.flush_run();
        if let Some(paragraph) = self.paragraph.as_mut() {
            paragraph.runs.push(Inline::Field(Field {
                id: self.alloc.alloc(),
                kind: FieldKind::Simple,
                instruction: "PAGE".to_owned(),
                result: Vec::new(),
                dirty: false,
            }));
        }
    }

    /// Смена мастер-страницы абзацем: закрыть текущую секцию и открыть новую.
    fn switch_section_if_needed(&mut self, style: &super::styles::ParagraphStyle) {
        let Some(master) = style.master.as_deref() else {
            return;
        };
        if self.current_master.as_deref() == Some(master) {
            return;
        }
        let Some(layout) = self.styles.page_layout_for_master(master).cloned() else {
            self.warn(format!("master page `{master}` has no page layout"));
            return;
        };
        let section = section_from_layout(&mut self.alloc, &layout);
        // Маркер `SectPr` разрывает секцию; его свойства раскладка берёт из
        // `body.sections`, а не из самого маркера (как у `w:sectPr` в DOCX).
        self.body_items
            .push(BlockItem::SectPr(section.properties.clone()));
        self.current_master = Some(master.to_owned());
        self.sections.push(section);
    }

    /// Закрыть абзац: отдать прогоны в ячейку или в тело.
    fn close_paragraph(&mut self) {
        self.flush_run();
        if let Some(hyperlink) = self.hyperlink.take() {
            if let Some(paragraph) = self.paragraph.as_mut() {
                paragraph
                    .runs
                    .push(Inline::Hyperlink(hyperlink.finish(&mut self.alloc)));
            }
        }
        if let Some(paragraph) = self.paragraph.take() {
            let block = BlockItem::Paragraph(paragraph.finish(&mut self.alloc));
            if let Some(cell) = self.cell.as_mut() {
                cell.items.push(block);
            } else {
                self.body_items.push(block);
            }
        }
        self.run = None;
        self.base_text = TextStyle::default();
    }

    /// Закрыть гиперссылку: прогоны внутри уходят в неё.
    fn close_hyperlink(&mut self) {
        self.flush_run();
        if let Some(hyperlink) = self.hyperlink.take() {
            if let Some(paragraph) = self.paragraph.as_mut() {
                paragraph
                    .runs
                    .push(Inline::Hyperlink(hyperlink.finish(&mut self.alloc)));
            }
        }
        self.run = Some(PendingRun {
            rpr: rpr_from_text(&self.base_text),
            content: Vec::new(),
        });
    }

    /// Закрыть картинку: прочитать байты и положить inline-рисунок в абзац.
    fn close_frame(&mut self) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        let Some(href) = frame.href else {
            self.warn("`draw:frame` has no `draw:image`");
            return;
        };
        // Байты читаются до заимствования абзаца: `media_id` мутирует архив и
        // список медиа, а `paragraph` — поле абзаца, вместе они не уживаются.
        if self.media_id(&href).is_none() {
            return;
        }
        let Some(paragraph) = self.paragraph.as_mut() else {
            return;
        };
        paragraph.runs.push(Inline::Drawing(InlineOrAnchor {
            id: self.alloc.alloc(),
            inline: Some(InlineImage {
                id: self.alloc.alloc(),
                rel_id: href.clone(),
                part: Some(href),
                name: None,
                description: None,
                extent: frame.extent,
            }),
            anchor: None,
        }));
    }

    /// Картинка по `xlink:href`: читается один раз и получает стабильный id.
    #[allow(clippy::cast_possible_truncation)]
    fn media_id(&mut self, href: &str) -> Option<u32> {
        if let Some(&id) = self.media_ids.get(href) {
            return Some(id);
        }
        let bytes = match self.archive.part(href) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.warn(format!("image `{href}` is not readable: {error}"));
                return None;
            }
        };
        let id = self.media.len() as u32;
        let mime = media_type(self.manifest, href);
        self.media.push(MediaPart {
            id,
            part: href.to_owned(),
            mime,
            bytes,
        });
        self.media_ids.insert(href.to_owned(), id);
        Some(id)
    }

    /// Закрыть ячейку: положить её в строку.
    fn close_cell(&mut self) {
        self.close_paragraph();
        if let Some(cell) = self.cell.take() {
            if let Some(row) = self.row.as_mut() {
                row.cells.push(cell.finish(&mut self.alloc));
            }
        }
    }

    /// Закрыть строку: положить её в таблицу.
    fn close_row(&mut self) {
        self.close_cell();
        if let Some(row) = self.row.take() {
            if let Some(table) = self.table.as_mut() {
                table.rows.push(row.finish(&mut self.alloc));
            }
        }
    }

    /// Закрыть таблицу: положить её в тело.
    fn close_table(&mut self) {
        self.close_row();
        if let Some(table) = self.table.take() {
            self.body_items
                .push(BlockItem::Table(table.finish(&mut self.alloc)));
        }
    }

    /// Конец части: незакрытые состояния закрываются, прочитанное не теряем.
    fn finish(mut self) -> Document {
        self.close_frame();
        self.close_hyperlink();
        self.close_paragraph();
        self.close_cell();
        self.close_row();
        self.close_table();

        Document {
            id: NodeId::ROOT,
            body: Body {
                id: self.body_id,
                items: self.body_items,
                sections: self.sections,
            },
            styles: StyleTable::default(),
            numbering: NumberingTable::default(),
            settings: Settings::default(),
            metadata: self.metadata,
            rels: Relationships::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            headers: BTreeMap::new(),
            footers: BTreeMap::new(),
            warnings: self.warnings,
            media: self.media,
            edits: RunEdits::default(),
        }
    }

    /// Текст текстового узла — только внутри абзаца.
    fn text(&mut self, text: &BytesText<'_>) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        let decoded = text.decode().map_err(|error| {
            OdfError::malformed(self.part, format!("text is not UTF-8: {error}"))
        })?;
        self.push_text(&decoded);
        Ok(())
    }

    /// Ссылка на сущность внутри текста.
    fn reference(&mut self, reference: &BytesRef<'_>) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        let decoded = resolve_reference(reference, self.part)?;
        self.push_text(&decoded);
        Ok(())
    }

    /// Дописать текст в текущий прогон (создав его при необходимости).
    fn push_text(&mut self, text: &str) {
        if self.paragraph.is_none() {
            return;
        }
        if self.run.is_none() {
            self.run = Some(PendingRun {
                rpr: rpr_from_text(&self.base_text),
                content: Vec::new(),
            });
        }
        if let Some(run) = self.run.as_mut() {
            if let Some(RunContent::Text(last)) = run.content.last_mut() {
                last.push_str(text);
            } else {
                run.content.push(RunContent::Text(text.to_owned()));
            }
        }
    }

    /// Табуляция в текущий прогон.
    fn push_tab(&mut self) {
        if self.paragraph.is_none() {
            return;
        }
        if self.run.is_none() {
            self.run = Some(PendingRun {
                rpr: rpr_from_text(&self.base_text),
                content: Vec::new(),
            });
        }
        if let Some(run) = self.run.as_mut() {
            run.content.push(RunContent::Tab);
        }
    }

    /// Разрыв строки в текущий прогон.
    fn push_line_break(&mut self) {
        if self.paragraph.is_none() {
            return;
        }
        if self.run.is_none() {
            self.run = Some(PendingRun {
                rpr: rpr_from_text(&self.base_text),
                content: Vec::new(),
            });
        }
        if let Some(run) = self.run.as_mut() {
            run.content.push(RunContent::Break(BreakKind::Line));
        }
    }

    /// Отдать текущий прогон в гиперссылку или абзац, если в нём есть текст.
    fn flush_run(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        if run.is_empty() {
            return;
        }
        if let Some(hyperlink) = self.hyperlink.as_mut() {
            hyperlink
                .runs
                .push(Inline::Run(run.finish(&mut self.alloc)));
        } else if let Some(paragraph) = self.paragraph.as_mut() {
            paragraph
                .runs
                .push(Inline::Run(run.finish(&mut self.alloc)));
        }
    }

    /// Предупреждение: копится до предела, не роняя документ.
    fn warn(&mut self, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings
                .push(ParseWarning::new(WarningKind::UnknownElement, message));
        }
    }
}

/// Секция из раскладки страницы: удобные поля повторяют сырые (как в DOCX).
fn section_from_layout(alloc: &mut NodeIdAllocator, layout: &PageLayoutDef) -> Section {
    let properties = SectionProperties {
        page_size: layout.page_size.clone(),
        orientation: layout.orientation,
        margins: layout.margins.clone(),
        columns: Columns {
            count: 1,
            space: Twips::new(0),
            equal_width: true,
            separator: false,
            defs: Vec::new(),
        },
        ..SectionProperties::default()
    };
    Section {
        id: alloc.alloc(),
        header_default: None,
        header_first: None,
        header_even: None,
        footer_default: None,
        footer_first: None,
        footer_even: None,
        title_pg: false,
        page_size: layout.page_size.clone(),
        orientation: layout.orientation,
        margins: layout.margins.clone(),
        columns: properties.columns.clone(),
        properties,
    }
}

/// Раскладка по умолчанию: A4 книжная с полями в дюйм (как `default_section` DOCX).
fn default_layout() -> PageLayoutDef {
    PageLayoutDef {
        page_size: PageSize {
            width: Twips::new(11906),
            height: Twips::new(16838),
        },
        orientation: Orientation::Portrait,
        margins: Margins {
            top: Twips::new(1440),
            right: Twips::new(1440),
            bottom: Twips::new(1440),
            left: Twips::new(1440),
            header: None,
            footer: None,
            gutter: None,
        },
    }
}

/// Сырые свойства знака из текстового стиля.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn rpr_from_text(text: &TextStyle) -> RawRPr {
    RawRPr {
        sz: text
            .size
            .map(|pt| HalfPoint::new((pt * 2.0).round() as i32)),
        b: text.bold.then_some(Toggle::On),
        i: text.italic.then_some(Toggle::On),
        u: text.underline.then_some(Underline::Single),
        color: text.color,
        r_fonts: text.font.as_ref().map(|font| RFonts {
            ascii: Some(font.clone()),
            ..RFonts::default()
        }),
        ..RawRPr::default()
    }
}

/// Наложить текстовый стиль поверх базового: булевы начертания объединяются ИЛИ,
/// скалярные поля (кегль, цвет, шрифт) перекрываются только если заданы.
fn merge_text(base: &TextStyle, overlay: &TextStyle) -> TextStyle {
    TextStyle {
        size: overlay.size.or(base.size),
        bold: base.bold || overlay.bold,
        italic: base.italic || overlay.italic,
        underline: base.underline || overlay.underline,
        color: overlay.color.or(base.color),
        font: overlay.font.clone().or_else(|| base.font.clone()),
    }
}

/// Прямые атрибуты `text:span` поверх свойств его стиля.
fn apply_direct_text(attrs: &[Attr<'_>], text: &mut TextStyle) {
    if let Some(size) = find(attrs, "font-size").and_then(length_pt) {
        text.size = Some(size);
    }
    if let Some(weight) = find(attrs, "font-weight") {
        text.bold = bold_from_weight(weight);
    }
    if let Some(style) = find(attrs, "font-style") {
        text.italic = italic_from_style(style);
    }
    if let Some(underline) = find(attrs, "text-underline-style") {
        text.underline = underline_from_style(underline);
    }
    if let Some(color) = find(attrs, "color").and_then(color_rgb) {
        text.color = Some(color);
    }
    if let Some(font) = find(attrs, "font-name") {
        text.font = Some(font.to_owned());
    }
}

/// Уровень заголовка 0..=8: `text:outline-level` (1..=9) или имя `Heading_N`.
fn heading_level(attrs: &[Attr<'_>], style_name: Option<&str>) -> Option<u8> {
    if let Some(level) = find(attrs, "outline-level")
        .and_then(|raw| raw.trim().parse::<u8>().ok())
        .filter(|level| (1..=9).contains(level))
    {
        return Some(level - 1);
    }
    style_name
        .and_then(|name| name.strip_prefix("Heading_"))
        .and_then(|level| level.parse::<u8>().ok())
        .filter(|level| (1..=9).contains(level))
        .map(|level| level - 1)
}
