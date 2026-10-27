//! `content.xml`: тело презентации в модель `pptx`.
//!
//! Разбор потоковый и однопроходный: `draw:page` открывает слайд, шейп —
//! `draw:frame`/`draw:text-box`/`draw:rect`/`draw:ellipse`/`draw:custom-shape`,
//! текст внутри текстовой рамки — `text:p`/`text:span`. Пробелы в тексте
//! значимы (`text:s`, `xml:space`), поэтому читатель не обрезает текстовые
//! узлы.
//!
//! Политика ошибок — как у PPTX (ADR-0016): фатально только то, без чего
//! модель недостоверна (нет `content.xml`, битый XML); всё восстановимое —
//! битый шейп, ненайденная картинка, незнакомый атрибут — копится в
//! [`Presentation::warnings`] и не роняет пакет.
//!
//! # Текст
//!
//! Каскад знака упрощён: свойства `text:span` накладываются на знаки по
//! умолчанию из стиля абзаца (`style:text-properties` внутри
//! `style:family="paragraph"`). Это покрывает `text:style-name` на обоих
//! уровнях; наследование от мастер-страницы и общие стили текста — следующий
//! срез (TODO).

use std::collections::BTreeMap;

use doc_converter_core::xml::{Attr, XmlReader};
use doc_converter_core::Archive;
use doc_converter_pptx::model::{
    Frame, GeometryPreset, ImagePart, Line, Paragraph, Presentation, Rgb, Run, RunProps, Shape,
    ShapeKind, Slide, SlideSize, TextBody, TextBox, Theme,
};
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};

use crate::error::{OdfError, Result};
use crate::manifest::Manifest;
use crate::xml::{find, map_core, resolve_reference};

use super::styles::StyleBook;
use super::{
    bold_from_weight, fill_color, italic_from_style, length_emu, length_pt, stroke_color,
    underline_from_style,
};

/// Разобрать `content.xml` в презентацию.
///
/// Картинки читаются из архива здесь: `draw:image` ссылается на часть пакета
/// (`Pictures/…`), и байты должны попасть в [`ImagePart`] модели.
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
) -> Result<Presentation> {
    let mut reader = XmlReader::preserving(bytes, part.to_owned());
    let mut parser = Parser {
        part,
        styles,
        manifest,
        archive,
        slides: Vec::new(),
        images: Vec::new(),
        image_ids: BTreeMap::new(),
        warnings: Vec::new(),
        size: None,
        page: None,
        shape: None,
        paragraph: None,
        run: None,
        paragraph_default: RunProps::default(),
        list_depth: 0,
        shape_counter: 0,
    };

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }
    Ok(parser.finish())
}

/// Слайд, который ещё собирается из `draw:page`.
struct PageState {
    name: String,
    size: SlideSize,
    shapes: Vec<Shape>,
}

/// Источник шейпа: от него зависит, что делает `draw:text-box`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShapeElement {
    Frame,
    TextBox,
    Rect,
    Ellipse,
    CustomShape,
}

/// Содержимое шейпа, ещё не сведённое в [`ShapeKind`].
enum KindState {
    /// Дети не встретились (пустой `draw:frame` → прямоугольник).
    Unknown,
    Text,
    Picture {
        image_id: Option<u32>,
    },
    Geometry(GeometryPreset),
}

/// Шейп, который ещё собирается.
struct ShapeState {
    name: String,
    element: ShapeElement,
    frame: Frame,
    fill: Option<Rgb>,
    line: Option<Line>,
    kind: KindState,
    paragraphs: Vec<Paragraph>,
}

