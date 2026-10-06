//! Водяной знак и колонтитулы: коды Excel, геометрия полос и размещение.
//!
//! Строки `oddHeader`/`oddFooter` книги приходят сырыми (поля
//! `doc_converter_xlsx::PrintSettings::odd_header`/`odd_footer`) и разбираются
//! на три секции: [`parse_header_footer`] подставляет номер страницы и прочие
//! поля. Геометрия — в точках (PDF user space, начало координат в левом нижнем
//! углу страницы): колонтитул живёт в поле страницы между её краем и областью
//! содержимого и потому не может наехать на ячейки; водяной знак центрируется
//! по странице.
//!
//! Модуль намеренно не зависит от `crate::options`/`crate::layout`: его
//! подключают `#[path]`-ом в тестах до регистрации в `lib.rs`, а встраивание в
//! painter — отдельный слой. Цвет, гарнитуру и порядок отрисовки выбирает
//! painter; здесь — только разбор, координаты и альфа.

use serde::{Deserialize, Serialize};

/// Точек на миллиметр — тот же коэффициент, что у `layout::PT_PER_MM`.
const PT_PER_MM: f32 = 72.0 / 25.4;

/// Доля кегля от базовой линии до верхнего края коробки строки.
///
/// Метрик шрифта у модуля нет: коробка строки считается квадратом кегля, а
/// базовая линия — на 0.8 кегля выше его низа (типичная доля ascender'а).
const ASCENT_RATIO: f32 = 0.8;

/// Половина высоты заглавной буквы в долях кегля — для центрирования строки
/// водяного знака по вертикали.
const HALF_CAP_RATIO: f32 = 0.35;

/// Средняя ширина глифа в долях кегля — приближение для центрирования
/// водяного знака, пока нет метрик шрифта.
const AVG_GLYPH_RATIO: f32 = 0.6;

/// Настройки колонтитулов и водяного знака.
///
/// Имя задано планом (`overlay::OverlayConfig`), поэтому повтор модуля в имени
/// типа допустим осознанно.
#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    /// Верхний колонтитул в кодах Excel; пустая строка — не рисовать.
    pub header: String,
    /// Нижний колонтитул в кодах Excel.
    pub footer: String,
    /// Водяной знак; `None` — не рисовать.
    pub watermark: Option<Watermark>,
}

impl OverlayConfig {
    /// Нечего рисовать: обе строки пусты и водяной знак пуст.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.header.is_empty()
            && self.footer.is_empty()
            && self
                .watermark
                .as_ref()
                .is_none_or(|watermark| watermark.text.trim().is_empty())
    }
}

/// Водяной знак: текст под углом, по центру страницы.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watermark {
    /// Текст; пустая строка (или одни пробелы) — знака нет.
    pub text: String,
    /// Поворот против часовой стрелки в системе PDF, градусы.
    pub angle_deg: f32,
    /// Непрозрачность в `[0, 1]`; меньше 1 — только через `/GS` (`CA`/`ca`).
    pub opacity: f32,
    /// Кегль, pt.
    pub font_size_pt: f32,
    /// Слой относительно содержимого листа.
    pub layer: WatermarkLayer,
}

impl Default for Watermark {
    fn default() -> Self {
        Self {
            text: String::new(),
            // Классический диагональный водяной знак: снизу слева вверх направо.
            angle_deg: 45.0,
            opacity: 0.2,
            font_size_pt: 48.0,
            layer: WatermarkLayer::Under,
        }
    }
}

/// Порядок водяного знака относительно содержимого листа.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkLayer {
    /// Под содержимым: ячейки, сетка и картинки рисуются поверх текста.
    #[default]
    Under,
    /// Поверх содержимого.
    Over,
}

/// Поля страницы в точках.
#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayMargins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl OverlayMargins {
    /// Поля из миллиметров — как `PageConfig::margins`.
    #[must_use]
    pub fn from_mm(top_mm: f32, right_mm: f32, bottom_mm: f32, left_mm: f32) -> Self {
        Self {
            top: top_mm * PT_PER_MM,
            right: right_mm * PT_PER_MM,
            bottom: bottom_mm * PT_PER_MM,
            left: left_mm * PT_PER_MM,
        }
    }
}

/// Прямоугольник в точках: `y` — вверх от нижнего края страницы.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Полоса верхнего колонтитула — поле страницы над областью содержимого.
#[must_use]
pub fn header_band(page_width_pt: f32, page_height_pt: f32, margins: OverlayMargins) -> Band {
    let height = margins.top.max(0.0);
    Band {
        x: margins.left,
        y: (page_height_pt - height).max(0.0),
        width: (page_width_pt - margins.left - margins.right).max(0.0),
        height,
    }
}

