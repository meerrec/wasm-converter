//! Экспорт документа в HTML: самостоятельная страница со встроенными стилями.
//!
//! Вывод — не фрагмент, а документ целиком: `<!DOCTYPE html>`, `<head>` со стилями и
//! полоса набора по размерам последней секции. Разметка семантическая, а не слепок
//! форматирования: заголовки — `<h1>`…`<h9>` по уровню структуры, списки —
//! `<ul>`/`<ol>`/`<li>` по нумерации, таблицы — `<table>` со `<th>` у строк
//! `w:tblHeader`, начертания — `<strong>`, `<em>`, `<u>`, `<del>`. Общее с Markdown
//! (каскад начертаний, уровень заголовка, счётчики списков) берётся из родительского
//! модуля, чтобы экспортёры не расходились.

use std::cmp::Ordering;
use std::fmt::Write as _;

use crate::layout::cascade::{ResolvedParagraph, StyleCache};
use crate::model::inline::Paragraph;
use crate::model::numbering::NumId;
use crate::model::raw::{
    Border, BorderStyle, CellVAlign, Color, Justification, Shading, ShadingPattern,
};
use crate::model::table::{Cell, Table, TableLayout, TableWidth};
use crate::model::BlockItem;
use crate::Document;

use super::grid::{self, Placed};
use super::{
    heading_level, list_ref, paragraph_layout, segments, ListNumbering, Marks, Piece, Segment,
};

/// Экспорт документа в HTML.
///
/// Возвращает самостоятельный документ: ширина и поля берутся из последней секции,
/// шрифт, кегль и цвет — из `docDefaults`, остальное оформление — из встроенной
/// таблицы стилей. Текст и адреса ссылок экранируются; выравнивание абзаца и ячейки
/// выражается классом, а не инлайн-стилем.
#[must_use]
pub fn to_html(document: &Document) -> String {
    let mut html = Html::new(document);
    html.head();
    for item in &document.body.items {
        html.block(item);
    }
    html.end_lists();
    html.out.push_str("</div>\n</body>\n</html>\n");
    html.out
}

/// Оформление, не зависящее от документа: заголовки, таблицы, списки и классы разметки.
///
/// Классы здесь только те, что ставит экспортёр; `h7`…`h9` — не элементы HTML
/// (`display` у неизвестного элемента строчный), и без явного `display: block`
/// заголовки седьмого уровня и глубже встали бы в строку с текстом.
///
/// Границы, заливка и поля ячеек приходят инлайн-стилями: значения у каждой ячейки
/// свои. `border: 0` здесь гасит границу там, где её нет в документе, а поля
/// `0 5.4pt` — умолчание `WordprocessingML` при отсутствии `w:tblCellMar`
/// (108 twips по горизонтали, ноль по вертикали), поэтому в разметку попадают
/// только отличия от него.
const STYLE: &str = "\
h1, h2, h3, h4, h5, h6, h7, h8, h9 {
  font-weight: 700;
  line-height: 1.2;
  margin: 0.8em 0 0.4em;
}
h7, h8, h9 {
  display: block;
  font-size: 1em;
}
h1 { font-size: 2em; }
h2 { font-size: 1.6em; }
h3 { font-size: 1.35em; }
h4 { font-size: 1.15em; }
h5 { font-size: 1.05em; }
h6 { font-size: 1em; }
p { margin: 0 0 0.6em; }
ul, ol { margin: 0.4em 0 0.8em; padding-left: 1.7em; }
li { margin: 0.15em 0; }
table { border-collapse: collapse; margin: 0.6em 0; }
th, td { border: 0; padding: 0 5.4pt; text-align: left; vertical-align: top; }
td p, th p { margin: 0; }
td p + p, th p + p { margin-top: 0.5em; }
img { max-width: 100%; height: auto; }
a { color: #0b57d0; }
.tab { display: inline-block; width: 2em; }
.align-center { text-align: center; }
.align-right { text-align: right; }
.align-justify { text-align: justify; }
table.align-center { margin-left: auto; margin-right: auto; }
table.align-right { margin-left: auto; margin-right: 0; }
.valign-middle { vertical-align: middle; }
.valign-bottom { vertical-align: bottom; }
";

/// Обход тела документа: разрешённые стили, счётчики списков и открытые списки.
struct Html<'a> {
    /// Разбираемый документ: каскад стилей и нумерация нужны на каждом абзаце.
    document: &'a Document,
    /// Кэш каскада — один на документ, как и требует [`StyleCache`].
    cache: StyleCache,
    /// Счётчики пунктов списков; состояние на документ, а не на абзац.
    numbering: ListNumbering,
    /// Открытые списки от внешнего к внутреннему.
    lists: Vec<ListLevel>,
    /// У верхнего списка открыт `<li>`: вложенный список вставляется внутрь него,
    /// и закрыть `<li>` можно, только поняв, что следующий пункт — не вложенный.
    li_open: bool,
    /// Собранный вывод.
    out: String,
}