/// Состояние разбора `content.xml`.
struct Parser<'a> {
    part: &'a str,
    styles: &'a StyleBook,
    manifest: &'a Manifest,
    archive: &'a mut Archive,
    slides: Vec<Slide>,
    images: Vec<ImagePart>,
    /// `xlink:href` картинки → её id в [`Self::images`] (дедупликация).
    image_ids: BTreeMap<String, u32>,
    warnings: Vec<String>,
    /// Размер презентации — из первой страницы.
    size: Option<SlideSize>,
    page: Option<PageState>,
    shape: Option<ShapeState>,
    paragraph: Option<Paragraph>,
    run: Option<Run>,
    /// Знаки по умолчанию текущего абзаца (из его стиля).
    paragraph_default: RunProps,
    list_depth: u32,
    shape_counter: usize,
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
            b"page" => {
                self.open_page(element)?;
                if !open {
                    self.close_page();
                }
                Ok(())
            }
            b"frame" => {
                self.open_shape(element, ShapeElement::Frame)?;
                if !open {
                    self.close_shape();
                }
                Ok(())
            }
            b"text-box" => {
                self.open_text_box(element)?;
                // Самостоятельный `draw:text-box` закрывается здесь же; внутри
                // `draw:frame` он лишь пометил frame текстовым — закрывать рано.
                if !open
                    && self
                        .shape
                        .as_ref()
                        .is_some_and(|s| s.element == ShapeElement::TextBox)
                {
                    self.close_shape();
                }
                Ok(())
            }
            b"image" => self.open_image(element),
            b"rect" => {
                self.open_shape(element, ShapeElement::Rect)?;
                if !open {
                    self.close_shape();
                }
                Ok(())
            }
            b"ellipse" => {
                self.open_shape(element, ShapeElement::Ellipse)?;
                if !open {
                    self.close_shape();
                }
                Ok(())
            }
            b"custom-shape" => {
                self.open_shape(element, ShapeElement::CustomShape)?;
                if !open {
                    self.close_shape();
                }
                Ok(())
            }
            b"p" => self.open_paragraph(element, open),
            b"span" => self.open_span(element, open),
            b"list" => {
                self.list_depth = self.list_depth.saturating_add(1);
                Ok(())
            }
            b"s" => self.space(element),
            b"tab" => {
                self.push_text("\t");
                Ok(())
            }
            b"line-break" => {
                self.push_text("\n");
                Ok(())
            }
            // `text:list-item` — просто обёртка вокруг `text:p`; `text:a`
            // (гиперссылка) игнорируется как элемент, но её текст внутри
            // пройдёт обычным прогоном (`xlink:href` не переносится, TODO).
            // Прочие неизвестные элементы тоже пропускаются.
            _ => Ok(()),
        }
    }

    /// Закрыть элемент.
    fn end(&mut self, name: &[u8]) {
        match name {
            b"page" => self.close_page(),
            b"frame" | b"rect" | b"ellipse" | b"custom-shape" => self.close_shape(),
            b"text-box" => {
                if self
                    .shape
                    .as_ref()
                    .is_some_and(|s| s.element == ShapeElement::TextBox)
                {
                    self.close_shape();
                }
            }
            b"p" => self.close_paragraph(),
            b"span" => {
                self.flush_run();
                self.reset_run();
            }
            b"list" => self.list_depth = self.list_depth.saturating_sub(1),
            _ => {}
        }
    }

    /// `draw:page`: имя, размер и новый слайд.
    fn open_page(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let name = find(&attrs, "name")
            .map_or_else(|| format!("Slide {}", self.slides.len() + 1), str::to_owned);
        let master = find(&attrs, "master-page-name").map(str::to_owned);
        let size = resolve_page_size(&attrs, master.as_deref(), self.styles, &mut self.warnings);
        self.page = Some(PageState {
            name,
            size,
            shapes: Vec::new(),
        });
        Ok(())
    }

    /// Открыть шейп: имя, рамка, заливка/контур и начальное содержимое.
    fn open_shape(&mut self, element: &BytesStart<'_>, el: ShapeElement) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let name = shape_name(&attrs, self.shape_counter);
        self.shape_counter += 1;
        let frame = frame_from(&attrs);
        let (fill, line) = resolve_graphic(&attrs, self.styles);
        let kind = match el {
            // `draw:enhanced-geometry` не читается: custom-shape сводится к
            // прямоугольнику (TODO).
            ShapeElement::Rect | ShapeElement::CustomShape => {
                KindState::Geometry(GeometryPreset::Rect)
            }
            ShapeElement::Ellipse => KindState::Geometry(GeometryPreset::Ellipse),
            ShapeElement::TextBox => KindState::Text,
            ShapeElement::Frame => KindState::Unknown,
        };
        self.shape = Some(ShapeState {
            name,
            element: el,
            frame,
            fill,
            line,
            kind,
            paragraphs: Vec::new(),
        });
        Ok(())
    }

    /// `draw:text-box`: пометить текущий frame текстовым или открыть свою форму.
    fn open_text_box(&mut self, element: &BytesStart<'_>) -> Result<()> {
        if self
            .shape
            .as_ref()
            .is_some_and(|s| s.element == ShapeElement::Frame)
        {
            if let Some(shape) = self.shape.as_mut() {
                shape.kind = KindState::Text;
            }
            return Ok(());
        }
        self.open_shape(element, ShapeElement::TextBox)
    }

    /// `draw:image`: прочитать картинку из архива и пометить шейп как picture.
    fn open_image(&mut self, element: &BytesStart<'_>) -> Result<()> {
        let attrs = attrs(element, self.part)?;
        let href = find(&attrs, "href").map(str::to_owned);
        let image_id = if let Some(href) = href.as_deref() {
            self.image_id(href)
        } else {
            self.warn("`draw:image` has no `xlink:href`");
            None
        };
        match self.shape.as_mut() {
            Some(shape) => shape.kind = KindState::Picture { image_id },
            None => self.warn("`draw:image` outside a shape"),
        }
        Ok(())
    }

    /// Картинка по `xlink:href`: читается один раз и получает стабильный id.
    #[allow(clippy::cast_possible_truncation)]
    fn image_id(&mut self, href: &str) -> Option<u32> {
        if let Some(&id) = self.image_ids.get(href) {
            return Some(id);
        }
        let bytes = match self.archive.part(href) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.warn(format!("image `{href}` is not readable: {error}"));
                return None;
            }
        };
        let id = self.images.len() as u32;
        let mime = self
            .manifest
            .entry(href)
            .and_then(|entry| entry.media_type.as_deref())
            .map_or_else(|| mime_from_name(href).to_owned(), str::to_owned);
        self.images.push(ImagePart {
            id,
            name: href.to_owned(),
            mime,
            bytes,
        });
        self.image_ids.insert(href.to_owned(), id);
        Some(id)
    }

    /// `text:p`: новый абзац со знаками по умолчанию из его стиля.
    fn open_paragraph(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.shape.is_none() {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        let style = self.styles.paragraph(find(&attrs, "style-name"));
        self.paragraph_default = style.text.to_run_props();
        // Первый уровень списка — без отступа; вложенный — со сдвигом.
        let level = u8::try_from(self.list_depth.saturating_sub(1)).unwrap_or(8);
        self.paragraph = Some(Paragraph {
            level,
            align: style.align,
            // Маркеры списков — следующий срез (TODO): текст идёт плоским.
            bullet: None,
            runs: Vec::new(),
        });
        self.reset_run();
        if !open {
            self.close_paragraph();
        }
        Ok(())
    }

    /// `text:span`: свойства прогона поверх каскада абзаца.
    fn open_span(&mut self, element: &BytesStart<'_>, open: bool) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        self.flush_run();
        let attrs = attrs(element, self.part)?;
        let props = span_props(&attrs, self.styles);
        let resolved = self.paragraph_default.merged(Some(&props));
        self.run = Some(Run {
            text: String::new(),
            props: resolved,
        });
        if !open {
            self.flush_run();
            self.reset_run();
        }
        Ok(())
    }

    /// `text:s`: значимые пробелы с повтором `text:c`.
    fn space(&mut self, element: &BytesStart<'_>) -> Result<()> {
        if self.paragraph.is_none() {
            return Ok(());
        }
        let attrs = attrs(element, self.part)?;
        // Недоверенный файл может просить миллионы пробелов — режем до разумного.
        let count = find(&attrs, "c")
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .min(4096);
        self.push_text(&" ".repeat(count as usize));
        Ok(())
    }

    /// Закрыть слайд и положить его в презентацию.
    fn close_page(&mut self) {
        // Незакрытый шейп внутри страницы — битый файл; закрываем и не теряем.
        if self.shape.is_some() {
            self.close_shape();
        }
        let Some(page) = self.page.take() else {
            return;
        };
        // Размер у презентации общий; его задаёт первая страница.
        if self.size.is_none() {
            self.size = Some(page.size);
        }
        self.slides.push(Slide {
            name: page.name,
            background: None,
            shapes: page.shapes,
        });
    }

    /// Закрыть шейп: свести его содержимое в [`Shape`] и положить в слайд.
    fn close_shape(&mut self) {
        self.flush_run();
        if let Some(paragraph) = self.paragraph.take() {
            if let Some(shape) = self.shape.as_mut() {
                shape.paragraphs.push(paragraph);
            }
        }
        self.run = None;
        self.paragraph_default = RunProps::default();

        let Some(shape) = self.shape.take() else {
            return;
        };
        let Some(page) = self.page.as_mut() else {
            self.warn("shape outside a page");
            return;
        };

        let kind = match shape.kind {
            KindState::Text => ShapeKind::Text(TextBox {
                body: TextBody {
                    paragraphs: shape.paragraphs,
                    ..TextBody::default()
                },
            }),
            KindState::Picture { image_id } => ShapeKind::Picture { image_id },
            KindState::Geometry(preset) => ShapeKind::Geometry(preset),
            // Пустой `draw:frame` — это просто прямоугольник с заливкой/контуром.
            KindState::Unknown => ShapeKind::Geometry(GeometryPreset::Rect),
        };
        page.shapes.push(Shape {
            name: shape.name,
            frame: shape.frame,
            fill: shape.fill,
            line: shape.line,
            kind,
        });
    }

    /// Закрыть абзац: отдать накопленные прогоны в текстовую рамку.
    fn close_paragraph(&mut self) {
        self.flush_run();
        if let Some(paragraph) = self.paragraph.take() {
            if let Some(shape) = self.shape.as_mut() {
                shape.paragraphs.push(paragraph);
            }
        }
        self.run = None;
        self.paragraph_default = RunProps::default();
    }

    /// Конец части: незакрытый шейп/страница закрываются, прочитанное не теряем.
    fn finish(mut self) -> Presentation {
        if self.shape.is_some() {
            self.close_shape();
        }
        if self.page.is_some() {
            self.close_page();
        }
        Presentation {
            size: self.size.unwrap_or_default(),
            slides: self.slides,
            images: self.images,
            theme: Theme::default(),
            warnings: self.warnings,
        }
    }

    /// Текст текстового узла — только внутри абзаца текстовой рамки.
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

    /// Дописать текст в текущий прогон.
    fn push_text(&mut self, text: &str) {
        if let Some(run) = self.run.as_mut() {
            run.text.push_str(text);
        }
    }

    /// Отдать текущий прогон в абзац, если в нём есть текст.
    fn flush_run(&mut self) {
        if let (Some(paragraph), Some(run)) = (self.paragraph.as_mut(), self.run.take()) {
            if !run.text.is_empty() {
                paragraph.runs.push(run);
            }
        }
    }

    /// Начать новый прогон со знаками по умолчанию абзаца.
    fn reset_run(&mut self) {
        self.run = Some(Run {
            text: String::new(),
            props: self.paragraph_default.clone(),
        });
    }

    /// Предупреждение: копится до предела, не роняя документ.
    fn warn(&mut self, message: impl Into<String>) {
        const MAX_WARNINGS: usize = 256;
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message.into());
        }
    }
}

