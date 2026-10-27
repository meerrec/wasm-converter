//! Стили ODT: текст, абзац, раскладка страницы, ширина столбца и заливка ячейки.
//!
//! ODT кладёт стили в два места: именованные — в `styles.xml` (`office:styles`,
//! `office:master-styles`), автоматические — в `content.xml`
//! (`office:automatic-styles`). Здесь собираются только свойства, которые нужны
//! модели `docx`: начертание и кегль прогона, выравнивание и разрывы абзаца,
//! размер/поля страницы и геометрия таблицы. Списки (`text:list-style`),
//! колонтитулы (`style:header`/`style:footer`), границы и отступы ячеек —
//! следующие срезы (TODO).

use std::collections::BTreeMap;

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_docx::{Color, Justification, Margins, Orientation, PageSize, Twips};
use quick_xml::events::{BytesStart, Event};

use crate::error::Result;
use crate::xml::{find, map_core};

use super::{
    bold_from_weight, color_rgb, italic_from_style, justification_from, length_pt, length_twips,
    underline_from_style,
};

/// Поле по умолчанию (1 дюйм) — то же умолчание, что у DOCX `default_section`.
const DEFAULT_MARGIN_TWIPS: Twips = Twips::new(1440);

/// Свойства знака после разрешения текстового стиля.
#[derive(Debug, Clone, Default)]
pub(crate) struct TextStyle {
    /// Кегль в пунктах; `None` — не задан (каскад возьмёт умолчание).
    pub size: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<Color>,
    pub font: Option<String>,
}

/// Стиль абзаца: выравнивание, разрыв, мастер-страница, отступы и знаки.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParagraphStyle {
    /// Знаки по умолчанию: `style:text-properties` внутри стиля абзаца.
    pub text: TextStyle,
    /// `fo:text-align`.
    pub align: Option<Justification>,
    /// `fo:break-before="page"`.
    pub break_before: bool,
    /// `style:master-page-name` — смена мастер-страницы (и секции) абзацем.
    pub master: Option<String>,
    /// `fo:margin-top`.
    pub margin_top: Option<Twips>,
    /// `fo:margin-bottom`.
    pub margin_bottom: Option<Twips>,
    /// `fo:margin-left`.
    pub margin_left: Option<Twips>,
    /// `fo:margin-right`.
    pub margin_right: Option<Twips>,
}

/// Раскладка страницы: размер, ориентация и поля.
#[derive(Debug, Clone)]
pub(crate) struct PageLayoutDef {
    pub page_size: PageSize,
    pub orientation: Orientation,
    pub margins: Margins,
}

/// Накопитель стилей ODT.
#[derive(Debug, Default)]
pub(crate) struct StyleBook {
    text: BTreeMap<String, TextStyle>,
    paragraph: BTreeMap<String, ParagraphStyle>,
    page_layouts: BTreeMap<String, PageLayoutDef>,
    /// Имя мастер-страницы → имя `style:page-layout`.
    master_layouts: BTreeMap<String, String>,
    /// Имена мастер-страниц в порядке файла (первая — мастер по умолчанию).
    master_order: Vec<String>,
    /// Стиль столбца → ширина в twips.
    column_widths: BTreeMap<String, Twips>,
    /// Стиль ячейки → цвет заливки (`fo:background-color`).
    cell_fills: BTreeMap<String, Color>,
}

impl StyleBook {
    /// Текстовый стиль по имени; неизвестный — пустой.
    #[must_use]
    pub(crate) fn text(&self, name: Option<&str>) -> TextStyle {
        name.and_then(|name| self.text.get(name).cloned())
            .unwrap_or_default()
    }

    /// Стиль абзаца по имени; неизвестный — пустой.
    #[must_use]
    pub(crate) fn paragraph(&self, name: Option<&str>) -> ParagraphStyle {
        name.and_then(|name| self.paragraph.get(name).cloned())
            .unwrap_or_default()
    }

    /// Раскладка страницы по имени мастер-страницы.
    #[must_use]
    pub(crate) fn page_layout_for_master(&self, master: &str) -> Option<&PageLayoutDef> {
        let layout = self.master_layouts.get(master)?;
        self.page_layouts.get(layout)
    }

