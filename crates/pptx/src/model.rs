//! Нормализованная модель презентации: слайды, шейпы, тема, media-части.
//!
//! Модель повторяет `PresentationML`: координаты — EMU, свойства знака — как в
//! `a:rPr`, размеры слайда — `p:sldSz`. Диаграмма — общая модель
//! [`ChartData`] (`doc_converter_render::chart`, ADR-0011): сериализуется в
//! блоб `DisplayList` и рисуется painter'ом и PDF-бэкендом.

use doc_converter_render::chart::ChartData;

/// EMU (English Metric Unit): 914400 EMU в дюйме, 9525 EMU в пикселе при 96 dpi.
pub type Emu = i64;

/// Пикселей раскладки на EMU: 96 dpi, та же плотность, что у DOCX/XLSX.
pub const EMU_PER_PX: f32 = 9525.0;

/// EMU → пиксели раскладки (96 dpi).
#[must_use]
pub fn emu_to_px(emu: Emu) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    {
        emu as f32 / EMU_PER_PX
    }
}

/// Пункты → пиксели раскладки (96 dpi).
#[must_use]
pub fn pt_to_px(pt: f32) -> f32 {
    pt * 4.0 / 3.0
}

/// Цвет RGB модели: `a:srgbClr val="RRGGBB"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// Цвет по каналам.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Цвет из шестнадцатеричной записи `RRGGBB`; битая запись — `None`.
    #[must_use]
    pub fn from_hex(value: &str) -> Option<Self> {
        let hex = value.strip_prefix('#').unwrap_or(value);
        if hex.len() != 6 {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }

    /// Цвет как `RRGGBB` — формат `srgbClr val`.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

/// Размер слайда из `p:sldSz`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlideSize {
    pub width: Emu,
    pub height: Emu,
}

impl SlideSize {
    /// Ширина слайда в пикселях раскладки.
    #[must_use]
    pub fn width_px(self) -> f32 {
        emu_to_px(self.width)
    }

    /// Высота слайда в пикселях раскладки.
    #[must_use]
    pub fn height_px(self) -> f32 {
        emu_to_px(self.height)
    }

    /// Ширина слайда в пунктах (1 pt = 12700 EMU).
    #[must_use]
    pub fn width_pt(self) -> f32 {
        self.width_px() * 0.75
    }

    /// Высота слайда в пунктах.
    #[must_use]
    pub fn height_pt(self) -> f32 {
        self.height_px() * 0.75
    }
}

impl Default for SlideSize {
    fn default() -> Self {
        // 16:9, 12192000 × 6858000 EMU — умолчание PowerPoint.
        Self {
            width: 12_192_000,
            height: 6_858_000,
        }
    }
}

/// Схема цветов темы (`a:clrScheme`).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeColors {
    /// `a:dk1` — тёмный основной (текст).
    pub dark1: Rgb,
    /// `a:lt1` — светлый основной (фон).
    pub light1: Rgb,
    /// `a:dk2` — тёмный дополнительный.
    pub dark2: Rgb,
    /// `a:lt2` — светлый дополнительный.
    pub light2: Rgb,
    /// `a:accent1`…`a:accent6`.
    pub accent: [Rgb; 6],
    /// `a:hlink`.
    pub hyperlink: Rgb,
    /// `a:folHlink`.
    pub followed_hyperlink: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            dark1: Rgb::new(0x00, 0x00, 0x00),
            light1: Rgb::new(0xFF, 0xFF, 0xFF),
            dark2: Rgb::new(0x44, 0x51, 0x6A),
            light2: Rgb::new(0xE7, 0xE6, 0xE6),
            accent: [
                Rgb::new(0x44, 0x72, 0xC4),
                Rgb::new(0xED, 0x7D, 0x31),
                Rgb::new(0xA5, 0xA5, 0xA5),
                Rgb::new(0xFF, 0xC0, 0x00),
                Rgb::new(0x5B, 0x9B, 0xD5),
                Rgb::new(0x70, 0xAD, 0x47),
            ],
            hyperlink: Rgb::new(0x05, 0x63, 0xC1),
            followed_hyperlink: Rgb::new(0x95, 0x4F, 0x72),
        }
    }
}

