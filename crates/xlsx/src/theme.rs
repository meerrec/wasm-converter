//! Разбор `xl/theme/theme1.xml`.
//!
//! Тема — часть `DrawingML`, а не `SpreadsheetML`: элементы приходят с префиксом
//! `a:`. Из всей темы нужны палитра `<a:clrScheme>` и схема шрифтов
//! `<a:fontScheme>`; остальное (`fmtScheme`, `objectDefaults`, `extLst`)
//! пропускается.
//!
//! Порядок элементов в `<a:clrScheme>` не совпадает с порядком индексов
//! `SpreadsheetML`, к которым отсылает `theme="n"` в `styles.xml`: в файле `dk1`
//! записан раньше `lt1`, а индекс 0 — это именно `lt1`. Соответствие задаёт
//! [`SLOTS`], и путать его нельзя: иначе цвет текста по умолчанию (`theme="1"`)
//! стал бы белым вместо чёрного.

use doc_converter_core::xml::XmlReader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use crate::error::Result;
use crate::model::{Color, Theme, THEME_COLOR_COUNT};
use crate::xml::{attributes, find, Attr};

/// Слоты `<a:clrScheme>` по индексам `SpreadsheetML` (ECMA-376, часть 1).
const SLOTS: [&[u8]; THEME_COLOR_COUNT] = [
    b"lt1",
    b"dk1",
    b"lt2",
    b"dk2",
    b"accent1",
    b"accent2",
    b"accent3",
    b"accent4",
    b"accent5",
    b"accent6",
    b"hlink",
    b"folHlink",
];

/// Разобрать `xl/theme/theme1.xml`.
///
/// # Errors
///
/// [`XlsxError::Core`](crate::XlsxError::Core) — XML не разбирается;
/// [`XlsxError::Malformed`](crate::XlsxError::Malformed) — атрибут элемента не
/// читается.
pub fn parse(bytes: &[u8], part: impl Into<String>) -> Result<Theme> {
    let part = part.into();
    let mut reader = XmlReader::new(bytes, part.clone());
    let mut parser = Parser::new(part);

    while let Some(event) = reader.next_significant()? {
        parser.handle(event)?;
    }

    Ok(parser.finish())
}

/// Схема шрифтов, внутри которой находимся.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontSlot {
    Major,
    Minor,
}

/// Состояние разбора: какой слот палитры открыт и какую схему читаем.
#[derive(Debug)]
struct Parser {
    part: String,
    colors: [Color; THEME_COLOR_COUNT],
    major_font: Option<String>,
    minor_font: Option<String>,
    /// Индекс открытого слота `<a:clrScheme>`; цвет придёт вложенным элементом.
    slot: Option<usize>,
    font_slot: Option<FontSlot>,
}

impl Parser {
    fn new(part: String) -> Self {
        Self {
            part,
            colors: [Color::None; THEME_COLOR_COUNT],
            major_font: None,
            minor_font: None,
            slot: None,
            font_slot: None,
        }
    }