    /// Имя мастер-страницы по умолчанию: первая в `office:master-styles`.
    #[must_use]
    pub(crate) fn default_master(&self) -> Option<&str> {
        self.master_order.first().map(String::as_str)
    }

    /// Ширина столбца по имени его стиля.
    #[must_use]
    pub(crate) fn column_width(&self, name: Option<&str>) -> Option<Twips> {
        self.column_widths.get(name?).copied()
    }

    /// Заливка ячейки по имени её стиля.
    #[must_use]
    pub(crate) fn cell_fill(&self, name: Option<&str>) -> Option<Color> {
        self.cell_fills.get(name?).copied()
    }
}

/// Собрать стили из одной части пакета.
///
/// Вызывается и для `styles.xml`, и для `content.xml`: в первом лежат
/// именованные стили, раскладки и мастер-страницы, во втором — автоматические
/// стили, на которые ссылаются абзацы и таблицы. Повторное объявление имени
/// перекрывает предыдущее, поэтому `content.xml` разбирается вторым.
///
/// # Errors
/// [`OdfError::Core`] — XML не разбирается; [`OdfError::Malformed`] — атрибут
/// элемента не читается.
pub(crate) fn collect(bytes: &[u8], part: &str, book: &mut StyleBook) -> Result<()> {
    let mut reader = XmlReader::preserving(bytes, part.to_owned());
    let mut parser = Parser {
        part,
        book,
        style: None,
        layout: None,
    };

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }
    Ok(())
}

/// Открытый `style:style`: имя и семейство.
#[derive(Debug, Clone)]
struct StyleDef {
    name: String,
    family: String,
}

/// Состояние одного прохода по части стилей.
struct Parser<'a> {
    part: &'a str,
    book: &'a mut StyleBook,
    style: Option<StyleDef>,
    /// Имя открытой `style:page-layout`.
    layout: Option<String>,
}