/// Именованный цвет схемы (`a:schemeClr val`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeColor {
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
}

impl SchemeColor {
    /// Цвет схемы по имени `a:schemeClr val`; незнакомое имя — `None`.
    #[must_use]
    pub fn from_val(value: &str) -> Option<Self> {
        Some(match value {
            "dk1" => Self::Dark1,
            "lt1" => Self::Light1,
            "dk2" => Self::Dark2,
            "lt2" => Self::Light2,
            "accent1" => Self::Accent1,
            "accent2" => Self::Accent2,
            "accent3" => Self::Accent3,
            "accent4" => Self::Accent4,
            "accent5" => Self::Accent5,
            "accent6" => Self::Accent6,
            "hlink" => Self::Hyperlink,
            "folHlink" => Self::FollowedHyperlink,
            _ => return None,
        })
    }
}

impl ThemeColors {
    /// Резолв именованного цвета схемы.
    #[must_use]
    pub fn resolve(&self, color: SchemeColor) -> Rgb {
        match color {
            SchemeColor::Dark1 => self.dark1,
            SchemeColor::Light1 => self.light1,
            SchemeColor::Dark2 => self.dark2,
            SchemeColor::Light2 => self.light2,
            SchemeColor::Accent1 => self.accent[0],
            SchemeColor::Accent2 => self.accent[1],
            SchemeColor::Accent3 => self.accent[2],
            SchemeColor::Accent4 => self.accent[3],
            SchemeColor::Accent5 => self.accent[4],
            SchemeColor::Accent6 => self.accent[5],
            SchemeColor::Hyperlink => self.hyperlink,
            SchemeColor::FollowedHyperlink => self.followed_hyperlink,
        }
    }
}

/// Тема презентации: шрифты и цвета.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Theme {
    /// `a:fontScheme/a:majorFont/a:latin@typeface` — гарнитура заголовков.
    pub major_font: String,
    /// `a:fontScheme/a:minorFont/a:latin@typeface` — гарнитура тела.
    pub minor_font: String,
    /// `a:clrScheme`.
    pub colors: ThemeColors,
}

/// Media-часть пакета: байты картинки и её MIME-тип.
#[derive(Debug, Clone)]
pub struct ImagePart {
    /// id картинки — номер записи в [`Presentation::images`]; тем же id
    /// помечены команды `Image` кадра.
    pub id: u32,
    /// Имя части в пакете (`ppt/media/image1.png`) — для диагностики.
    pub name: String,
    /// MIME-тип по расширению части.
    pub mime: String,
    /// Байты media-части.
    pub bytes: Vec<u8>,
}

/// Разобранная презентация.
#[derive(Debug, Clone, Default)]
pub struct Presentation {
    /// Размер слайда (`p:sldSz`).
    pub size: SlideSize,
    /// Слайды в порядке `p:sldIdLst`.
    pub slides: Vec<Slide>,
    /// Media-части, на которые ссылается хотя бы один `p:pic`.
    pub images: Vec<ImagePart>,
    /// Тема (`a:theme`): шрифты и цвета.
    pub theme: Theme,
    /// Восстановимые нарушения пакета — не мешают читать презентацию.
    pub warnings: Vec<String>,
}

impl Presentation {
    /// Слайд по индексу.
    #[must_use]
    pub fn slide(&self, index: usize) -> Option<&Slide> {
        self.slides.get(index)
    }

    /// Число слайдов.
    #[must_use]
    pub fn slide_count(&self) -> usize {
        self.slides.len()
    }

    /// Media-часть по id.
    #[must_use]
    pub fn image(&self, id: u32) -> Option<&ImagePart> {
        self.images.iter().find(|part| part.id == id)
    }
}

