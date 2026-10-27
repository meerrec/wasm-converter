//! Стили ODP: графика, текст, абзацы и раскладка страницы.
//!
//! ODP кладёт стили в два места: именованные — в `styles.xml`
//! (`office:styles`, `office:master-styles`), автоматические — в `content.xml`
//! (`office:automatic-styles`). Здесь собираются только свойства, которые
//! нужны модели `pptx`: заливка/контур шейпа, кегль/цвет/начертание прогона,
//! выравнивание абзаца и размер страницы. Списки (`text:list-style`), анимации
//! и эффекты — следующие срезы (TODO).

use std::collections::BTreeMap;

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_pptx::model::{Align, Emu, Rgb, RunProps};
use quick_xml::events::{BytesStart, Event};

use crate::error::Result;
use crate::xml::{find, map_core};

use super::{
    align_from, bold_from_weight, fill_color, italic_from_style, length_emu, length_pt,
    stroke_color, underline_from_style,
};

/// Свойства знака после разрешения текстового стиля.
#[derive(Debug, Clone, Default)]
pub(crate) struct TextStyle {
    /// Кегль в пунктах; `None` — не задан (каскад возьмёт из абзаца/умолчания).
    pub size: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<Rgb>,
    pub font: Option<String>,
}

impl TextStyle {
    /// Свойства знака как [`RunProps`] — до наложения на каскад абзаца.
    #[must_use]
    pub(crate) fn to_run_props(&self) -> RunProps {
        RunProps {
            size: self.size,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            color: self.color,
            font: self.font.clone(),
        }
    }
}

/// Стиль абзаца: выравнивание и знаки по умолчанию.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParagraphStyle {
    pub align: Align,
    /// Знаки по умолчанию: `style:text-properties` внутри стиля абзаца.
    pub text: TextStyle,
}

/// Стиль графики: заливка и контур шейпа.
#[derive(Debug, Clone, Default)]
pub(crate) struct GraphicStyle {
    /// Заливка; `None` — без заливки (`draw:fill="none"` или не задана).
    pub fill: Option<Rgb>,
    /// Цвет контура; `None` — без контура.
    pub stroke: Option<Rgb>,
    /// Толщина контура в EMU.
    pub stroke_width: Option<Emu>,
}

/// Накопитель стилей ODP.
#[derive(Debug, Default)]
pub(crate) struct StyleBook {
    text: BTreeMap<String, TextStyle>,
    paragraph: BTreeMap<String, ParagraphStyle>,
    graphic: BTreeMap<String, GraphicStyle>,
    /// Имя `style:page-layout` → (ширина, высота) в EMU.
    page_layouts: BTreeMap<String, (Emu, Emu)>,
    /// Имя `style:master-page` → имя `style:page-layout`.
    master_pages: BTreeMap<String, String>,
}

impl StyleBook {
    /// Текстовый стиль по имени; неизвестный — пустой (каскад возьмёт абзац).
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

    /// Стиль графики по имени; неизвестный — пустой.
    #[must_use]
    pub(crate) fn graphic(&self, name: Option<&str>) -> GraphicStyle {
        name.and_then(|name| self.graphic.get(name).cloned())
            .unwrap_or_default()
    }

    /// Размер страницы по имени мастер-страницы: мастер → раскладка → размер.
    #[must_use]
    pub(crate) fn page_size(&self, master: Option<&str>) -> Option<(Emu, Emu)> {
        let layout = self.master_pages.get(master?)?;
        self.page_layouts.get(layout).copied()
    }
}

/// Собрать стили из одной части пакета.
///
/// Вызывается и для `styles.xml`, и для `content.xml`: в первом лежат
/// именованные стили, раскладки и мастер-страницы, во втором — автоматические
/// стили, на которые ссылаются шейпы и абзацы. Повторное объявление имени
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
        master: None,
    };

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }
    Ok(())
}

/// Открытый `style:style`: имя и семейство.
struct StyleDef {
    name: String,
    family: String,
}

