//! Разбор `xl/styles.xml`.
//!
//! Таблицы шрифтов, заливок, рамок и форматов чисел читаются одним проходом.
//! Отдельная забота — `cellXfs`: каждый формат ссылается на базовый
//! (`xfId` → `cellStyleXfs`), и по ECMA-376 группа, для которой явно задан
//! `applyX="false"`, наследуется от базового формата. Если флаг не задан вовсе,
//! берём значение самого `cellXf`: так пишет Excel, когда значение своё, и так
//! же поступают генераторы, которые флаги не ставят вообще.

use std::collections::BTreeMap;

use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use crate::error::Result;
use crate::model::{
    Border, BorderSide, BorderStyle, CellFormat, Color, Fill, FillPattern, Font, StyleTable,
};
use crate::xml::{attributes, find, is_true, Attr};

/// Разобрать `xl/styles.xml`.
///
/// # Errors
///
/// [`XlsxError::Core`](crate::XlsxError::Core) — XML не разбирается;
/// [`XlsxError::Malformed`](crate::XlsxError::Malformed) — атрибут элемента не
/// читается.
pub fn parse(bytes: &[u8], part: impl Into<String>) -> Result<StyleTable> {
    let part = part.into();
    let mut reader = XmlReader::new(bytes, part.clone());
    let mut parser = StyleParser::new(part);

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }

    Ok(parser.finish())
}

/// Контейнер верхнего уровня, внутри которого мы находимся.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Ничего из интересного: `dxfs`, `tableStyles`, `extLst` и прочее.
    Other,
    Fonts,
    Fills,
    Borders,
    NumberFormats,
    CellStyleXfs,
    CellXfs,
}

impl Section {
    /// Контейнер по имени элемента; `None` — это не контейнер.
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"fonts" => Some(Self::Fonts),
            b"fills" => Some(Self::Fills),
            b"borders" => Some(Self::Borders),
            b"numFmts" => Some(Self::NumberFormats),
            b"cellStyleXfs" => Some(Self::CellStyleXfs),
            b"cellXfs" => Some(Self::CellXfs),
            _ => None,
        }
    }
}

/// Сторона рамки, которую сейчас читаем.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
    Diagonal,
}

impl Side {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"left" => Some(Self::Left),
            b"right" => Some(Self::Right),
            b"top" => Some(Self::Top),
            b"bottom" => Some(Self::Bottom),
            b"diagonal" => Some(Self::Diagonal),
            _ => None,
        }
    }

    fn of(self, border: &mut Border) -> &mut BorderSide {
        match self {
            Self::Left => &mut border.left,
            Self::Right => &mut border.right,
            Self::Top => &mut border.top,
            Self::Bottom => &mut border.bottom,
            Self::Diagonal => &mut border.diagonal,
        }
    }
}

/// Какие группы `cellXf` явно помечены как «не применять» (`None` — флага нет).
#[derive(Debug, Clone, Copy, Default)]
struct ApplyFlags {
    number_format: Option<bool>,
    font: Option<bool>,
    fill: Option<bool>,
    border: Option<bool>,
}

/// Сырой `xf` из `cellXfs`: разрешается, когда прочитаны `cellStyleXfs`.
#[derive(Debug, Clone, Copy)]
struct RawFormat {
    format: CellFormat,
    /// Индекс базового формата (`xfId`).
    base: u32,
    apply: ApplyFlags,
}

impl RawFormat {
    /// Разрешить формат через базовый.
    fn resolve(self, base: Option<&CellFormat>) -> CellFormat {
        let Some(base) = base else {
            return self.format;
        };
        CellFormat {
            num_fmt: group(self.apply.number_format, self.format.num_fmt, base.num_fmt),
            font: group(self.apply.font, self.format.font, base.font),
            fill: group(self.apply.fill, self.format.fill, base.fill),
            border: group(self.apply.border, self.format.border, base.border),
        }
    }
}

/// Снятый флаг (`applyX="false"`) означает наследование от базового формата.
fn group(apply: Option<bool>, own: u32, base: u32) -> u32 {
    if apply == Some(false) {
        base
    } else {
        own
    }
}

