//! Чертежи листа: изображения и их якоря (`xl/drawings/drawing*.xml`).
//!
//! Картинка в OOXML — не свойство ячейки, а отдельная часть пакета: лист
//! ссылается на чертёж элементом `<drawing r:id="…"/>`, а чертёж хранит якоря,
//! привязывающие картинки к сетке. Якорей два вида: one-cell задаёт угол
//! ячейкой и размер явно, two-cell растягивает картинку между двумя ячейками.
//!
//! Разбираются только картинки (`xdr:pic`): фигуры, диаграммы и составные
//! объекты в модель листа не идут. Повреждённый якорь роняет только себя —
//! картинка не стоит того, чтобы из-за неё книга не открывалась.

use doc_converter_core::rels::RelMap;
use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use crate::error::Result;
use crate::layout::emu_to_px;
use crate::xml::{attributes, find};

/// `editAs`: как картинка ведёт себя при изменении ячеек.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAs {
    /// Двигается и тянется вместе с ячейками — обычное поведение Excel.
    TwoCell,
    /// Двигается с ячейкой, но размер не меняет.
    OneCell,
    /// Не двигается и не меняет размер: привязана к месту на листе.
    Absolute,
}

impl EditAs {
    /// Разобрать значение атрибута; незнакомое значение — как отсутствие.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "twoCell" => Some(Self::TwoCell),
            "oneCell" => Some(Self::OneCell),
            "absolute" => Some(Self::Absolute),
            _ => None,
        }
    }
}

/// Угол якоря: ячейка и смещение внутри неё.
///
/// Отсчёт от нуля, как в модели листа; смещения — в пикселях раскладки (96 dpi).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageMarker {
    /// Столбец.
    pub col: u32,
    /// Строка.
    pub row: u32,
    /// Смещение от левого края ячейки.
    pub col_off: f32,
    /// Смещение от верхнего края ячейки.
    pub row_off: f32,
}

/// Размер картинки в пикселях раскладки.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageExtent {
    /// Ширина.
    pub cx: f32,
    /// Высота.
    pub cy: f32,
}

/// Якорь изображения.
///
/// Координаты — в пикселях раскладки, без учёта зума: масштаб — дело того, кто
/// рисует.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageAnchor {
    /// Позиция задана одной ячейкой, размер — явно: картинка может вылезать за
    /// границы якорной ячейки.
    OneCell {
        /// Верхний левый угол.
        from: ImageMarker,
        /// Размер картинки.
        ext: ImageExtent,
    },
    /// Картинка растянута между углами двух ячеек.
    TwoCell {
        /// Верхний левый угол.
        from: ImageMarker,
        /// Нижний правый угол.
        to: ImageMarker,
    },
}

/// Изображение листа: ссылка на media-часть и якорь.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetImage {
    /// Имя из `cNvPr` (`Picture 1`): служит только для диагностики.
    pub name: Option<String>,
    /// Часть пакета с байтами картинки (`xl/media/image1.png`). `None` — связь
    /// не разрешилась или части нет в пакете.
    pub media: Option<String>,
    /// Поведение при изменении ячеек; `None` — атрибута в файле нет.
    pub edit_as: Option<EditAs>,
    /// Где картинка лежит на листе.
    pub anchor: ImageAnchor,
}

/// Разобрать чертёж в изображения листа.
///
/// Изображения возвращаются в порядке документа — это порядок наложения:
/// нарисованное раньше лежит ниже.
///
/// # Errors
///
/// [`crate::XlsxError::Core`] — XML не разбирается.
pub fn parse(
    bytes: &[u8],
    part: impl Into<String>,
    rels: Option<&RelMap>,
) -> Result<Vec<SheetImage>> {
    let part = part.into();
    let mut parser = DrawingParser::new(part.clone(), rels);
    let mut reader = XmlReader::new(bytes, part);

    while let Some(event) = reader.next_significant()? {
        parser.handle(event);
    }

    Ok(parser.images)
}

/// Значение атрибута по локальному имени; битые атрибуты — как отсутствие.
fn attr(element: &BytesStart<'_>, part: &str, name: &str) -> Option<String> {
    let attrs = attributes(element, part).ok()?;
    find(&attrs, name).map(str::to_owned)
}