/// Открытый список: по смене `w:numId` или уровня его пора закрывать.
#[derive(Clone, Copy)]
struct ListLevel {
    /// Ссылка абзаца на нумерацию.
    num_id: NumId,
    /// Уровень пункта (`w:ilvl`).
    ilvl: u8,
    /// Нумерованный список: маркер — число, а не символ.
    ordered: bool,
}

/// Поля страницы в twips: ширина, высота и поля (сверху по часовой стрелке).
struct PageSetup {
    width: i32,
    height: i32,
    margins: [i32; 4],
}

impl Default for PageSetup {
    /// Умолчания `WordprocessingML`, когда `w:sectPr` в документе нет: Letter с полями в дюйм.
    fn default() -> Self {
        Self {
            width: 12240,
            height: 15840,
            margins: [1440; 4],
        }
    }
}

impl<'a> Html<'a> {
    /// Пустая страница: тело ещё не обойдено.
    fn new(document: &'a Document) -> Self {
        Self {
            document,
            cache: StyleCache::new(),
            numbering: ListNumbering::default(),
            lists: Vec::new(),
            li_open: false,
            out: String::new(),
        }
    }

    /// Шапка: заголовок страницы, встроенные стили и открытие полосы набора.
    fn head(&mut self) {
        let document = self.document;
        let title = document
            .metadata
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("Document");
        // Язык документа в модели не разобран (`w:lang` до неё не доезжает), а атрибут
        // `lang` нужен для переносов и доступности — остаётся умолчание Word.
        self.out
            .push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
        self.out.push_str("<meta charset=\"utf-8\">\n");
        self.out
            .push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
        self.out.push_str("<title>");
        self.out.push_str(&escape(title));
        self.out.push_str("</title>\n<style>\n");
        self.stylesheet();
        self.out
            .push_str("</style>\n</head>\n<body>\n<div class=\"page\">\n");
    }

    /// Таблица стилей: шрифт, кегль и цвет документа, полоса набора и разметка.
    fn stylesheet(&mut self) {
        let defaults = &self.document.styles.doc_defaults.r_pr;
        self.out
            .push_str("body {\n  margin: 0;\n  padding: 24px 12px;\n  background: #f3f3f3;\n  line-height: 1.45;\n");
        if let Some(fonts) = defaults.r_fonts.as_ref() {
            if let Some(family) = fonts.ascii.as_deref().or(fonts.h_ansi.as_deref()) {
                self.out.push_str("  font-family: \"");
                self.out.push_str(&css_string(family));
                self.out.push_str("\";\n");
            }
        }
        if let Some(size) = defaults.sz {
            self.out.push_str("  font-size: ");
            push_points(&mut self.out, f64::from(size.value()) / 2.0);
            self.out.push_str(";\n");
        }
        if let Some(Color::Rgb(rgb)) = defaults.color {
            self.out.push_str("  color: ");
            // Цвет в модели — `RRGGBB`, как в CSS, но без ведущего `#`.
            let _ = write!(self.out, "#{rgb:06x}");
            self.out.push_str(";\n");
        }
        self.out.push_str("}\n");
        self.page();
        self.out.push_str(STYLE);
    }

    /// Полоса набора: ширина и высота страницы, поля — из последней секции документа.
    fn page(&mut self) {
        let page = page_setup(self.document);
        self.out
            .push_str(".page {\n  box-sizing: border-box;\n  width: ");
        push_twips(&mut self.out, page.width);
        self.out.push_str(";\n  max-width: 100%;\n  min-height: ");
        push_twips(&mut self.out, page.height);
        self.out.push_str(";\n  margin: 0 auto;\n  padding:");
        for margin in page.margins {
            self.out.push(' ');
            push_twips(&mut self.out, margin);
        }
        self.out
            .push_str(";\n  background: #fff;\n  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2);\n}\n");
    }

    /// Блок тела: абзац или таблица; `w:sectPr` и неподдержанные блоки разметки не имеют.
    fn block(&mut self, item: &BlockItem) {
        match item {
            BlockItem::Paragraph(paragraph) => self.paragraph(paragraph),
            BlockItem::Table(table) => self.table(table),
            BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
        }
    }