/// Полоса нижнего колонтитула — поле страницы под областью содержимого.
#[must_use]
pub fn footer_band(page_width_pt: f32, margins: OverlayMargins) -> Band {
    Band {
        x: margins.left,
        y: 0.0,
        width: (page_width_pt - margins.left - margins.right).max(0.0),
        height: margins.bottom.max(0.0),
    }
}

/// Горизонтальное выравнивание секции колонтитула.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// `&L` — от левого края области содержимого.
    Left,
    /// `&C` — по центру области содержимого.
    Center,
    /// `&R` — по правому краю области содержимого.
    Right,
}

/// X начала текста шириной `text_width_pt` в полосе.
///
/// Текст, который шире области содержимого, к левому краю не сдвигается:
/// переполнение уходит вправо, как в Excel.
#[must_use]
pub fn anchor_x(band: Band, align: Align, text_width_pt: f32) -> f32 {
    let free = band.width - text_width_pt.max(0.0);
    match align {
        Align::Left => band.x,
        Align::Center => band.x + (free / 2.0).max(0.0),
        Align::Right => band.x + free.max(0.0),
    }
}

/// Базовая линия строки кегля `font_size_pt` в полосе шапки.
///
/// Коробка строки центрируется по полосе; кегль, переросший поле, прижимается
/// к внешнему (верхнему) краю страницы, а не к содержимому. Пока
/// `font_size_pt <= band.height`, строка содержимого не касается.
#[must_use]
pub fn header_baseline(band: Band, font_size_pt: f32) -> f32 {
    let size = font_size_pt.max(0.0);
    let bottom = if size <= band.height {
        band.y + (band.height - size) / 2.0
    } else {
        band.y + band.height - size
    };
    bottom + size * ASCENT_RATIO
}

/// Базовая линия строки в полосе подвала; см. [`header_baseline`].
#[must_use]
pub fn footer_baseline(band: Band, font_size_pt: f32) -> f32 {
    let size = font_size_pt.max(0.0);
    let bottom = if size <= band.height {
        band.y + (band.height - size) / 2.0
    } else {
        band.y
    };
    bottom + size * ASCENT_RATIO
}

/// Линия-разделитель между колонтитулом и областью содержимого.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
}

/// Разделитель под верхним колонтитулом — по границе области содержимого.
#[must_use]
pub fn header_rule(band: Band) -> Rule {
    Rule {
        x0: band.x,
        x1: band.x + band.width,
        y: band.y,
    }
}

/// Разделитель над нижним колонтитулом — по границе области содержимого.
#[must_use]
pub fn footer_rule(band: Band) -> Rule {
    Rule {
        x0: band.x,
        x1: band.x + band.width,
        y: band.y + band.height,
    }
}

/// Значения подстановок колонтитула для одной страницы.
#[derive(Debug, Clone, Copy)]
pub struct PageContext<'a> {
    /// Номер страницы, с 1 (код `&P`).
    pub page_no: usize,
    /// Всего страниц (код `&N`).
    pub page_count: usize,
    /// Дата для `&D`; `None` — код разворачивается в пустую строку.
    pub date: Option<&'a str>,
    /// Имя файла для `&F`; `None` — пустая строка.
    pub file_name: Option<&'a str>,
}

impl PageContext<'_> {
    /// Контекст без даты и имени файла: коды `&D`/`&F` дадут пусто.
    #[must_use]
    pub fn new(page_no: usize, page_count: usize) -> Self {
        Self {
            page_no,
            page_count,
            date: None,
            file_name: None,
        }
    }
}

/// Колонтитул, разобранный на три секции Excel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderFooterParts {
    /// `&L` — левая секция.
    pub left: String,
    /// `&C` — центральная; сюда же попадает текст до первого кода.
    pub center: String,
    /// `&R` — правая секция.
    pub right: String,
}

impl HeaderFooterParts {
    /// Все секции пусты.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.center.is_empty() && self.right.is_empty()
    }
}

/// Секция, в которую пишется текущий текст.
#[derive(Debug, Clone, Copy)]
enum Section {
    Left,
    Center,
    Right,
}

fn section_mut(parts: &mut HeaderFooterParts, section: Section) -> &mut String {
    match section {
        Section::Left => &mut parts.left,
        Section::Center => &mut parts.center,
        Section::Right => &mut parts.right,
    }
}