/// Размер страницы: прямые `svg:width/height` страницы либо раскладка мастер-страницы.
fn resolve_page_size(
    attrs: &[Attr<'_>],
    master: Option<&str>,
    styles: &StyleBook,
    warnings: &mut Vec<String>,
) -> SlideSize {
    if let (Some(width), Some(height)) = (
        find(attrs, "width").and_then(length_emu),
        find(attrs, "height").and_then(length_emu),
    ) {
        return SlideSize { width, height };
    }
    if let Some((width, height)) = styles.page_size(master) {
        return SlideSize { width, height };
    }
    warnings.push("slide size is not declared; using the 16:9 default".to_owned());
    SlideSize::default()
}

/// Имя шейпа: `draw:name` → `draw:id` → сгенерированное «Shape N».
fn shape_name(attrs: &[Attr<'_>], counter: usize) -> String {
    find(attrs, "name")
        .or_else(|| find(attrs, "id"))
        .map_or_else(|| format!("Shape {}", counter + 1), str::to_owned)
}

/// Рамка шейпа из `svg:x/y/width/height` (EMU); отсутствующие — 0.
fn frame_from(attrs: &[Attr<'_>]) -> Frame {
    Frame {
        x: find(attrs, "x").and_then(length_emu).unwrap_or(0),
        y: find(attrs, "y").and_then(length_emu).unwrap_or(0),
        w: find(attrs, "width").and_then(length_emu).unwrap_or(0),
        h: find(attrs, "height").and_then(length_emu).unwrap_or(0),
    }
}

/// Заливка и контур шейпа: стиль `draw:style-name` + прямые атрибуты поверх.
fn resolve_graphic(attrs: &[Attr<'_>], styles: &StyleBook) -> (Option<Rgb>, Option<Line>) {
    let base = styles.graphic(find(attrs, "style-name"));
    let mut fill = base.fill;
    let mut stroke = base.stroke;
    let mut width = base.stroke_width;

    // Прямые атрибуты элемента перекрывают значения стиля.
    if find(attrs, "fill").is_some() || find(attrs, "fill-color").is_some() {
        fill = fill_color(attrs);
    }
    if find(attrs, "stroke").is_some() || find(attrs, "stroke-color").is_some() {
        stroke = stroke_color(attrs);
    }
    if let Some(raw) = find(attrs, "stroke-width") {
        width = length_emu(raw);
    }

    let line = (stroke.is_some() || width.is_some()).then_some(Line {
        color: stroke,
        width,
    });
    (fill, line)
}

/// Свойства прогона `text:span`: текстовый стиль + прямые атрибуты.
fn span_props(attrs: &[Attr<'_>], styles: &StyleBook) -> RunProps {
    let mut props = styles.text(find(attrs, "style-name")).to_run_props();
    if let Some(size) = find(attrs, "font-size").and_then(length_pt) {
        props.size = Some(size);
    }
    if let Some(weight) = find(attrs, "font-weight") {
        props.bold = bold_from_weight(weight);
    }
    if let Some(style) = find(attrs, "font-style") {
        props.italic = italic_from_style(style);
    }
    if let Some(underline) = find(attrs, "text-underline-style") {
        props.underline = underline_from_style(underline);
    }
    if let Some(color) = find(attrs, "color").and_then(Rgb::from_hex) {
        props.color = Some(color);
    }
    if let Some(font) = find(attrs, "font-name") {
        props.font = Some(font.to_owned());
    }
    props
}

/// MIME-тип по расширению части — запас, когда каталог не назвал media-type.
fn mime_from_name(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Атрибуты элемента; `Vec` вместо `SmallVec` — атрибутов у шейпа единицы.
fn attrs<'a>(element: &'a BytesStart<'a>, part: &str) -> Result<Vec<Attr<'a>>> {
    let attrs =
        doc_converter_core::xml::attributes(element).map_err(|error| map_core(part, error))?;
    Ok(attrs.into_vec())
}