    /// Абзац: заголовок, пункт списка или обычный абзац.
    ///
    /// Пустой абзац пропускается — как и в Markdown: пустых абзацев в документе
    /// много, а смысла в `<p></p>` нет. Проверка пустоты идёт до счётчика списка,
    /// чтобы нумерация совпадала с Markdown.
    fn paragraph(&mut self, paragraph: &Paragraph) {
        let document = self.document;
        let resolved = paragraph_layout(document, paragraph, &mut self.cache);
        let content = inline(document, paragraph, &resolved, &mut self.cache);
        if content.trim().is_empty() {
            return;
        }
        let class = class_attribute(alignment_class(resolved.ppr.jc.as_ref()));
        if let Some(level) = heading_level(paragraph, &resolved) {
            self.end_lists();
            self.out.push_str("<h");
            self.out.push_str(&level.to_string());
            self.out.push_str(&class);
            self.out.push('>');
            self.out.push_str(&content);
            self.out.push_str("</h");
            self.out.push_str(&level.to_string());
            self.out.push('>');
        } else if let Some((num_id, ilvl)) = list_ref(paragraph, &resolved) {
            self.list_item(num_id, ilvl, &content, &class);
            return;
        } else {
            self.end_lists();
            self.out.push_str("<p");
            self.out.push_str(&class);
            self.out.push('>');
            self.out.push_str(&content);
            self.out.push_str("</p>");
        }
        self.out.push('\n');
    }

    /// Пункт списка: открывает списки, закрывает кончившиеся и пишет `<li>`.
    ///
    /// Списки `<ol>`/`<ul>` расставляет HTML, номер пункта браузер считает сам;
    /// маркер нужен лишь затем, чтобы отличить маркированный список от
    /// нумерованного и подставить `start`, если список начинается не с единицы.
    fn list_item(&mut self, num_id: NumId, ilvl: u8, content: &str, class: &str) {
        let marker = self
            .numbering
            .marker(&self.document.numbering, num_id, ilvl);
        let ordered = marker != "- ";
        while let Some(top) = self.lists.last() {
            if top.num_id == num_id && top.ilvl == ilvl {
                break;
            }
            if top.ilvl < ilvl {
                // Список выше по уровню не кончился: пункт вкладывается в него.
                break;
            }
            if self.li_open {
                self.out.push_str("</li>");
                self.li_open = false;
            }
            self.out
                .push_str(if top.ordered { "</ol>" } else { "</ul>" });
            self.lists.pop();
            // Возврат внутрь `<li>` родительского списка: вложенный список был в нём.
            self.li_open = !self.lists.is_empty();
        }
        let same = self
            .lists
            .last()
            .is_some_and(|top| top.num_id == num_id && top.ilvl == ilvl);
        if same {
            if self.li_open {
                self.out.push_str("</li>");
                self.li_open = false;
            }
        } else {
            self.out.push_str(if ordered { "<ol" } else { "<ul" });
            let start = if ordered {
                ordered_start(&marker)
            } else {
                None
            };
            if let Some(start) = start {
                if start != 1 {
                    self.out.push_str(" start=\"");
                    self.out.push_str(&start.to_string());
                    self.out.push('"');
                }
            }
            self.out.push('>');
            self.lists.push(ListLevel {
                num_id,
                ilvl,
                ordered,
            });
        }
        self.out.push_str("<li");
        self.out.push_str(class);
        self.out.push('>');
        self.out.push_str(content);
        self.li_open = true;
    }

    /// Закрывает все открытые списки вместе с их незакрытыми `<li>`.
    fn end_lists(&mut self) {
        if self.lists.is_empty() {
            return;
        }
        while let Some(list) = self.lists.pop() {
            if self.li_open {
                self.out.push_str("</li>");
            }
            self.out
                .push_str(if list.ordered { "</ol>" } else { "</ul>" });
            self.li_open = !self.lists.is_empty();
        }
        self.out.push('\n');
    }

    /// Таблица: полоса столбцов, строки и ячейки.
    ///
    /// Вложенная таблица остаётся таблицей — HTML это позволяет, в отличие от
    /// GFM-таблицы Markdown, где вложенность пришлось разворачивать в текст.
    /// Внешний вид ячеек (границы, заливка, поля) выражается инлайн-стилями:
    /// значения у каждой ячейки свои, классом их не выразить.
    fn table(&mut self, table: &Table) {
        self.end_lists();
        let placed = grid::place(table);
        if placed.is_empty() {
            return;
        }
        self.out.push_str("<table");
        self.out
            .push_str(&class_attribute(alignment_class(table.jc.as_ref())));
        let style = table_style(table);
        if !style.is_empty() {
            self.out.push_str(" style=\"");
            self.out.push_str(&style);
            self.out.push('"');
        }
        self.out.push_str(">\n");
        self.colgroup(table);
        let mut next = 0;
        for (index, row) in table.rows.iter().enumerate() {
            self.out.push_str("<tr>");
            while let Some(place) = placed.get(next).filter(|place| place.row == index) {
                // Ячейка-продолжение своего элемента не получает: её место занято
                // `rowspan`-ом начала объединения.
                if !place.continuation {
                    self.cell(table, place, index, row.header);
                }
                next += 1;
            }
            self.out.push_str("</tr>\n");
        }
        self.out.push_str("</table>\n");
    }