/// Разобрать строку колонтитула Excel и подставить значения страницы.
///
/// Поддержаны `&L`/`&C`/`&R` (секции), `&P` (номер страницы), `&N` (всего
/// страниц), `&D` (дата), `&F` (имя файла) и `&&` (сам амперсанд). Текст до
/// первого кода идёт в центральную секцию — так же, как в Excel и Apache POI.
/// Неизвестный код сохраняется дословно: символ `&` и следующий за ним (коды
/// шрифта `&"…"` и `&nn` отдельно не разбираются). Одиночный `&` в конце
/// строки — тоже литерал.
#[must_use]
pub fn parse_header_footer(raw: &str, ctx: PageContext<'_>) -> HeaderFooterParts {
    let mut parts = HeaderFooterParts::default();
    let mut section = Section::Center;
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            section_mut(&mut parts, section).push(ch);
            continue;
        }
        let target = section_mut(&mut parts, section);
        match chars.next() {
            None | Some('&') => target.push('&'),
            Some('L') => section = Section::Left,
            Some('C') => section = Section::Center,
            Some('R') => section = Section::Right,
            Some('P') => target.push_str(&ctx.page_no.to_string()),
            Some('N') => target.push_str(&ctx.page_count.to_string()),
            Some('D') => target.push_str(ctx.date.unwrap_or("")),
            Some('F') => target.push_str(ctx.file_name.unwrap_or("")),
            Some(other) => {
                target.push('&');
                target.push(other);
            }
        }
    }
    parts
}

/// Непрозрачность в `[0, 1]`: выход за диапазон зажимается, `NaN` — 1.0.
#[must_use]
pub fn clamp_opacity(opacity: f32) -> f32 {
    if opacity.is_nan() {
        1.0
    } else {
        opacity.clamp(0.0, 1.0)
    }
}

/// Нужен ли extended graphics state: при `opacity < 1` сплошная заливка
/// недопустима, painter обязан выпустить `/GS` с `CA`/`ca` (`DoD` F2).
#[must_use]
pub fn needs_extgstate(opacity: f32) -> bool {
    clamp_opacity(opacity) < 1.0
}

/// Оценка ширины строки в точках: средний глиф — 0.6 кегля.
///
/// Метрик шрифта у модуля нет; неточность ширины сдвигает центр водяного
/// знака слабее, чем ошибка в выборе позиции, а painter может уточнить
/// смещение своими метриками.
#[must_use]
pub fn estimate_text_width(text: &str, font_size_pt: f32) -> f32 {
    let count = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
    f32::from(count) * font_size_pt.max(0.0) * AVG_GLYPH_RATIO
}

/// Матрица `Tm` (`a b c d e f`) для текста, повёрнутого на `angle_deg` против
/// часовой стрелки и начинающегося в точке `(tx, ty)`.
///
/// Годится и для `printpdf::TextMatrix::Raw`.
#[must_use]
pub fn rotation_matrix(angle_deg: f32, tx: f32, ty: f32) -> [f32; 6] {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    [cos, sin, -sin, cos, tx, ty]
}

/// Готовое к отрисовке размещение водяного знака.
#[derive(Debug, Clone, PartialEq)]
pub struct WatermarkDraw {
    pub text: String,
    /// Матрица `Tm`; painter подаёт её как `printpdf::TextMatrix::Raw`.
    pub matrix: [f32; 6],
    pub font_size_pt: f32,
    /// Альфа в `[0, 1]`: меньше 1 — рисовать через `ExtendedGraphicsState`.
    pub opacity: f32,
    pub layer: WatermarkLayer,
}

/// Разместить водяной знак по центру страницы; `None` — текст пуст.
///
/// Центр повёрнутой коробки текста совпадает с центром страницы, поэтому
/// матрица получает сдвиг на повёрнутый вектор половин ширины и высоты
/// заглавной буквы.
#[must_use]
pub fn watermark_draw(
    page_width_pt: f32,
    page_height_pt: f32,
    watermark: &Watermark,
) -> Option<WatermarkDraw> {
    if watermark.text.trim().is_empty() {
        return None;
    }
    let font_size_pt = watermark.font_size_pt.max(0.0);
    let width = estimate_text_width(&watermark.text, font_size_pt);
    let (sin, cos) = watermark.angle_deg.to_radians().sin_cos();
    let half = width / 2.0;
    let half_cap = font_size_pt * HALF_CAP_RATIO;
    let center_x = page_width_pt / 2.0;
    let center_y = page_height_pt / 2.0;
    let tx = half_cap.mul_add(sin, (-half).mul_add(cos, center_x));
    let ty = (-half_cap).mul_add(cos, (-half).mul_add(sin, center_y));
    Some(WatermarkDraw {
        text: watermark.text.clone(),
        matrix: [cos, sin, -sin, cos, tx, ty],
        font_size_pt,
        opacity: clamp_opacity(watermark.opacity),
        layer: watermark.layer,
    })
}