    fn handle(&mut self, event: Event<'static>) -> Result<()> {
        match event {
            Event::Start(element) => self.element(&element, true),
            // Пустой элемент значит то же, что открытый и сразу закрытый;
            // контейнеры он не открывает.
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

        if let Some(index) = slot_index(name.as_ref()) {
            // Пустой слот цвета не несёт, поэтому его не открываем.
            if is_start {
                self.slot = Some(index);
            }
            return Ok(());
        }

        match name.as_ref() {
            b"srgbClr" => self.set_color(srgb_color(&attrs)),
            b"sysClr" => self.set_color(Some(sys_color(&attrs))),
            b"majorFont" => {
                self.font_slot = if is_start {
                    Some(FontSlot::Major)
                } else {
                    None
                };
            }
            b"minorFont" => {
                self.font_slot = if is_start {
                    Some(FontSlot::Minor)
                } else {
                    None
                };
            }
            b"latin" => self.set_font(find(&attrs, "typeface")),
            _ => {}
        }
        Ok(())
    }

    /// Записать цвет открытого слота палитры; без слота или без цвета — мимо.
    fn set_color(&mut self, color: Option<Color>) {
        if let (Some(index), Some(color)) = (self.slot, color) {
            if let Some(slot) = self.colors.get_mut(index) {
                *slot = color;
            }
        }
    }

    /// Записать латинскую гарнитуру открытой схемы шрифтов.
    fn set_font(&mut self, typeface: Option<&str>) {
        let (Some(slot), Some(typeface)) = (self.font_slot, typeface) else {
            return;
        };
        // `<a:ea typeface=""/>` — пустое имя; пустая гарнитура не значит ничего.
        if typeface.is_empty() {
            return;
        }
        let typeface = typeface.to_owned();
        match slot {
            FontSlot::Major => self.major_font = Some(typeface),
            FontSlot::Minor => self.minor_font = Some(typeface),
        }
    }

    /// Закрывающий элемент: слот палитры или схема шрифтов кончились.
    fn end(&mut self, element: &BytesEnd<'_>) {
        let name = element.local_name();
        if slot_index(name.as_ref()).is_some() {
            self.slot = None;
            return;
        }
        if matches!(name.as_ref(), b"majorFont" | b"minorFont") {
            self.font_slot = None;
        }
    }

    fn finish(self) -> Theme {
        Theme::new(self.colors, self.major_font, self.minor_font)
    }
}

/// Индекс `SpreadsheetML` для элемента `<a:clrScheme>`; `None` — это не цвет.
fn slot_index(name: &[u8]) -> Option<usize> {
    SLOTS.iter().position(|slot| *slot == name)
}

/// Цвет `<a:srgbClr val="RRGGBB"/>`.
fn srgb_color(attrs: &[Attr<'_>]) -> Option<Color> {
    hex_color(find(attrs, "val")?)
}

/// Системный цвет `<a:sysClr val="windowText" lastClr="000000"/>`.
///
/// `lastClr` — то, чем цвет отрисовался в последний раз; по спецификации он
/// необязателен. Без него берём системные значения по умолчанию: окно
/// (`window`) светлое, текст и всё прочее тёмные. Иначе `dk1` без `lastClr`
/// оставил бы цвет текста неразрешённым, и текст исчез бы.
fn sys_color(attrs: &[Attr<'_>]) -> Color {
    if let Some(last) = find(attrs, "lastClr").and_then(hex_color) {
        return last;
    }
    match find(attrs, "val") {
        Some("window") => Color::Rgb(0xFFFF_FFFF),
        _ => Color::Rgb(0xFF00_0000),
    }
}

/// Шесть шестнадцатеричных цифр `RRGGBB`; цвет непрозрачный, поэтому в
/// `AARRGGBB` дописывается `FF`.
fn hex_color(value: &str) -> Option<Color> {
    let rgb = u32::from_str_radix(value.trim(), 16).ok()? & 0x00FF_FFFF;
    Some(Color::Rgb(0xFF00_0000 | rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PART: &str = "xl/theme/theme1.xml";

    /// Тема, как её пишет Excel: в XML `dk1`/`lt1` идут раньше `dk2`/`lt2`,
    /// а индексы `SpreadsheetML` считают `lt1` нулевым.
    const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
        <a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
          <a:themeElements>
            <a:clrScheme name="Office">
              <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
              <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
              <a:dk2><a:srgbClr val="1F497D"/></a:dk2>
              <a:lt2><a:srgbClr val="EEECE1"/></a:lt2>
              <a:accent1><a:srgbClr val="4F81BD"/></a:accent1>
              <a:accent2><a:srgbClr val="C0504D"/></a:accent2>
              <a:accent3><a:srgbClr val="9BBB59"/></a:accent3>
              <a:accent4><a:srgbClr val="8064A2"/></a:accent4>
              <a:accent5><a:srgbClr val="4BACC6"/></a:accent5>
              <a:accent6><a:srgbClr val="F79646"/></a:accent6>
              <a:hlink><a:srgbClr val="0000FF"/></a:hlink>
              <a:folHlink><a:srgbClr val="800080"/></a:folHlink>
            </a:clrScheme>
            <a:fontScheme name="Office">
              <a:majorFont><a:latin typeface="Cambria"/><a:ea typeface=""/></a:majorFont>
              <a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/></a:minorFont>
            </a:fontScheme>
          </a:themeElements>
        </a:theme>"#;

    fn theme() -> Theme {
        parse(THEME.as_bytes(), PART).unwrap()
    }

    #[test]
    fn colors_follow_spreadsheetml_indices() {
        let theme = theme();

        // 0 — `lt1` (фон), 1 — `dk1` (текст): в XML `dk1` записан первым, но
        // `theme="1"` в стилях — это именно текст.
        assert_eq!(theme.color(0), Some(Color::Rgb(0xFFFF_FFFF)));
        assert_eq!(theme.color(1), Some(Color::Rgb(0xFF00_0000)));
        assert_eq!(theme.color(2), Some(Color::Rgb(0xFFEE_ECE1)));
        assert_eq!(theme.color(3), Some(Color::Rgb(0xFF1F_497D)));
        assert_eq!(theme.color(4), Some(Color::Rgb(0xFF4F_81BD)));
        assert_eq!(theme.color(9), Some(Color::Rgb(0xFFF7_9646)));
        assert_eq!(theme.color(10), Some(Color::Rgb(0xFF00_00FF)));
        assert_eq!(theme.color(11), Some(Color::Rgb(0xFF80_0080)));
        assert_eq!(theme.color(12), None);
    }

    #[test]
    fn system_color_without_last_clr_falls_back() {
        let theme = parse(
            br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                  <a:clrScheme>
                    <a:dk1><a:sysClr val="windowText"/></a:dk1>
                    <a:lt1><a:sysClr val="window"/></a:lt1>
                  </a:clrScheme>
                </a:theme>"#,
            PART,
        )
        .unwrap();

        assert_eq!(
            theme.color(1),
            Some(Color::Rgb(0xFF00_0000)),
            "текст тёмный"
        );
        assert_eq!(
            theme.color(0),
            Some(Color::Rgb(0xFFFF_FFFF)),
            "окно светлое"
        );
    }

    #[test]
    fn color_with_nested_transform_keeps_its_value() {
        // Внутри цвета могут лежать `<a:alpha/>`, `<a:lumMod/>` и прочие
        // правки; базовое значение при этом не теряется.
        let theme = parse(
            br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                  <a:clrScheme>
                    <a:accent1><a:srgbClr val="4F81BD"><a:lumMod val="80000"/></a:srgbClr></a:accent1>
                  </a:clrScheme>
                </a:theme>"#,
            PART,
        )
        .unwrap();

        assert_eq!(theme.color(4), Some(Color::Rgb(0xFF4F_81BD)));
    }

    #[test]
    fn font_scheme_is_parsed() {
        let theme = theme();

        assert_eq!(theme.major_font(), Some("Cambria"));
        assert_eq!(theme.minor_font(), Some("Calibri"));
    }

    #[test]
    fn missing_elements_leave_an_empty_theme() {
        let theme = parse(
            br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                  <a:themeElements/>
                </a:theme>"#,
            PART,
        )
        .unwrap();

        assert_eq!(theme, Theme::default());
        assert_eq!(theme.color(1), None);
        assert_eq!(theme.major_font(), None);
        assert_eq!(theme.minor_font(), None);
    }

    #[test]
    fn colors_outside_the_scheme_are_ignored() {
        // `srgbClr` встречается и в других частях темы (`fmtScheme`,
        // `objectDefaults`); в палитру он попадать не должен.
        let theme = parse(
            br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                  <a:themeElements>
                    <a:fmtScheme><a:fillStyleLst><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:fillStyleLst></a:fmtScheme>
                  </a:themeElements>
                </a:theme>"#,
            PART,
        )
        .unwrap();

        assert_eq!(theme, Theme::default());
    }
}