    /// Полоса столбцов: ширины `w:tblGrid` в процентах от их суммы.
    ///
    /// Проценты, а не пункты: сетка задаёт доли столбцов, а ширину всей таблицы —
    /// `w:tblW`, который попадает в стиль таблицы. Сетки может не быть вовсе
    /// (`w:tblGrid` необязателен) — тогда столбцы размечает браузер.
    fn colgroup(&mut self, table: &Table) {
        let widths = column_widths(table);
        if widths.is_empty() {
            return;
        }
        self.out.push_str("<colgroup>");
        for width in &widths {
            self.out.push_str("<col style=\"width: ");
            self.out.push_str(width);
            self.out.push_str("%\">");
        }
        self.out.push_str("</colgroup>\n");
    }

    /// Ячейка: объединения — атрибутами, оформление — стилем, содержимое — блоками,
    /// как в теле документа.
    fn cell(&mut self, table: &Table, place: &Placed<'_>, row: usize, header: bool) {
        let tag = if header { "th" } else { "td" };
        self.out.push('<');
        self.out.push_str(tag);
        if place.cols > 1 {
            self.out.push_str(" colspan=\"");
            self.out.push_str(&place.cols.to_string());
            self.out.push('"');
        }
        if place.rows > 1 {
            self.out.push_str(" rowspan=\"");
            self.out.push_str(&place.rows.to_string());
            self.out.push('"');
        }
        let style = cell_style(table, place, row);
        if !style.is_empty() {
            self.out.push_str(" style=\"");
            self.out.push_str(&style);
            self.out.push('"');
        }
        self.out
            .push_str(&class_attribute(v_align_class(place.cell.v_align)));
        self.out.push('>');
        let start = self.out.len();
        for item in &place.cell.items {
            self.block(item);
        }
        self.end_lists();
        // Отбивка последнего блока перед закрывающим тегом — лишняя.
        if self.out.len() > start && self.out.ends_with('\n') {
            self.out.pop();
        }
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }
}

/// Стиль таблицы: ширина (`w:tblW`), фиксированная раскладка и отступ от поля.
///
/// Выравнивание (`w:jc`) — классом: `margin-left/right: auto` в таблице стилей.
/// `w:tblW` типа `auto` ширины не задаёт: Word считает её по содержимому, а в HTML
/// это и есть `width: auto`. `w:tblInd` при выравнивании по центру и справа Word
/// игнорирует — отступ тогда не выводится.
fn table_style(table: &Table) -> String {
    let mut parts: Vec<String> = Vec::new();
    match table.width.as_ref() {
        Some(TableWidth::Pct(pct)) if pct.is_finite() && *pct > 0.0 => {
            parts.push(format!("width: {}%", number(f64::from(*pct))));
        }
        Some(TableWidth::Dxa(twips)) if twips.value() > 0 => {
            parts.push(format!(
                "width: {}pt",
                number(f64::from(twips.value()) / 20.0)
            ));
        }
        Some(TableWidth::Auto | TableWidth::Pct(_) | TableWidth::Dxa(_)) | None => {}
    }
    // Фиксированная раскладка держит ширины столбцов из сетки: так же считает таблицу
    // и Word при `w:tblLayout w:type="fixed"`.
    if table.layout == TableLayout::Fixed && !table.grid.is_empty() {
        parts.push("table-layout: fixed".to_owned());
    }
    let aligned = matches!(
        table.jc,
        Some(Justification::Center | Justification::Right | Justification::End)
    );
    if let Some(indent) = table.indent.filter(|_| !aligned) {
        parts.push(format!(
            "margin-left: {}pt",
            number(f64::from(indent.value().max(0)) / 20.0)
        ));
    }
    parts.join("; ")
}