/// Состояние одного прохода по части стилей.
struct Parser<'a> {
    part: &'a str,
    book: &'a mut StyleBook,
    style: Option<StyleDef>,
    layout: Option<String>,
    master: Option<String>,
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
            b"master-page" => self.start_master_page(element, open),
            b"text-properties" => self.text_properties(element),
            b"paragraph-properties" => self.paragraph_properties(element),
            b"graphic-properties" => self.graphic_properties(element),
            b"page-layout-properties" => self.page_layout_properties(element),
            _ => Ok(()),
        }
    }

    /// `style:style`: запомнить имя и семейство для вложенных свойств.
    fn start_style(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let Some(name) = find(&attrs, "name") else {
            return Ok(());
        };
        let family = find(&attrs, "family").unwrap_or("").to_owned();
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
    fn start_master_page(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        if let (Some(name), Some(layout)) = (find(&attrs, "name"), find(&attrs, "page-layout-name"))
        {
            self.book
                .master_pages
                .insert(name.to_owned(), layout.to_owned());
        }
        self.master = find(&attrs, "name").map(str::to_owned);
        if !open {
            self.master = None;
        }
        Ok(())
    }

    /// `style:text-properties` — свойства знака в текстовом стиле или стиле абзаца.
    fn text_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some((name, family)) = self
            .style
            .as_ref()
            .map(|style| (style.name.clone(), style.family.clone()))
        else {
            return Ok(());
        };
        let attrs = attrs(element, self.part)?;
        let text = TextStyle {
            size: find(&attrs, "font-size").and_then(length_pt),
            bold: find(&attrs, "font-weight").is_some_and(bold_from_weight),
            italic: find(&attrs, "font-style").is_some_and(italic_from_style),
            underline: find(&attrs, "text-underline-style").is_some_and(underline_from_style),
            color: find(&attrs, "color").and_then(Rgb::from_hex),
            font: find(&attrs, "font-name").map(str::to_owned),
        };
        match family.as_str() {
            "text" => {
                self.book.text.insert(name, text);
            }
            "paragraph" => {
                self.book.paragraph.entry(name).or_default().text = text;
            }
            _ => {}
        }
        Ok(())
    }

    /// `style:paragraph-properties` — выравнивание абзаца.
    fn paragraph_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some((name, family)) = self
            .style
            .as_ref()
            .map(|style| (style.name.clone(), style.family.clone()))
        else {
            return Ok(());
        };
        if family != "paragraph" {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        let align = find(&attrs, "text-align").map_or(Align::Left, align_from);
        self.book.paragraph.entry(name).or_default().align = align;
        Ok(())
    }

    /// `style:graphic-properties` — заливка и контур шейпа.
    fn graphic_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some((name, family)) = self
            .style
            .as_ref()
            .map(|style| (style.name.clone(), style.family.clone()))
        else {
            return Ok(());
        };
        if family != "graphic" {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        let fill = fill_color(&attrs);
        let stroke = stroke_color(&attrs);
        let stroke_width = find(&attrs, "stroke-width").and_then(length_emu);
        self.book.graphic.insert(
            name,
            GraphicStyle {
                fill,
                stroke,
                stroke_width,
            },
        );
        Ok(())
    }

    /// `style:page-layout-properties` — `fo:page-width/height` раскладки.
    fn page_layout_properties(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let Some(name) = self.layout.clone() else {
            return Ok(());
        };
        let attrs = attrs(element, self.part)?;
        let width = find(&attrs, "page-width").and_then(length_emu);
        let height = find(&attrs, "page-height").and_then(length_emu);
        if let (Some(width), Some(height)) = (width, height) {
            self.book.page_layouts.insert(name, (width, height));
        }
        Ok(())
    }

    /// Закрыть элемент: снять открытый стиль/раскладку/мастер-страницу.
    fn end(&mut self, name: &[u8]) {
        match name {
            b"style" => self.style = None,
            b"page-layout" => self.layout = None,
            b"master-page" => self.master = None,
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