/// Слайд: имя, фон и шейпы.
#[derive(Debug, Clone, Default)]
pub struct Slide {
    /// Имя слайда (`p:cSld name`), по умолчанию — «Slide N».
    pub name: String,
    /// Фон слайда; `None` — белый.
    pub background: Option<Rgb>,
    /// Шейпы в порядке `p:spTree` (порядок задаёт наложение).
    pub shapes: Vec<Shape>,
}

/// Прямоугольник шейпа в EMU (`a:xfrm`/`p:xfrm`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame {
    pub x: Emu,
    pub y: Emu,
    pub w: Emu,
    pub h: Emu,
}

impl Frame {
    /// Прямоугольник в пикселях раскладки.
    #[must_use]
    pub fn px(self) -> doc_converter_render::geometry::Rect {
        doc_converter_render::geometry::Rect::new(
            emu_to_px(self.x),
            emu_to_px(self.y),
            emu_to_px(self.w),
            emu_to_px(self.h),
        )
    }
}

/// Линия контура шейпа (`a:ln`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub color: Option<Rgb>,
    /// Толщина в EMU; `None` — не задана (не рисуется).
    pub width: Option<Emu>,
}

/// Шейп слайда.
#[derive(Debug, Clone)]
pub struct Shape {
    /// Имя (`p:cNvPr name`) — для диагностики и поиска.
    pub name: String,
    /// Прямоугольник шейпа после наследования плейсхолдеров.
    pub frame: Frame,
    /// Заливка (`a:solidFill`/`a:schemeClr`); `None` — без заливки.
    pub fill: Option<Rgb>,
    /// Контур; `None` — без контура.
    pub line: Option<Line>,
    /// Что шейп несёт: текст, картинку, диаграмму или голую геометрию.
    pub kind: ShapeKind,
}

/// Содержимое шейпа.
#[derive(Debug, Clone)]
pub enum ShapeKind {
    /// Текстовая рамка (`p:txBody`).
    Text(TextBox),
    /// Картинка (`p:pic`): id media-части; `None` — часть не прочитана.
    Picture {
        /// id в [`Presentation::images`].
        image_id: Option<u32>,
    },
    /// Диаграмма (`p:graphicFrame` → `c:chart`), общая модель ADR-0011.
    Chart(ChartData),
    /// Геометрия без содержимого: `rect`, `roundRect`, `ellipse`.
    Geometry(GeometryPreset),
}

/// Геометрия шейпа (`a:prstGeom prst`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeometryPreset {
    #[default]
    Rect,
    RoundRect,
    Ellipse,
}

/// Текстовая рамка.
#[derive(Debug, Clone, Default)]
pub struct TextBox {
    /// Свойства и абзацы тела.
    pub body: TextBody,
}

/// Тело текстовой рамки (`p:txBody`).
#[derive(Debug, Clone, Default)]
pub struct TextBody {
    /// Отступы текста от рамки (`a:bodyPr`), EMU.
    pub insets: Insets,
    /// Вертикальный якорь (`a:bodyPr anchor`).
    pub anchor: Anchor,
    /// Перенос по словам (`a:bodyPr wrap`); `false` — `wrap="none"`.
    pub wrap: bool,
    /// Абзацы.
    pub paragraphs: Vec<Paragraph>,
}

/// Отступы текста внутри рамки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insets {
    pub left: Emu,
    pub top: Emu,
    pub right: Emu,
    pub bottom: Emu,
}

impl Default for Insets {
    fn default() -> Self {
        // Умолчания PowerPoint: 0.1 дюйма по бокам, 0.05 сверху и снизу.
        Self {
            left: 91_440,
            top: 45_720,
            right: 91_440,
            bottom: 45_720,
        }
    }
}

/// Вертикальный якорь текста.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Абзац текстовой рамки.
#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    /// Уровень списка (`a:pPr lvl`), 0..=8.
    pub level: u8,
    /// Горизонтальное выравнивание (`a:pPr algn`).
    pub align: Align,
    /// Маркер списка (`a:buChar`/`a:buAutoNum`/`a:buNone`).
    pub bullet: Option<Bullet>,
    /// Прогоны текста; свойства уже разрешены каскадом.
    pub runs: Vec<Run>,
}