/// Ширины столбцов в процентах от суммы `w:tblGrid`; пусто — сетки нет или она нулевая.
fn column_widths(table: &Table) -> Vec<String> {
    let total: f64 = table
        .grid
        .iter()
        .map(|column| f64::from(column.width.value().max(0)))
        .sum();
    if total <= 0.0 {
        return Vec::new();
    }
    table
        .grid
        .iter()
        .map(|column| number(f64::from(column.width.value().max(0)) / total * 100.0))
        .collect()
}

/// Оформление ячейки: границы, заливка и поля — только то, что отличается от
/// умолчаний таблицы стилей.
fn cell_style(table: &Table, place: &Placed<'_>, row: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (side, border) in cell_borders(table, place, row) {
        let Some(border) = border else {
            // Граница не нарисована: в стилях её гасит `border: 0` таблицы стилей.
            continue;
        };
        parts.push(format!(
            "border-{side}: {} {} {}",
            border_width(border),
            line_style(&border.val),
            color(border.color.as_ref())
        ));
    }
    if let Some(fill) = fill_color(place.cell.shading.as_ref()) {
        parts.push(format!("background-color: {fill}"));
    }
    if let Some(padding) = padding(table, place.cell) {
        parts.push(format!("padding: {padding}"));
    }
    parts.join("; ")
}

/// Эффективные границы ячейки по четырём сторонам.
///
/// Внешние рёбра таблицы берут её `top`/`left`/`bottom`/`right`, внутренние —
/// `insideH`/`insideV`; на общем ребре двух ячеек остаётся сильнейшая из границ —
/// так же, как схлопывает границы Word (ECMA-376 §17.7.6: ширина, затем стиль).
fn cell_borders<'a>(
    table: &'a Table,
    place: &Placed<'a>,
    row: usize,
) -> [(&'static str, Option<&'a Border>); 4] {
    let borders = &table.borders;
    [
        (
            "top",
            edge(
                place.cell.borders.top.as_ref(),
                &neighbours(table, place, Side::Top),
                if row == 0 {
                    borders.top.as_ref()
                } else {
                    borders.inside_h.as_ref()
                },
            ),
        ),
        (
            "right",
            edge(
                place.cell.borders.right.as_ref(),
                &neighbours(table, place, Side::Right),
                if is_last_column(table, place) {
                    borders.right.as_ref()
                } else {
                    borders.inside_v.as_ref()
                },
            ),
        ),
        (
            "bottom",
            edge(
                place.cell.borders.bottom.as_ref(),
                &neighbours(table, place, Side::Bottom),
                if row + usize::try_from(place.rows).unwrap_or(usize::MAX) >= table.rows.len() {
                    borders.bottom.as_ref()
                } else {
                    borders.inside_h.as_ref()
                },
            ),
        ),
        (
            "left",
            edge(
                place.cell.borders.left.as_ref(),
                &neighbours(table, place, Side::Left),
                if place.col == 0 {
                    borders.left.as_ref()
                } else {
                    borders.inside_v.as_ref()
                },
            ),
        ),
    ]
}

/// Сторона ячейки, по которой ищутся соседи.
#[derive(Clone, Copy)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// Границы соседей по стороне: чужие рёбра, которые смотрят на это.
///
/// У объединённой по вертикали ячейки соседи справа и слева стоят в разных строках
/// и их может быть несколько — своё ребро у каждого.
fn neighbours<'a>(table: &'a Table, place: &Placed<'_>, side: Side) -> Vec<&'a Border> {
    let end = place.end_col();
    let rows = usize::try_from(place.rows).unwrap_or(usize::MAX);
    let mut out = Vec::new();
    match side {
        Side::Top => {
            if let Some(row) = place.row.checked_sub(1).and_then(|row| table.rows.get(row)) {
                for (col, cell, cols) in grid::row_cells(row) {
                    if overlaps(
                        col,
                        col + usize::try_from(cols).unwrap_or(usize::MAX),
                        place.col,
                        end,
                    ) {
                        out.push(cell.borders.bottom.as_ref());
                    }
                }
            }
        }
        Side::Bottom => {
            if let Some(row) = table.rows.get(place.row.saturating_add(rows)) {
                for (col, cell, cols) in grid::row_cells(row) {
                    if overlaps(
                        col,
                        col + usize::try_from(cols).unwrap_or(usize::MAX),
                        place.col,
                        end,
                    ) {
                        out.push(cell.borders.top.as_ref());
                    }
                }
            }
        }
        Side::Left => {
            for row in table.rows.iter().skip(place.row).take(rows) {
                for (col, cell, cols) in grid::row_cells(row) {
                    if col + usize::try_from(cols).unwrap_or(usize::MAX) == place.col {
                        out.push(cell.borders.right.as_ref());
                    }
                }
            }
        }
        Side::Right => {
            for row in table.rows.iter().skip(place.row).take(rows) {
                for (col, cell, _) in grid::row_cells(row) {
                    if col == end {
                        out.push(cell.borders.left.as_ref());
                    }
                }
            }
        }
    }
    out.into_iter().flatten().collect()
}