impl Parser<'_> {
    fn handle(&mut self, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.start(&element, true),
            // Пустой элемент значит то же, что открытый и сразу закрытый.
            Event::Empty(element) => self.start(&element, false),
            Event::End(element) => {
                self.end(element.local_name().as_ref());
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Открыть или обработать элемент.
    fn start(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let name = element.local_name();
        match name.as_ref() {
            b"style" => self.start_style(element, open),
            b"page-layout" => self.start_page_layout(element, open),
            b"master-page" => self.start_master_page(element),
            b"text-properties" => self.text_properties(element),
            b"paragraph-properties" => self.paragraph_properties(element),
            b"page-layout-properties" => self.page_layout_properties(element),
            b"table-column-properties" => self.table_column_properties(element),
            b"table-cell-properties" => self.table_cell_properties(element),
            _ => Ok(()),
        }
    }

    /// `style:style`: имя, семейство и ссылка на мастер-страницу у абзацев.
    fn start_style(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let Some(name) = find(&attrs, "name") else {
            return Ok(());
        };
        let family = find(&attrs, "family").unwrap_or("").to_owned();

        // `style:master-page-name` лежит на самом `style:style` (так его понимает
        // LibreOffice): абзац с этим стилем переключает мастер-страницу и секцию.
        if family == "paragraph" {
            if let Some(master) = find(&attrs, "master-page-name") {
                self.book
                    .paragraph
                    .entry(name.to_owned())
                    .or_default()
                    .master = Some(master.to_owned());
            }
        }

        self.style = Some(StyleDef {
            name: name.to_owned(),
            family,
        });
        if !open {
            self.style = None;
        }
        Ok(())
    }

    /// `style:page-layout`: имя, под которым его найдёт мастер-страница.
    fn start_page_layout(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        self.layout = find(&attrs, "name").map(str::to_owned);
        if !open {
            self.layout = None;
        }
        Ok(())
    }

    /// `style:master-page`: имя и ссылка на раскладку.
    fn start_master_page(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        if let (Some(name), Some(layout)) = (find(&attrs, "name"), find(&attrs, "page-layout-name"))
        {
            // Порядок файла решает, какая мастер-страница — по умолчанию.
            self.book.master_order.push(name.to_owned());
            self.book
                .master_layouts
                .insert(name.to_owned(), layout.to_owned());
        }
        Ok(())
    }

    /// `style:text-properties` — свойства знака в текстовом стиле или стиле абзаца.
    fn text_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(style) = self.style.clone() else {
            return Ok(());
        };
        let attrs = attrs(element, self.part)?;
        let text = TextStyle {
            size: find(&attrs, "font-size").and_then(length_pt),
            bold: find(&attrs, "font-weight").is_some_and(bold_from_weight),
            italic: find(&attrs, "font-style").is_some_and(italic_from_style),
            underline: find(&attrs, "text-underline-style").is_some_and(underline_from_style),
            color: find(&attrs, "color").and_then(color_rgb),
            font: find(&attrs, "font-name").map(str::to_owned),
        };
        match style.family.as_str() {
            "text" => {
                self.book.text.insert(style.name, text);
            }
            "paragraph" => {
                self.book.paragraph.entry(style.name).or_default().text = text;
            }
            _ => {}
        }
        Ok(())
    }

    /// `style:paragraph-properties` — выравнивание, разрыв и отступы абзаца.
    fn paragraph_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(style) = self.style.clone() else {
            return Ok(());
        };
        if style.family != "paragraph" {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        let entry = self.book.paragraph.entry(style.name).or_default();
        entry.align = find(&attrs, "text-align").map(justification_from);
        entry.break_before = find(&attrs, "break-before").is_some_and(|v| v == "page");
        entry.margin_top = find(&attrs, "margin-top").and_then(length_twips);
        entry.margin_bottom = find(&attrs, "margin-bottom").and_then(length_twips);
        entry.margin_left = find(&attrs, "margin-left").and_then(length_twips);
        entry.margin_right = find(&attrs, "margin-right").and_then(length_twips);
        Ok(())
    }

    /// `style:page-layout-properties` — `fo:page-width/height`, ориентация и поля.
    fn page_layout_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(name) = self.layout.clone() else {
            return Ok(());
        };
        let attrs = attrs(element, self.part)?;
        let (Some(width), Some(height)) = (
            find(&attrs, "page-width").and_then(length_twips),
            find(&attrs, "page-height").and_then(length_twips),
        ) else {
            return Ok(());
        };
        let orientation = match find(&attrs, "print-orientation") {
            Some("landscape") => Orientation::Landscape,
            _ => Orientation::Portrait,
        };
        let margins = Margins {
            top: find(&attrs, "margin-top")
                .and_then(length_twips)
                .unwrap_or(DEFAULT_MARGIN_TWIPS),
            right: find(&attrs, "margin-right")
                .and_then(length_twips)
                .unwrap_or(DEFAULT_MARGIN_TWIPS),
            bottom: find(&attrs, "margin-bottom")
                .and_then(length_twips)
                .unwrap_or(DEFAULT_MARGIN_TWIPS),
            left: find(&attrs, "margin-left")
                .and_then(length_twips)
                .unwrap_or(DEFAULT_MARGIN_TWIPS),
            header: None,
            footer: None,
            gutter: None,
        };
        self.book.page_layouts.insert(
            name,
            PageLayoutDef {
                page_size: PageSize { width, height },
                orientation,
                margins,
            },
        );
        Ok(())
    }

    /// `style:table-column-properties` — ширина столбца таблицы.
    fn table_column_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(style) = self.style.clone() else {
            return Ok(());
        };
        if style.family != "table-column" {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        if let Some(width) = find(&attrs, "column-width").and_then(length_twips) {
            self.book.column_widths.insert(style.name, width);
        }
        Ok(())
    }

    /// `style:table-cell-properties` — заливка ячейки.
    fn table_cell_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(style) = self.style.clone() else {
            return Ok(());
        };
        if style.family != "table-cell" {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        if let Some(fill) = find(&attrs, "background-color").and_then(color_rgb) {
            self.book.cell_fills.insert(style.name, fill);
        }
        Ok(())
    }

    /// Закрыть элемент: снять открытый стиль или раскладку.
    fn end(&mut self, name: &[u8]) {
        match name {
            b"style" => self.style = None,
            b"page-layout" => self.layout = None,
            _ => {}
        }
    }
}

/// Атрибуты элемента с именем части в ошибке.
fn attrs<'a>(element: &'a BytesStart<'a>, part: &str) -> Result<Vec<Attr<'a>>> {
    let attrs =
        doc_converter_core::xml::attributes(element).map_err(|error| map_core(part, error))?;
    Ok(attrs.into_vec())
}