/// Координата, которая читается из текста элемента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink {
    /// Мы не внутри элемента с числом.
    Idle,
    /// `col` — номер столбца.
    Col,
    /// `row` — номер строки.
    Row,
    /// `colOff` — смещение по горизонтали, в EMU.
    ColOff,
    /// `rowOff` — смещение по вертикали, в EMU.
    RowOff,
}

/// Сторона маркера, которая читается сейчас.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerSide {
    /// `<xdr:from>` — верхний левый угол.
    From,
    /// `<xdr:to>` — нижний правый угол.
    To,
}

/// Вид якоря.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnchorKind {
    /// Угол ячейкой и размер явно.
    OneCell,
    /// Диапазон между двумя ячейками.
    TwoCell,
}

/// Где мы относительно `<xdr:pic>`.
///
/// Внутри картинки `<a:ext>` — преобразование фигуры, а не размер якоря;
/// а без картинки якорь может принадлежать фигуре или диаграмме.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PicState {
    /// Картинки ещё не было.
    Absent,
    /// Читается сейчас.
    Inside,
    /// Уже закончилась.
    Done,
}

/// Разобранное значение координаты маркера.
#[derive(Debug, Clone, Copy)]
enum MarkerValue {
    /// Номер столбца.
    Col(u32),
    /// Номер строки.
    Row(u32),
    /// Горизонтальное смещение, уже в пикселях.
    ColOff(f32),
    /// Вертикальное смещение, уже в пикселях.
    RowOff(f32),
}

impl ImageMarker {
    /// Записать разобранную координату.
    fn apply(&mut self, value: MarkerValue) {
        match value {
            MarkerValue::Col(col) => self.col = col,
            MarkerValue::Row(row) => self.row = row,
            MarkerValue::ColOff(off) => self.col_off = off,
            MarkerValue::RowOff(off) => self.row_off = off,
        }
    }
}

/// Якорь, который собирается из текущего потока.
struct AnchorBuilder {
    /// Вид якоря.
    kind: AnchorKind,
    /// Что-то внутри якоря не разобралось: якорь пропускается целиком, чтобы
    /// картинка не уехала в угол листа по нулевым координатам.
    bad: bool,
    edit_as: Option<EditAs>,
    from: Option<ImageMarker>,
    to: Option<ImageMarker>,
    ext: Option<ImageExtent>,
    name: Option<String>,
    /// `r:embed` — id связи с media-частью.
    embed: Option<String>,
    /// Читаемая сторона маркера.
    marker: Option<MarkerSide>,
    /// Состояние `<xdr:pic>`.
    pic: PicState,
}

impl AnchorBuilder {
    fn new(kind: AnchorKind, edit_as: Option<EditAs>) -> Self {
        Self {
            kind,
            bad: false,
            edit_as,
            from: None,
            to: None,
            ext: None,
            name: None,
            embed: None,
            marker: None,
            pic: PicState::Absent,
        }
    }

    /// Начать маркер (`from` или `to`).
    fn open_marker(&mut self, side: MarkerSide) {
        match side {
            MarkerSide::From => self.from = Some(ImageMarker::default()),
            MarkerSide::To => self.to = Some(ImageMarker::default()),
        }
        self.marker = Some(side);
    }

    /// Маркер, в который пишутся координаты.
    fn marker_mut(&mut self) -> Option<&mut ImageMarker> {
        match self.marker? {
            MarkerSide::From => self.from.as_mut(),
            MarkerSide::To => self.to.as_mut(),
        }
    }
}

/// Состояние одного прохода по чертежу.
struct DrawingParser<'a> {
    /// Часть чертежа: по ней разрешаются относительные цели связей.
    part: String,
    /// Связи чертежа: по ним находится media-часть.
    rels: Option<&'a RelMap>,
    images: Vec<SheetImage>,
    anchor: Option<AnchorBuilder>,
    /// Что читается из текста.
    sink: Sink,
    /// Текст текущего элемента.
    text: String,
}

impl<'a> DrawingParser<'a> {
    fn new(part: String, rels: Option<&'a RelMap>) -> Self {
        Self {
            part,
            rels,
            images: Vec::new(),
            anchor: None,
            sink: Sink::Idle,
            text: String::new(),
        }
    }