/// Пересекаются ли отрезки столбцов `[a, a_end)` и `[b, b_end)`.
fn overlaps(a: usize, a_end: usize, b: usize, b_end: usize) -> bool {
    a < b_end && b < a_end
}

/// Ячейка занимает последний столбец таблицы: правого соседа нет.
fn is_last_column(table: &Table, place: &Placed<'_>) -> bool {
    let end = place.end_col();
    table.rows.iter().all(|row| {
        grid::row_cells(row)
            .iter()
            .all(|(col, _, _)| *col < end || *col == place.col)
    })
}

/// Граница ребра: своя у ячейки, своя у соседей, затем табличная.
///
/// Границы ячеек сильнее табличной; на общем ребре двух ячеек остаётся сильнейшая.
/// Между равными по силе выигрывает первая — своя у ячейки, дальше соседи сверху
/// и слева: то же правило, что у Word для равных границ.
fn edge<'a>(
    own: Option<&'a Border>,
    neighbours: &[&'a Border],
    shared: Option<&'a Border>,
) -> Option<&'a Border> {
    let border = strongest(
        own.into_iter()
            .chain(neighbours.iter().copied())
            .filter(|border| draws(border)),
    );
    if border.is_some() {
        return border;
    }
    // Ни ячейка, ни соседи своей линии не рисуют: остаётся граница таблицы.
    // Своя граница без линии (`nil`/`none`) гасит и её — так в Word убирают
    // табличную границу у одной ячейки.
    if own.is_some() {
        return None;
    }
    shared.filter(|border| draws(border))
}

/// Сильнейшая из границ: сначала по толщине, затем по заметности стиля.
fn strongest<'a>(borders: impl IntoIterator<Item = &'a Border>) -> Option<&'a Border> {
    let mut best: Option<&Border> = None;
    for border in borders {
        if best.is_none_or(|current| wins(border, current)) {
            best = Some(border);
        }
    }
    best
}

/// `a` сильнее `b`: толще, а при равной толщине — заметнее по стилю.
fn wins(a: &Border, b: &Border) -> bool {
    match width(a).cmp(&width(b)) {
        Ordering::Equal => style_rank(&a.val) > style_rank(&b.val),
        ordering => ordering.is_gt(),
    }
}

/// Толщина границы в восьмых долях пункта; отсутствующая — как у одиночной линии Word.
fn width(border: &Border) -> u32 {
    border.sz.unwrap_or(4)
}

/// Порядок заметности стилей для схлопывания: приближение порядка `ECMA-376`
/// (§17.7.6) — там, где стили не сравнимы напрямую (штрих-пунктир против волнистой
/// линии), порядок выбран по внешнему виду.
fn style_rank(style: &BorderStyle) -> u8 {
    match style {
        BorderStyle::Nil | BorderStyle::None => 0,
        BorderStyle::Single | BorderStyle::Thick => 1,
        BorderStyle::Dotted
        | BorderStyle::Dashed
        | BorderStyle::DashSmallGap
        | BorderStyle::DotDash
        | BorderStyle::DotDotDash
        | BorderStyle::DashDotStroked => 2,
        BorderStyle::Double | BorderStyle::Triple => 3,
        BorderStyle::Wave | BorderStyle::DoubleWave => 4,
        BorderStyle::ThreeDEmboss
        | BorderStyle::ThreeDEngrave
        | BorderStyle::Inset
        | BorderStyle::Outset => 5,
        // Составные линии с зазором (`thinThickSmallGap` и родственные) и неизвестные
        // стили: в CSS им соответствия нет, но линия заметная.
        BorderStyle::ThinThickSmallGap
        | BorderStyle::ThickThinSmallGap
        | BorderStyle::ThinThickThinSmallGap
        | BorderStyle::ThinThickMediumGap
        | BorderStyle::ThickThinMediumGap
        | BorderStyle::ThinThickThinMediumGap
        | BorderStyle::ThinThickLargeGap
        | BorderStyle::ThickThinLargeGap
        | BorderStyle::ThinThickThinLargeGap
        | BorderStyle::Other(_) => 6,
    }
}

/// Рисует ли граница линию: `nil` и `none` — нет.
fn draws(border: &Border) -> bool {
    !matches!(border.val, BorderStyle::Nil | BorderStyle::None)
}