/// Состояние одного прохода по `styles.xml`.
struct StyleParser {
    part: String,
    section: Section,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    number_formats: BTreeMap<u32, String>,
    /// `cellStyleXfs` — базовые форматы для цепочки `xfId`.
    base_formats: Vec<CellFormat>,
    raw_formats: Vec<RawFormat>,
    font: Option<Font>,
    fill: Option<Fill>,
    border: Option<Border>,
    side: Option<Side>,
}

impl StyleParser {
    fn new(part: String) -> Self {
        Self {
            part,
            section: Section::Other,
            fonts: Vec::new(),
            fills: Vec::new(),
            borders: Vec::new(),
            number_formats: BTreeMap::new(),
            base_formats: Vec::new(),
            raw_formats: Vec::new(),
            font: None,
            fill: None,
            border: None,
            side: None,
        }
    }

    fn handle(&mut self, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.element(&element, true),
            // Пустой элемент: `<b/>`, `<numFmt/>`, `<font/>` — всё они значат то
            // же, что открытый и сразу закрытый тег.
            Event::Empty(element) => self.element(&element, false),
            Event::End(element) => {
                self.end(&element);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Обработать открывающий (или пустой) элемент.
    fn element(&mut self, element: &BytesStart<'_>, is_start: bool) -> Result<()> {
        let name = element.local_name();
        let attrs = attributes(element, &self.part)?;

        match name.as_ref() {
            // Пустой контейнер ничего не открывает: иначе он оставил бы секцию
            // включённой до конца файла.
            _ if Section::from_name(name.as_ref()).is_some() => {
                if is_start {
                    self.section = Section::from_name(name.as_ref()).unwrap_or(Section::Other);
                }
            }
            b"font" if self.section == Section::Fonts => {
                self.font = Some(Font::default());
                if !is_start {
                    if let Some(font) = self.font.take() {
                        self.fonts.push(font);
                    }
                }
            }
            b"fill" if self.section == Section::Fills => {
                self.fill = Some(Fill::default());
                if !is_start {
                    if let Some(fill) = self.fill.take() {
                        self.fills.push(fill);
                    }
                }
            }
            b"border" if self.section == Section::Borders => {
                let border = Border {
                    diagonal_up: find(&attrs, "diagonalUp").is_some_and(is_true),
                    diagonal_down: find(&attrs, "diagonalDown").is_some_and(is_true),
                    ..Border::default()
                };
                self.border = Some(border);
                if !is_start {
                    if let Some(border) = self.border.take() {
                        self.borders.push(border);
                    }
                }
            }
            b"patternFill" => {
                if let Some(fill) = self.fill.as_mut() {
                    fill.pattern =
                        find(&attrs, "patternType").map_or(FillPattern::None, FillPattern::parse);
                }
            }
            b"fgColor" => {
                if let Some(fill) = self.fill.as_mut() {
                    fill.foreground = color(&attrs);
                }
            }
            b"bgColor" => {
                if let Some(fill) = self.fill.as_mut() {
                    fill.background = color(&attrs);
                }
            }
            b"left" | b"right" | b"top" | b"bottom" | b"diagonal" if self.border.is_some() => {
                let side = Side::from_name(name.as_ref());
                if let (Some(side), Some(border)) = (side, self.border.as_mut()) {
                    side.of(border).style =
                        find(&attrs, "style").map_or(BorderStyle::None, BorderStyle::parse);
                }
                // Цвет стороны придёт отдельным `<color/>`, поэтому запоминаем,
                // чью сторону читаем, только если элемент не закрылся сразу.
                self.side = if is_start { side } else { None };
            }
            b"color" => self.set_color(&attrs),
            b"sz" => {
                if let Some(font) = self.font.as_mut() {
                    if let Some(value) = find(&attrs, "val").and_then(|v| v.trim().parse().ok()) {
                        font.size = value;
                    }
                }
            }
            b"name" => {
                if let Some(font) = self.font.as_mut() {
                    if let Some(value) = find(&attrs, "val") {
                        value.clone_into(&mut font.name);
                    }
                }
            }
            b"b" | b"i" | b"strike" => {
                if let Some(font) = self.font.as_mut() {
                    // Элемент без `val` означает «включено».
                    let on = find(&attrs, "val").is_none_or(is_true);
                    match name.as_ref() {
                        b"b" => font.bold = on,
                        b"i" => font.italic = on,
                        _ => font.strike = on,
                    }
                }
            }
            b"u" => {
                if let Some(font) = self.font.as_mut() {
                    // Кроме `none` подчёркивание выключает и `val="0"`, а любой
                    // другой стиль линии (`single`, `double`, …) его включает.
                    font.underline = find(&attrs, "val").is_none_or(|v| v != "none" && v != "0");
                }
            }
            b"xf" => self.push_format(&attrs),
            b"numFmt" => self.push_number_format(&attrs),
            _ => {}
        }

        Ok(())
    }

    /// Обработать закрывающий элемент.
    fn end(&mut self, element: &BytesEnd<'_>) {
        let name = element.local_name();
        if Section::from_name(name.as_ref()).is_some() {
            self.section = Section::Other;
            return;
        }
        match name.as_ref() {
            b"font" => {
                if let Some(font) = self.font.take() {
                    self.fonts.push(font);
                }
            }
            b"fill" => {
                if let Some(fill) = self.fill.take() {
                    self.fills.push(fill);
                }
            }
            b"border" => {
                if let Some(border) = self.border.take() {
                    self.borders.push(border);
                }
            }
            b"left" | b"right" | b"top" | b"bottom" | b"diagonal" => self.side = None,
            _ => {}
        }
    }

    /// Цвет текущего контекста: шрифта или стороны рамки.
    fn set_color(&mut self, attrs: &[Attr<'_>]) {
        let value = color(attrs);
        if let Some(side) = self.side {
            if let Some(border) = self.border.as_mut() {
                side.of(border).color = value;
            }
        } else if let Some(font) = self.font.as_mut() {
            font.color = value;
        }
    }

    /// Записать `xf` в базовые форматы или в `cellXfs`.
    fn push_format(&mut self, attrs: &[Attr<'_>]) {
        let format = CellFormat {
            num_fmt: number(attrs, "numFmtId"),
            font: number(attrs, "fontId"),
            fill: number(attrs, "fillId"),
            border: number(attrs, "borderId"),
        };
        match self.section {
            Section::CellStyleXfs => self.base_formats.push(format),
            Section::CellXfs => self.raw_formats.push(RawFormat {
                format,
                base: number(attrs, "xfId"),
                apply: ApplyFlags {
                    number_format: flag(attrs, "applyNumberFormat"),
                    font: flag(attrs, "applyFont"),
                    fill: flag(attrs, "applyFill"),
                    border: flag(attrs, "applyBorder"),
                },
            }),
            _ => {}
        }
    }

    /// Записать пользовательский формат числа.
    fn push_number_format(&mut self, attrs: &[Attr<'_>]) {
        let (Some(id), Some(code)) = (find(attrs, "numFmtId"), find(attrs, "formatCode")) else {
            return;
        };
        let Ok(id) = id.trim().parse() else {
            return;
        };
        self.number_formats.insert(id, code.to_owned());
    }

    /// Разрешить `cellXfs` через базовые форматы.
    fn finish(self) -> StyleTable {
        let Self {
            fonts,
            fills,
            borders,
            number_formats,
            base_formats,
            raw_formats,
            ..
        } = self;

        let formats = raw_formats
            .into_iter()
            .map(|raw| raw.resolve(base_formats.get(raw.base as usize)))
            .collect();

        StyleTable::new(formats, fonts, fills, borders, number_formats)
    }
}

/// Целочисленный атрибут; битое значение считается нулём.
fn number(attrs: &[Attr<'_>], name: &str) -> u32 {
    find(attrs, name)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// Необязательный флаг `applyX`.
fn flag(attrs: &[Attr<'_>], name: &str) -> Option<bool> {
    find(attrs, name).map(is_true)
}

/// Цвет из атрибутов `rgb`, `theme` или `indexed`.
///
/// `rgb` бывает восьмизначным (`AARRGGBB`) и шестизначным (`RRGGBB`); во втором
/// случае альфы в файле нет, и она считается непрозрачной.
fn color(attrs: &[Attr<'_>]) -> Color {
    if let Some(rgb) = find(attrs, "rgb") {
        if let Ok(value) = u32::from_str_radix(rgb.trim().trim_start_matches('#'), 16) {
            return Color::Rgb(value);
        }
    }
    if let Some(theme) = find(attrs, "theme").and_then(|value| value.trim().parse().ok()) {
        return Color::Theme(theme);
    }
    if let Some(indexed) = find(attrs, "indexed").and_then(|value| value.trim().parse().ok()) {
        return Color::Indexed(indexed);
    }
    Color::None
}

#[cfg(test)]
// Кегли в тестах — точные литералы (`11`, `14.5`), оба представимы в `f32`,
// поэтому сравнение на равенство здесь осмысленно.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const PART: &str = "xl/styles.xml";

    fn styles(body: &str) -> StyleTable {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
               <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</styleSheet>"#
        );
        parse(xml.as_bytes(), PART).unwrap()
    }

    #[test]
    fn reads_fonts() {
        let table = styles(
            r#"<fonts count="2">
                 <font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/></font>
                 <font><b/><i/><u/><strike/><sz val="14.5"/><color rgb="FFFF0000"/><name val="Arial"/></font>
               </fonts>"#,
        );

        let first = table.font(0).unwrap();
        assert_eq!(first.name, "Calibri");
        assert_eq!(first.size, 11.0);
        assert_eq!(first.color, Color::Theme(1));
        assert!(!first.bold);

        let second = table.font(1).unwrap();
        assert_eq!(second.name, "Arial");
        assert_eq!(second.size, 14.5);
        assert_eq!(second.color, Color::Rgb(0xFFFF_0000));
        assert!(second.bold && second.italic && second.underline && second.strike);
        assert_eq!(table.font(2), None);
    }

    #[test]
    fn flags_can_be_switched_off() {
        let table = styles(
            r#"<fonts><font><b val="0"/><u val="none"/><sz val="9"/><name val="X"/></font></fonts>"#,
        );

        let font = table.font(0).unwrap();
        assert!(!font.bold);
        assert!(!font.underline);
    }

    #[test]
    fn underline_styles_count_as_underline() {
        let table = styles(
            r#"<fonts><font><u val="double"/></font><font><u val="single"/></font></fonts>"#,
        );

        assert!(table.font(0).unwrap().underline);
        assert!(table.font(1).unwrap().underline);
    }

    #[test]
    fn reads_fills() {
        let table = styles(
            r#"<fills count="3">
                 <fill><patternFill patternType="none"/></fill>
                 <fill><patternFill patternType="gray125"/></fill>
                 <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill>
               </fills>"#,
        );

        assert_eq!(table.fill(0).unwrap().pattern, FillPattern::None);
        assert_eq!(table.fill(1).unwrap().pattern, FillPattern::Gray125);

        let solid = table.fill(2).unwrap();
        assert_eq!(solid.pattern, FillPattern::Solid);
        assert_eq!(solid.foreground, Color::Rgb(0xFFFF_FF00));
        assert_eq!(solid.background, Color::Indexed(64));
        assert_eq!(table.fill(3), None);
    }

    #[test]
    fn reads_borders() {
        let table = styles(
            r#"<borders count="2">
                 <border><left/><right/><top/><bottom/><diagonal/></border>
                 <border diagonalUp="1">
                   <left style="thin"><color rgb="FF000000"/></left>
                   <right style="double"/>
                   <bottom style="mediumDashed"><color theme="4"/></bottom>
                 </border>
               </borders>"#,
        );

        let plain = table.border(0).unwrap();
        assert_eq!(plain.left.style, BorderStyle::None);
        assert!(!plain.diagonal_up);

        let fancy = table.border(1).unwrap();
        assert_eq!(fancy.left.style, BorderStyle::Thin);
        assert_eq!(fancy.left.color, Color::Rgb(0xFF00_0000));
        assert_eq!(fancy.right.style, BorderStyle::Double);
        assert_eq!(fancy.bottom.style, BorderStyle::MediumDashed);
        assert_eq!(fancy.bottom.color, Color::Theme(4));
        assert!(fancy.diagonal_up);
        assert!(!fancy.diagonal_down);
        assert_eq!(fancy.top.style, BorderStyle::None);
    }

    #[test]
    fn reads_custom_number_formats() {
        let table = styles(
            r##"<numFmts count="2">
                 <numFmt numFmtId="164" formatCode="0.00%"/>
                 <numFmt numFmtId="165" formatCode="# ##0,00 &quot;₽&quot;"/>
               </numFmts>"##,
        );

        assert_eq!(table.number_format(164), Some("0.00%"));
        // Кавычки в коде формата приходят развёрнутыми из сущностей.
        assert_eq!(table.number_format(165), Some("# ##0,00 \"₽\""));
        assert_eq!(table.number_format(0), None);
        assert_eq!(table.number_formats().count(), 2);
    }

    #[test]
    fn reads_cell_formats() {
        let table = styles(
            r#"<cellXfs count="2">
                 <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
                 <xf numFmtId="164" fontId="1" fillId="2" borderId="1" xfId="0" applyNumberFormat="1"/>
               </cellXfs>"#,
        );

        assert_eq!(table.len(), 2);
        let plain = table.get(0).unwrap();
        assert_eq!(*plain, CellFormat::default());
        assert_eq!(
            *table.get(1).unwrap(),
            CellFormat {
                num_fmt: 164,
                font: 1,
                fill: 2,
                border: 1,
            }
        );
    }

    #[test]
    fn inherited_groups_come_from_the_base_format() {
        let table = styles(
            r#"<cellStyleXfs count="1">
                 <xf numFmtId="14" fontId="1" fillId="0" borderId="0"/>
               </cellStyleXfs>
               <cellXfs count="2">
                 <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="0"/>
                 <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
               </cellXfs>"#,
        );

        // Флаг снят явно — формат числа берётся у базового.
        assert_eq!(table.get(0).unwrap().num_fmt, 14);
        // Флага нет — значение своё: так пишут генераторы, не ставящие applyX.
        assert_eq!(table.get(1).unwrap().num_fmt, 0);
    }

    #[test]
    fn unknown_base_index_is_not_fatal() {
        let table = styles(
            r#"<cellXfs count="1"><xf numFmtId="164" xfId="7" applyNumberFormat="0"/></cellXfs>"#,
        );

        assert_eq!(table.get(0).unwrap().num_fmt, 164);
    }

    #[test]
    fn tolerates_broken_values() {
        let table = styles(
            r#"<fonts><font><sz val="огромный"/><color rgb="zzz"/><name val="X"/></font></fonts>
               <cellXfs><xf numFmtId="не число" fontId=""/></cellXfs>"#,
        );

        // Кегль остаётся значением по умолчанию, цвет — «не задан».
        assert_eq!(table.font(0).unwrap().size, Font::default().size);
        assert_eq!(table.font(0).unwrap().color, Color::None);
        assert_eq!(table.get(0).unwrap().num_fmt, 0);
    }

    #[test]
    fn empty_stylesheet_gives_an_empty_table() {
        let table = styles("");

        assert!(table.is_empty());
        assert_eq!(table.font(0), None);
        assert_eq!(table.number_format(164), None);
    }

    #[test]
    fn sections_do_not_leak_into_each_other() {
        let table = styles(
            r#"<fonts><font><name val="Calibri"/></font></fonts>
               <fills><fill><patternFill patternType="solid"/></fill></fills>
               <borders><border diagonalDown="1"><left style="thin"/></border></borders>
               <cellXfs><xf fontId="0"/></cellXfs>"#,
        );

        assert_eq!(table.fonts().count(), 1);
        assert_eq!(table.fills().count(), 1);
        assert_eq!(table.borders().count(), 1);
        assert_eq!(table.len(), 1);
        assert!(table.border(0).unwrap().diagonal_down);
    }
}