/// Горизонтальное выравнивание абзаца.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Маркер списка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bullet {
    /// Символ (`a:buChar`), например `•`.
    Char(char),
    /// Автонумерация (`a:buAutoNum`), `start` — начальное значение.
    AutoNum { start: u32 },
}

/// Прогон текста.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub text: String,
    /// Свойства после каскада; `size` в пунктах.
    pub props: RunProps,
}

/// Свойства знака после каскада (ADR-0033 §3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunProps {
    /// Кегль в пунктах; `None` — ещё не резолвнут (парсер обязан выставить).
    pub size: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Цвет знака; `None` — `dark1` темы.
    pub color: Option<Rgb>,
    /// Гарнитура (`a:latin typeface`); `None` — minor/major темы.
    pub font: Option<String>,
}

impl RunProps {
    /// Наложить `over` поверх себя: заполненные поля перекрывают.
    #[must_use]
    pub fn merged(&self, over: Option<&Self>) -> Self {
        let Some(over) = over else {
            return self.clone();
        };
        Self {
            size: over.size.or(self.size),
            bold: self.bold || over.bold,
            italic: self.italic || over.italic,
            underline: self.underline || over.underline,
            color: over.color.or(self.color),
            font: over.font.clone().or_else(|| self.font.clone()),
        }
    }

    /// Свойства с кеглем по умолчанию, если он не задан.
    #[must_use]
    pub fn with_default_size(&self, default_pt: f32) -> Self {
        let mut props = self.clone();
        if props.size.is_none() {
            props.size = Some(default_pt);
        }
        props
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn emu_conversions_match_96dpi() {
        assert_eq!(emu_to_px(9525), 1.0);
        assert_eq!(emu_to_px(12_192_000), 1280.0);
        assert_eq!(pt_to_px(18.0), 24.0);
    }

    #[test]
    fn slide_size_default_is_16_by_9() {
        let size = SlideSize::default();
        assert_eq!(size.width_px(), 1280.0);
        assert_eq!(size.height_px(), 720.0);
        assert_eq!(size.width_pt(), 960.0);
        assert_eq!(size.height_pt(), 540.0);
    }

    #[test]
    fn hex_color_parses_rrggbb() {
        assert_eq!(Rgb::from_hex("4472C4"), Some(Rgb::new(0x44, 0x72, 0xC4)));
        assert_eq!(Rgb::from_hex("#FF0000"), Some(Rgb::new(255, 0, 0)));
        assert_eq!(Rgb::from_hex("12345"), None);
        assert_eq!(Rgb::from_hex("GG0000"), None);
        assert_eq!(Rgb::new(0x44, 0x72, 0xC4).to_u32(), 0x0044_72C4);
    }

    #[test]
    fn scheme_colors_resolve_through_theme() {
        let theme = Theme::default();
        assert_eq!(
            theme.colors.resolve(SchemeColor::Accent1),
            Rgb::new(0x44, 0x72, 0xC4)
        );
        assert_eq!(SchemeColor::from_val("accent2"), Some(SchemeColor::Accent2));
        assert_eq!(SchemeColor::from_val("bogus"), None);
    }

    #[test]
    fn run_props_merge_keeps_overrides() {
        let base = RunProps {
            size: Some(18.0),
            color: Some(Rgb::new(1, 2, 3)),
            font: Some("Calibri".into()),
            bold: true,
            italic: false,
            underline: false,
        };
        let over = RunProps {
            size: Some(24.0),
            bold: false,
            italic: true,
            ..RunProps::default()
        };
        let merged = base.merged(Some(&over));
        assert_eq!(merged.size, Some(24.0));
        assert!(merged.bold && merged.italic);
        assert_eq!(merged.color, Some(Rgb::new(1, 2, 3)));
        assert_eq!(merged.font.as_deref(), Some("Calibri"));
        assert_eq!(base.merged(None), base);
    }
}