/// Стиль линии CSS; чего в CSS нет — ближайшее существующее (штрих-пунктир — штрих,
/// волнистая линия — сплошная), чтобы граница осталась видимой и на месте.
fn line_style(style: &BorderStyle) -> &'static str {
    match style {
        BorderStyle::Double
        | BorderStyle::Triple
        | BorderStyle::ThinThickSmallGap
        | BorderStyle::ThickThinSmallGap
        | BorderStyle::ThinThickThinSmallGap
        | BorderStyle::ThinThickMediumGap
        | BorderStyle::ThickThinMediumGap
        | BorderStyle::ThinThickThinMediumGap
        | BorderStyle::ThinThickLargeGap
        | BorderStyle::ThickThinLargeGap
        | BorderStyle::ThinThickThinLargeGap => "double",
        BorderStyle::Dotted | BorderStyle::DotDotDash => "dotted",
        BorderStyle::Dashed
        | BorderStyle::DashSmallGap
        | BorderStyle::DotDash
        | BorderStyle::DashDotStroked => "dashed",
        BorderStyle::ThreeDEmboss => "ridge",
        BorderStyle::ThreeDEngrave => "groove",
        BorderStyle::Inset => "inset",
        BorderStyle::Outset => "outset",
        _ => "solid",
    }
}

/// Толщина линии в пикселях: `w:sz` — восьмые доли пункта, пункт — 4/3 пикселя.
fn border_width(border: &Border) -> String {
    let points = f64::from(width(border)) / 8.0;
    format!("{}px", number(points * 4.0 / 3.0))
}

/// Цвет линии: `auto` и отсутствующий Word рисует основным цветом текста (чёрным).
fn color(color: Option<&Color>) -> String {
    match color {
        Some(Color::Rgb(rgb)) => format!("#{rgb:06x}"),
        Some(Color::Auto | Color::None) | None => "#000000".to_owned(),
    }
}

/// Заливка ячейки (`w:shd w:fill`); `auto` и `nil` заливкой не считаются.
///
/// Узор заливки (`w:val` вида `horzStripe`) в CSS не выражается: берётся цвет фона,
/// а штриховка теряется.
fn fill_color(shading: Option<&Shading>) -> Option<String> {
    let shading = shading?;
    if shading.val == ShadingPattern::Nil {
        return None;
    }
    match shading.fill.as_ref()? {
        Color::Rgb(rgb) => Some(format!("#{rgb:06x}")),
        Color::Auto | Color::None => None,
    }
}

/// Поля ячейки: свои поверх табличных, при отсутствии обоих — умолчание
/// `WordprocessingML` (108 twips по горизонтали, ноль по вертикали).
///
/// `None` — поля совпали с умолчанием, которое уже стоит в таблице стилей.
fn padding(table: &Table, cell: &Cell) -> Option<String> {
    let table_margins = &table.cell_margins;
    // Порядок сторон — как в CSS: сверху, справа, снизу, слева.
    let twips = [
        cell.margins
            .top
            .or(table_margins.top)
            .map_or(0, |twips| twips.value()),
        cell.margins
            .right
            .or(table_margins.right)
            .map_or(108, |twips| twips.value()),
        cell.margins
            .bottom
            .or(table_margins.bottom)
            .map_or(0, |twips| twips.value()),
        cell.margins
            .left
            .or(table_margins.left)
            .map_or(108, |twips| twips.value()),
    ];
    if twips == [0, 108, 0, 108] {
        return None;
    }
    Some(
        twips
            .iter()
            .map(|&twips| format!("{}pt", number(f64::from(twips.max(0)) / 20.0)))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Содержимое абзаца разметкой: `document` нужен каскаду для начертаний и стилей ссылок.
fn inline(
    document: &Document,
    paragraph: &Paragraph,
    resolved: &ResolvedParagraph,
    cache: &mut StyleCache,
) -> String {
    render(&segments(document, resolved, &paragraph.runs, cache))
}

/// Кусок содержимого: экранирование текста, начертания, ссылка и рисунок.
fn render(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match &segment.piece {
            Piece::Text(text) => out.push_str(&mark(escape(text), segment.marks)),
            // Табуляция в HTML — обычный пробел, а колонка в документе — отбивка.
            Piece::Tab => out.push_str("<span class=\"tab\"></span>"),
            // Разрыва страницы в потоке HTML нет: и он, и разрыв колонки — перенос.
            Piece::Break(_) => out.push_str("<br>"),
            Piece::Image { part, alt } => {
                out.push_str("<img src=\"");
                out.push_str(&escape(part));
                out.push_str("\" alt=\"");
                out.push_str(&escape(alt));
                out.push_str("\">");
            }
            Piece::Link { target, inner } => {
                out.push_str("<a href=\"");
                out.push_str(&escape(target));
                out.push_str("\">");
                out.push_str(&render(inner));
                out.push_str("</a>");
            }
        }
    }
    out
}

/// Начертания элементами; порядок вложенности — как у маркеров Markdown:
/// полужирный снаружи, подчёркивание внутри.
fn mark(text: String, marks: Marks) -> String {
    let mut out = text;
    if marks.underline {
        out = format!("<u>{out}</u>");
    }
    if marks.strike {
        out = format!("<del>{out}</del>");
    }
    if marks.italic {
        out = format!("<em>{out}</em>");
    }
    if marks.bold {
        out = format!("<strong>{out}</strong>");
    }
    out
}

/// Экранирует текст и значения атрибутов: `&`, `<`, `>` и `"`.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Класс выравнивания: `w:jc` абзаца или `w:jc` таблицы.
///
/// Выравнивание по левому краю (и начало строки) — умолчание HTML, класс ему не нужен.
fn alignment_class(jc: Option<&Justification>) -> Option<&'static str> {
    match jc? {
        Justification::Center => Some("align-center"),
        Justification::Right | Justification::End => Some("align-right"),
        Justification::Both | Justification::Distribute => Some("align-justify"),
        Justification::Left | Justification::Start | Justification::Other(_) => None,
    }
}