    fn handle(&mut self, event: Event<'static>) {
        match event {
            Event::Start(element) => self.on_start(&element),
            Event::Empty(element) => self.on_empty(&element),
            Event::Text(chunk) if self.sink != Sink::Idle => {
                if let Ok(text) = chunk.xml10_content() {
                    self.text.push_str(&text);
                }
            }
            Event::End(element) => self.on_end(&element),
            _ => {}
        }
    }

    fn on_start<'e>(&mut self, element: &'e BytesStart<'e>) {
        match element.local_name().as_ref() {
            b"oneCellAnchor" | b"twoCellAnchor" => {
                let kind = if element.local_name().as_ref() == b"oneCellAnchor" {
                    AnchorKind::OneCell
                } else {
                    AnchorKind::TwoCell
                };
                let edit_as = attr(element, &self.part, "editAs")
                    .as_deref()
                    .and_then(EditAs::parse);
                self.anchor = Some(AnchorBuilder::new(kind, edit_as));
            }
            b"from" | b"to" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    let side = if element.local_name().as_ref() == b"from" {
                        MarkerSide::From
                    } else {
                        MarkerSide::To
                    };
                    anchor.open_marker(side);
                }
            }
            b"pic" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    anchor.pic = PicState::Inside;
                }
            }
            b"col" => self.open_value(Sink::Col),
            b"row" => self.open_value(Sink::Row),
            b"colOff" => self.open_value(Sink::ColOff),
            b"rowOff" => self.open_value(Sink::RowOff),
            // `ext`, `cNvPr` и `blip` несут только атрибуты и могут прийти как
            // Start (если внутри есть дети, например `extLst`), так и Empty.
            _ => self.read_attrs(element),
        }
    }

    fn on_empty<'e>(&mut self, element: &'e BytesStart<'e>) {
        match element.local_name().as_ref() {
            // Координата обязана нести текст: пустой элемент — порча файла,
            // и якорь с нулём на месте настоящей координаты хуже пропуска.
            b"col" | b"row" | b"colOff" | b"rowOff" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    if anchor.marker.is_some() {
                        anchor.bad = true;
                    }
                }
            }
            _ => self.read_attrs(element),
        }
    }

    /// Элементы, у которых всё содержимое — атрибуты.
    fn read_attrs<'e>(&mut self, element: &'e BytesStart<'e>) {
        match element.local_name().as_ref() {
            // `<a:ext>` внутри `<xdr:pic>` — размер фигуры; якорю он не указ.
            b"ext"
                if self
                    .anchor
                    .as_ref()
                    .is_some_and(|anchor| anchor.pic != PicState::Inside) =>
            {
                self.read_ext(element);
            }
            b"cNvPr" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    if anchor.name.is_none() {
                        anchor.name = attr(element, &self.part, "name");
                    }
                }
            }
            b"blip" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    anchor.embed = attr(element, &self.part, "embed");
                }
            }
            _ => {}
        }
    }

    fn on_end(&mut self, element: &BytesEnd<'_>) {
        match element.local_name().as_ref() {
            b"oneCellAnchor" | b"twoCellAnchor" => self.finish_anchor(),
            b"from" | b"to" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    anchor.marker = None;
                }
            }
            b"pic" => {
                if let Some(anchor) = self.anchor.as_mut() {
                    anchor.pic = PicState::Done;
                }
            }
            b"col" | b"row" | b"colOff" | b"rowOff" => self.take_value(),
            _ => {}
        }
    }

    /// Размер якоря one-cell (`<xdr:ext>`).
    fn read_ext<'e>(&mut self, element: &'e BytesStart<'e>) {
        let Some(cx) = attr(element, &self.part, "cx").and_then(|value| value.parse().ok()) else {
            return;
        };
        let Some(cy) = attr(element, &self.part, "cy").and_then(|value| value.parse().ok()) else {
            return;
        };
        if let Some(anchor) = self.anchor.as_mut() {
            anchor.ext = Some(ImageExtent {
                cx: emu_to_px(cx),
                cy: emu_to_px(cy),
            });
        }
    }

    fn open_value(&mut self, sink: Sink) {
        self.sink = sink;
        self.text.clear();
    }

    /// Забрать накопленный текст как координату текущего маркера.
    fn take_value(&mut self) {
        let sink = std::mem::replace(&mut self.sink, Sink::Idle);
        let raw = self.text.trim();
        let Some(anchor) = self.anchor.as_mut() else {
            return;
        };
        let value = match sink {
            Sink::Idle => return,
            Sink::Col => raw.parse::<u32>().ok().map(MarkerValue::Col),
            Sink::Row => raw.parse::<u32>().ok().map(MarkerValue::Row),
            Sink::ColOff => raw
                .parse::<f32>()
                .ok()
                .map(|off| MarkerValue::ColOff(emu_to_px(off))),
            Sink::RowOff => raw
                .parse::<f32>()
                .ok()
                .map(|off| MarkerValue::RowOff(emu_to_px(off))),
        };
        match value {
            Some(value) => {
                if let Some(marker) = anchor.marker_mut() {
                    marker.apply(value);
                }
            }
            None => anchor.bad = true,
        }
    }

    /// Закрыть текущий якорь, если из него получилось изображение.
    fn finish_anchor(&mut self) {
        let Some(anchor) = self.anchor.take() else {
            return;
        };
        if anchor.bad || anchor.pic == PicState::Absent {
            return;
        }
        // Ссылка может не разрешиться: битая связь — не повод терять якорь, но
        // рисовать по ней нечего.
        let media = anchor
            .embed
            .as_deref()
            .and_then(|id| self.rels?.get(id))
            .and_then(|rel| rel.part(&self.part));
        let shape = match anchor.kind {
            AnchorKind::OneCell => {
                let (Some(from), Some(ext)) = (anchor.from, anchor.ext) else {
                    return;
                };
                ImageAnchor::OneCell { from, ext }
            }
            AnchorKind::TwoCell => {
                let (Some(from), Some(to)) = (anchor.from, anchor.to) else {
                    return;
                };
                ImageAnchor::TwoCell { from, to }
            }
        };
        self.images.push(SheetImage {
            name: anchor.name,
            media,
            edit_as: anchor.edit_as,
            anchor: shape,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use doc_converter_core::rels::Relationship;

    use super::*;

    /// Связи чертежа: id → цель.
    fn rels(items: &[(&str, &str)]) -> RelMap {
        RelMap {
            items: items
                .iter()
                .map(|(id, target)| {
                    let item = Relationship {
                        id: (*id).to_owned(),
                        rel_type:
                            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
                                .to_owned(),
                        target: (*target).to_owned(),
                        target_mode: None,
                    };
                    ((*id).to_owned(), item)
                })
                .collect::<HashMap<_, _>>(),
        }
    }

    fn parse_str(xml: &str, rels: Option<&RelMap>) -> Vec<SheetImage> {
        parse(xml.as_bytes(), "xl/drawings/drawing1.xml", rels).unwrap()
    }

    /// One-cell якорь с картинкой: смещения и `ext` переводятся в пиксели, а
    /// размер фигуры из `spPr` их не перебивает.
    #[test]
    fn one_cell_anchor_keeps_offsets_and_extent() {
        let rels = rels(&[("rId1", "../media/image1.png")]);
        let xml = r#"<?xml version="1.0"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
          xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <xdr:oneCellAnchor editAs="oneCell">
    <xdr:from><xdr:col>1</xdr:col><xdr:colOff>9525</xdr:colOff>
              <xdr:row>2</xdr:row><xdr:rowOff>19050</xdr:rowOff></xdr:from>
    <xdr:ext cx="95250" cy="47625"/>
    <xdr:pic>
      <xdr:nvPicPr><xdr:cNvPr id="1" name="Picture 1"/><xdr:cNvPicPr/></xdr:nvPicPr>
      <xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill>
      <xdr:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></a:xfrm></xdr:spPr>
    </xdr:pic>
    <xdr:clientData/>
  </xdr:oneCellAnchor>
</xdr:wsDr>"#;

        let images = parse_str(xml, Some(&rels));

        assert_eq!(images.len(), 1);
        let image = &images[0];
        assert_eq!(image.name.as_deref(), Some("Picture 1"));
        assert_eq!(image.media.as_deref(), Some("xl/media/image1.png"));
        assert_eq!(image.edit_as, Some(EditAs::OneCell));
        match image.anchor {
            ImageAnchor::OneCell { from, ext } => {
                assert_eq!((from.col, from.row), (1, 2));
                assert!((from.col_off - 1.0).abs() < 1e-6, "{from:?}");
                assert!((from.row_off - 2.0).abs() < 1e-6, "{from:?}");
                // 10×5 пикселей, а не нули из `<a:ext>` внутри `<xdr:pic>`.
                assert!((ext.cx - 10.0).abs() < 1e-6, "{ext:?}");
                assert!((ext.cy - 5.0).abs() < 1e-6, "{ext:?}");
            }
            other @ ImageAnchor::TwoCell { .. } => panic!("ожидался one-cell якорь: {other:?}"),
        }
    }

    /// Two-cell якорь: размер задают границы, в том числе смещения внутри
    /// конечной ячейки.
    #[test]
    fn two_cell_anchor_keeps_both_corners() {
        let xml = r#"<?xml version="1.0"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing">
  <xdr:twoCellAnchor>
    <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff>
              <xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:to><xdr:col>3</xdr:col><xdr:colOff>9525</xdr:colOff>
            <xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
    <xdr:pic>
      <xdr:nvPicPr><xdr:cNvPr id="1" name="Picture 1"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                            r:embed="rId1"/></xdr:blipFill>
    </xdr:pic>
  </xdr:twoCellAnchor>
</xdr:wsDr>"#;

        let images = parse_str(xml, None);

        assert_eq!(images.len(), 1);
        let image = &images[0];
        assert_eq!(image.edit_as, None);
        // Связи нет — якорь остаётся, media не разрешается.
        assert_eq!(image.media, None);
        match image.anchor {
            ImageAnchor::TwoCell { from, to } => {
                assert_eq!((from.col, from.row), (0, 0));
                assert_eq!((to.col, to.row), (3, 4));
                assert!((to.col_off - 1.0).abs() < 1e-6, "{to:?}");
            }
            other @ ImageAnchor::OneCell { .. } => panic!("ожидался two-cell якорь: {other:?}"),
        }
    }

    /// Незнакомый вид якоря, неполный якорь и битая координата пропускаются,
    /// не роняя разбор.
    #[test]
    fn broken_and_unknown_anchors_are_skipped() {
        let xml = r#"<?xml version="1.0"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing">
  <xdr:absoluteAnchor>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="1" name="Picture 1"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                            r:embed="rId1"/></xdr:blipFill></xdr:pic>
  </xdr:absoluteAnchor>
  <xdr:twoCellAnchor>
    <xdr:from><xdr:col>0</xdr:col><xdr:row>0</xdr:row></xdr:from>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Picture 2"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                            r:embed="rId1"/></xdr:blipFill></xdr:pic>
  </xdr:twoCellAnchor>
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>абв</xdr:col><xdr:row>0</xdr:row></xdr:from>
    <xdr:ext cx="9525" cy="9525"/>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="3" name="Picture 3"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                            r:embed="rId1"/></xdr:blipFill></xdr:pic>
  </xdr:oneCellAnchor>
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>0</xdr:col><xdr:row>0</xdr:row></xdr:from>
    <xdr:ext cx="9525" cy="9525"/>
    <xdr:sp><xdr:nvSpPr><xdr:cNvPr id="4" name="Прямоугольник"/></xdr:nvSpPr></xdr:sp>
  </xdr:oneCellAnchor>
</xdr:wsDr>"#;

        assert!(parse_str(xml, None).is_empty());
    }

    /// Внешняя цель связи — не часть пакета: media не разрешается.
    #[test]
    fn external_media_link_has_no_part() {
        let mut map = rels(&[("rId1", "https://example.com/picture.png")]);
        if let Some(rel) = map.items.get_mut("rId1") {
            rel.target_mode = Some("External".to_owned());
        }
        let xml = r#"<?xml version="1.0"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing">
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>0</xdr:col><xdr:row>0</xdr:row></xdr:from>
    <xdr:ext cx="9525" cy="9525"/>
    <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="1" name="Picture 1"/></xdr:nvPicPr>
      <xdr:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                            r:embed="rId1"/></xdr:blipFill></xdr:pic>
  </xdr:oneCellAnchor>
</xdr:wsDr>"#;

        let images = parse_str(xml, Some(&map));

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media, None);
    }
}
