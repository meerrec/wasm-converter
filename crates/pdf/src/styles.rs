//! Стиль ячейки в терминах PDF: цвета, рамка, кегль, выравнивание.
//!
//! Цвет и выравнивание берутся из [`doc_converter_render::display_list`] — те
//! же типы, что у canvas-пути. Общих оттенков и осей у экрана и PDF не бывает
//! по построению, а не потому, что их синхронизируют (ADR-0007).

use doc_converter_xlsx::model::{Cell, Fill, FillPattern, HorizontalAlign, VerticalAlign};
use doc_converter_xlsx::paint::resolve_color;
use doc_converter_xlsx::{Border, Workbook};
use printpdf::Color as PdfColor;

pub use doc_converter_render::display_list::{Color, TextAlign};

/// Стиль линии рамки, приведённый к тому, что умеет PDF.
///
/// Толщина — в точках. Рисунок штриха (пунктир, двойная линия) пока не
/// переносится: `Op::SetLineDashPattern` — следующий срез.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderStyle {
    /// Толщина линии в точках; 0.0 — линии нет.
    pub width_pt: f32,
}

impl BorderStyle {
    /// Рисуется ли линия вообще.
    #[must_use]
    pub fn is_visible(self) -> bool {
        self.width_pt > 0.0
    }
}

/// Перевести стиль линии OOXML в толщину PDF.
///
/// Excel рисует тонкую линию в 1 px (0.75 pt), среднюю — 2 px (1.5 pt),
/// толстую — 3 px (2.25 pt). Штриховые варианты пока идут сплошной линией той
/// же толщины.
#[must_use]
pub fn border_style(style: doc_converter_xlsx::BorderStyle) -> BorderStyle {
    use doc_converter_xlsx::BorderStyle as Model;
    let width_pt = match style {
        Model::None => 0.0,
        Model::Hair => 0.5,
        Model::Thin
        | Model::Dashed
        | Model::Dotted
        | Model::DashDot
        | Model::DashDotDot
        | Model::SlantDashDot => 0.75,
        // Двойная линия — две тонкие с зазором; пока рисуется одной средней.
        Model::Medium
        | Model::MediumDashed
        | Model::MediumDashDot
        | Model::MediumDashDotDot
        | Model::Double => 1.5,
        Model::Thick => 2.25,
    };
    BorderStyle { width_pt }
}

/// Стиль ячейки, собранный из модели книги.
///
/// Четыре флага начертания — это ровно то, что записано в `styles.xml`:
/// здесь булев набор не состояние, а данные (как в `xlsx::model::Font`).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
pub struct CellStyle {
    /// Кегль в точках (в модели — пункты, масштаб печати применяет painter).
    pub font_size_pt: f32,
    /// Цвет текста.
    pub text_color: Color,
    /// Цвет заливки; `None` — заливки нет.
    pub fill: Option<Color>,
    /// Выравнивание по горизонтали.
    pub align: TextAlign,
    /// Выравнивание по вертикали: от него зависит базовая линия первой строки.
    pub vertical: VerticalAlign,
    /// Переносить текст по словам.
    pub wrap: bool,
    /// Полужирный: выбирает встраиваемое начертание, но не метрики —
    /// перенос и «#####» считаются по regular, как в canvas-пути.
    pub bold: bool,
    /// Курсив: см. [`CellStyle::bold`].
    pub italic: bool,
    /// Подчёркивание. TODO: PDF не рисует его сам, линию поставит отдельный срез.
    pub underline: bool,
    /// Рамка ячейки как она записана в формате.
    pub border: Border,
    /// Индекс формата числа (в модель передался из [`Cell`]).
    pub number_format: u32,
}

/// Собрать стиль ячейки из таблицы стилей книги.
///
/// Условное форматирование не применяется: эффективный стиль (`EffectiveStyle`)
/// потребует прогона правил и попадёт сюда отдельным срезом. Пока берётся
/// формат ячейки как он записан в файле.
#[must_use]
pub fn resolve(book: &Workbook, cell: &Cell) -> CellStyle {
    let styles = book.styles();
    let format = styles.resolve(cell.style);
    let font = styles.font(format.font).cloned().unwrap_or_default();
    CellStyle {
        font_size_pt: font.size,
        text_color: resolve_color(book.theme(), font.color).unwrap_or(Color::BLACK),
        fill: styles
            .fill(format.fill)
            .and_then(|fill| fill_color(book, fill)),
        align: text_align(format.alignment.horizontal, cell),
        vertical: format.alignment.vertical,
        wrap: format.alignment.wrap_text,
        bold: font.bold,
        italic: font.italic,
        underline: font.underline,
        border: styles.border(format.border).copied().unwrap_or_default(),
        number_format: format.num_fmt,
    }
}

/// Заливка ячейки; `None` — заливки нет.
fn fill_color(book: &Workbook, fill: &Fill) -> Option<Color> {
    match fill.pattern {
        FillPattern::None => None,
        FillPattern::Solid => resolve_color(book.theme(), fill.foreground),
        // Узоры Excel рисует растром; в PDF растра нет — показываем узор его
        // цветом, как canvas-путь. TODO: растр узора.
        _ => resolve_color(book.theme(), fill.foreground)
            .or_else(|| resolve_color(book.theme(), fill.background)),
    }
}

/// Выравнивание по горизонтали: явное из формата, иначе по типу значения.
///
/// Повторяет правило Excel и canvas-путь (`xlsx::paint::horizontal`).
fn text_align(align: HorizontalAlign, cell: &Cell) -> TextAlign {
    match align {
        HorizontalAlign::Center | HorizontalAlign::CenterContinuous => TextAlign::Center,
        HorizontalAlign::Right => TextAlign::Right,
        // По ширине PDF-путём пока не выравнивается — это левый край.
        HorizontalAlign::Left
        | HorizontalAlign::Fill
        | HorizontalAlign::Justify
        | HorizontalAlign::Distributed => TextAlign::Left,
        HorizontalAlign::General => match cell.value {
            doc_converter_xlsx::CellValue::Number(_) => TextAlign::Right,
            doc_converter_xlsx::CellValue::Bool(_) => TextAlign::Center,
            _ => TextAlign::Left,
        },
    }
}

/// Цвет `DisplayList` (`RRGGBBAA`) в цвет printpdf (нормированный RGB).
///
/// Альфа в PDF не переносится: прозрачность потребовала бы `/ExtGState`, это
/// отдельный срез. Непрозрачный цвет от неё не зависит.
#[must_use]
pub fn to_pdf_color(color: Color) -> PdfColor {
    let [r, g, b, _alpha] = color.0.to_be_bytes();
    PdfColor::Rgb(printpdf::Rgb::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        None,
    ))
}