/// Класс вертикального выравнивания содержимого ячейки (`w:vAlign`).
fn v_align_class(align: CellVAlign) -> Option<&'static str> {
    match align {
        CellVAlign::Top => None,
        CellVAlign::Center => Some("valign-middle"),
        CellVAlign::Bottom => Some("valign-bottom"),
    }
}

/// Атрибут `class`, если класс есть.
fn class_attribute(class: Option<&str>) -> String {
    class.map_or_else(String::new, |class| format!(" class=\"{class}\""))
}

/// Номер, с которого начинается нумерованный список: последняя группа цифр маркера.
///
/// Счётчик самого пункта подставляется в `w:lvlText` последним (`%1` в `5.`), поэтому
/// последняя цифра маркера — это и есть номер пункта: у списка с `w:startOverride`
/// он пятизначно сдвинут, а у продолженного после абзаца — не единица. Маркер без цифр
/// (буквы, римские числа) `start` не даёт: HTML-нумерация начнётся с единицы.
fn ordered_start(marker: &str) -> Option<u32> {
    let mut run = String::new();
    let mut last = String::new();
    for ch in marker.trim().chars() {
        if ch.is_ascii_digit() {
            run.push(ch);
        } else if !run.is_empty() {
            last = std::mem::take(&mut run);
        }
    }
    if !run.is_empty() {
        last = run;
    }
    last.parse().ok()
}

/// Ширина, высота и поля страницы: последняя секция документа задаёт их для всей страницы.
fn page_setup(document: &Document) -> PageSetup {
    let Some(section) = document.body.sections.last() else {
        return PageSetup::default();
    };
    let margins = &section.margins;
    PageSetup {
        width: section.page_size.width.value(),
        height: section.page_size.height.value(),
        margins: [
            margins.top.value(),
            margins.right.value(),
            margins.bottom.value(),
            margins.left.value(),
        ],
    }
}

/// Длина в twips как значение в пунктах.
fn push_twips(out: &mut String, twips: i32) {
    push_points(out, f64::from(twips) / 20.0);
}

/// Число в пунктах: без хвостовых нулей, чтобы `612.00` не попало в стили.
fn push_points(out: &mut String, points: f64) {
    out.push_str(&number(points));
    out.push_str("pt");
}

/// Число с точностью до сотых и без хвостовых нулей: `612.00` — это `612`, а
/// `33.333333` — `33.33`.
fn number(value: f64) -> String {
    let mut text = format!("{value:.2}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Имя шрифта как строка CSS: кавычка или обратный слэш закрыли бы её раньше времени.
fn css_string(value: &str) -> String {
    value.replace(['"', '\\'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Полный набор начертаний: зачёркивания (`w:strike`) нет ни в одной фикстуре
    /// корпуса, а элемент нужен. Порядок вложенности тот же, что у маркеров Markdown.
    #[test]
    fn marks_of_every_kind_nest_like_markdown() {
        let marks = Marks {
            bold: true,
            italic: true,
            underline: true,
            strike: true,
        };
        assert_eq!(
            mark("x".to_owned(), marks),
            "<strong><em><del><u>x</u></del></em></strong>"
        );
    }
}
