//! Движок раскладки DOCX: преобразование модели в страницы.
//!
//! Обходит блоки тела документа по порядку, раскладывает абзацы и таблицы, а
//! постраничную разбивку ведёт [`Paginator`](super::pagination::Paginator):
//! он же — единственный источник текущей позиции Y. Текст измеряется средствами
//! `doc-converter-render`, чтобы точки разрыва не разъезжались между canvas
//! и PDF (ADR-0005).

use doc_converter_core::NodeId;
use doc_converter_render::{
    font::{FontId, FontRegistry},
    text_measure,
};

use crate::layout::line_break::LineBreaker;
use crate::layout::pagination::Paginator;

use crate::model::raw::HalfPoint;
use crate::model::{
    BlockItem, Document, Margins, Orientation, PageSize, Paragraph, Section, SectionProperties,
};
use crate::{Columns, Twips};

/// Пикселей в типографском пункте: 96 DPI / 72 DPI = 4/3.
pub const PX_PER_POINT: f32 = 96.0 / 72.0;

/// Преобразовать twips в пиксели: 1 twip = 1/1440 дюйма, 1 пиксель = 1/96 дюйма.
/// Отношение: (1/1440) / (1/96) = 96/1440 = 1/15.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn twips_to_px(twips: Twips) -> f32 {
    twips.value() as f32 / 15.0
}

/// Преобразовать полупункты в пиксели: 1 pt = 1/72 дюйма, 1 px = 1/96 дюйма.
/// Отношение: (1/72) / (1/96) = 96/72 = 4/3.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn half_points_to_px(half_points: i32) -> f32 {
    (half_points as f32 / 2.0) * PX_PER_POINT
}

/// Прямоугольник в пикселях от левого верхнего угла страницы.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Левая граница.
    pub x: f32,
    /// Верхняя граница.
    pub y: f32,
    /// Ширина.
    pub width: f32,
    /// Высота.
    pub height: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Проверить, перекрывается ли с другим прямоугольником.
    #[must_use]
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    /// Сместить на смещение.
    #[must_use]
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

/// Выравнивание содержимого по вертикали.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VAlign {
    /// По верхнему краю.
    Top,
    /// По центру.
    Center,
    /// По нижнему краю.
    Bottom,
}

/// Элемент раскладки: что и где нарисовано на странице.
#[derive(Debug, Clone)]
pub enum LayoutItem {
    /// Абзац с текстом.
    Paragraph {
        /// Идентификатор узла.
        node_id: NodeId,
        /// Прямоугольник положения.
        rect: Rect,
        /// Текст для рендера.
        text: String,
        /// Высота строки.
        line_height: f32,
        /// Цвет текста.
        color: Option<u32>,
    },
    /// Таблица.
    Table {
        /// Идентификатор узла.
        node_id: NodeId,
        /// Прямоугольник положения.
        rect: Rect,
        /// Раскладка ячеек.
        cells: Vec<TableCellLayout>,
    },
    /// Разрыв страницы.
    PageBreak,
    /// Разрыв колонки.
    ColumnBreak,
}

/// Раскладка одной ячейки таблицы.
#[derive(Debug, Clone)]
pub struct TableCellLayout {
    /// Прямоугольник положения.
    pub rect: Rect,
    /// Содержимое ячейки.
    pub content: Vec<LayoutItem>,
}

/// Страница документа после раскладки.
#[derive(Debug, Clone)]
pub struct Page {
    /// Номер страницы (с единицы).
    pub number: u32,
    /// Ширина страницы в пикселях.
    pub width: f32,
    /// Высота страницы в пикселях.
    pub height: f32,
    /// Элементы на странице в порядке рендеринга (сверху вниз, слева направо).
    pub items: Vec<LayoutItem>,
    /// Поля страницы.
    pub margins: MarginsLayout,
}

/// Раскладка полей страницы в пикселях.
#[derive(Debug, Clone, Copy)]
pub struct MarginsLayout {
    /// Верхнее поле.
    pub top: f32,
    /// Правое поле.
    pub right: f32,
    /// Нижнее поле.
    pub bottom: f32,
    /// Левое поле.
    pub left: f32,
    /// Поле до верхнего колонтитула.
    pub header: f32,
    /// Поле до нижнего колонтитула.
    pub footer: f32,
    /// Добавка на переплёт.
    pub gutter: f32,
}

impl MarginsLayout {
    #[must_use]
    pub fn from_margins(margins: &Margins) -> Self {
        Self {
            top: twips_to_px(margins.top),
            right: twips_to_px(margins.right),
            bottom: twips_to_px(margins.bottom),
            left: twips_to_px(margins.left),
            header: margins.header.map_or(0.0, twips_to_px),
            footer: margins.footer.map_or(0.0, twips_to_px),
            gutter: margins.gutter.map_or(0.0, twips_to_px),
        }
    }
}

/// Общая раскладка документа.
#[derive(Debug, Clone)]
pub struct PageLayout {
    /// Страницы документа.
    pub pages: Vec<Page>,
    /// Общая высота содержимого (без учёта полей и колонтитулов).
    pub total_height: f32,
}

/// Состояние раскладки: колонка и текущая позиция в ней.
#[derive(Debug, Clone)]
pub struct LayoutState {
    /// Текущая позиция X в пикселях от левого края страницы.
    pub x: f32,
    /// Текущая позиция Y в пикселях от верхнего края страницы.
    pub y: f32,
    /// Текущая колонка (для многоколоночной раскладки).
    pub column_index: usize,
    /// Ширина текущей колонки.
    pub column_width: f32,
    /// Отступ слева для текущей колонки (от левого края страницы).
    pub column_x: f32,
    /// Верхняя граница полосы набора (верхнее поле страницы).
    pub content_top: f32,
    /// Нижняя граница полосы набора (высота страницы минус нижнее поле).
    pub content_bottom: f32,
}

/// Опции раскладки.
#[derive(Debug, Clone)]
pub struct LayoutOptions {
    /// Верхняя граница ширины полосы набора в физических пикселях canvas.
    ///
    /// Именно ограничение сверху, а не замена ширины: ширину задаёт страница,
    /// canvas лишь режет её, когда окно уже страницы.
    pub canvas_width: Option<f32>,
    /// Масштаб рендеринга. Параметр рисования, а не верстки.
    pub scale: f32,
    /// DPR (device pixel ratio). Параметр рисования, а не верстки.
    pub dpr: f32,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            canvas_width: None,
            scale: 1.0,
            dpr: 1.0,
        }
    }
}

impl LayoutOptions {
    /// Эффективная ширина полосы набора в пикселях раскладки.
    ///
    /// `canvas_width` задан в физических пикселях, поэтому переводится в пиксели
    /// раскладки делением на `scale * dpr`; без него ширина равна ширине страницы.
    /// Некорректные (нулевые или нечисловые) параметры рисования не сужают полосу
    /// набора до нуля — иначе текст перестал бы раскладываться вовсе.
    #[must_use]
    pub fn effective_width(&self, page_width: f32) -> f32 {
        let Some(canvas_width) = self.canvas_width else {
            return page_width;
        };
        let factor = self.scale * self.dpr;
        if !factor.is_finite() || factor <= 0.0 {
            return page_width;
        }
        (canvas_width / factor).min(page_width)
    }
}

/// Контекст рендеринга для измерения текста.
///
/// Обёртка над `.FontRegistry` из render, чтобы не таскать его через все функции.
pub struct RenderContext<'a> {
    pub fonts: &'a mut doc_converter_render::font::FontRegistry,
    pub default_font_id: doc_converter_render::font::FontId,
}

impl RenderContext<'_> {
    /// Измерить текст.
    #[must_use]
    pub fn measure_text(
        &mut self,
        text: &str,
        font_id: doc_converter_render::font::FontId,
        size_px: f32,
    ) -> f32 {
        text_measure::measure_text(self.fonts, font_id, size_px, text)
    }

    /// Разбить текст на строки.
    #[must_use]
    pub fn break_lines(
        &mut self,
        text: &str,
        font_id: doc_converter_render::font::FontId,
        size_px: f32,
        max_width: f32,
    ) -> Vec<std::ops::Range<usize>> {
        text_measure::break_lines(self.fonts, font_id, size_px, text, max_width)
    }
}

/// Разложить документ по страницам.
///
/// # Arguments
/// * `document` - DOCX документ
/// * `options` - опции раскладки
/// * `fonts` - реестр шрифтов для измерения текста
///
/// # Errors
/// Возвращает ошибку если не удалось измерить текст или распределить содержимое.
pub fn layout_document(
    document: &Document,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
) -> Result<PageLayout, LayoutError> {
    let mut pages: Vec<Page> = Vec::new();
    let mut placed_floats: Vec<crate::layout::float::FloatElement> = Vec::new();

    let first_section = section_at(document, 0);
    let mut paginator = Paginator::new(&first_section);
    let mut state = LayoutState::for_page(paginator.current_page(), &first_section);
    let default_font_id = FontId::default();

    // Process all blocks
    for block in &document.body.items {
        match block {
            BlockItem::Paragraph(paragraph) => {
                // Курсор страницы — источник истины: состояние возвращается к нему перед
                // каждым абзацем, иначе элементы получают координаты от прошлого блока.
                state.x = state.column_x;
                state.y = paginator.current_y();

                // Layout paragraph
                let mut line_breaker = LineBreaker::new(fonts, default_font_id);
                let (content_height, layout_items) =
                    layout_paragraph(paragraph, &mut state, options, &mut line_breaker);

                // Check if we need a new page
                if !paginator.fits(content_height) {
                    let next_section = next_page_section(document, pages.len());
                    pages.push(paginator.push_page_with_section(&next_section));
                    placed_floats.clear(); // Reset floats for new page
                    state = LayoutState::for_page(paginator.current_page(), &next_section);
                }

                paginator.current_page_mut().items.extend(layout_items);
                paginator.advance(content_height);
            }
            BlockItem::Table(table) => {
                // Layout table
                let table_width = state.column_width;
                let table_height = layout_table(table, &mut state, options, fonts, table_width);

                // Check if table fits on current page
                if !paginator.fits(table_height) {
                    let next_section = next_page_section(document, pages.len());
                    pages.push(paginator.push_page_with_section(&next_section));
                    placed_floats.clear();
                    state = LayoutState::for_page(paginator.current_page(), &next_section);
                }

                // TODO: Add table to current page
                paginator.advance(table_height);
            }
            BlockItem::SectPr(_sect_pr) => {
                // Handle section break
                // Force new page if needed
                if crate::layout::pagination::block_needs_page_break(block, None) {
                    let next_section = next_page_section(document, pages.len());
                    pages.push(paginator.push_page_with_section(&next_section));
                    placed_floats.clear();
                    state = LayoutState::for_page(paginator.current_page(), &next_section);
                }
            }
            BlockItem::Unknown { .. } => {
                // Skip unknown
            }
        }
    }

    // Текущая страница отдаётся как есть, даже пустая: правка этой части меняет
    // число страниц, а здесь движок переезжает на пагинатор без смены поведения.
    pages.push(paginator.current_page().clone());

    let total_height = pages.iter().map(|page| page.height).sum();

    Ok(PageLayout {
        pages,
        total_height,
    })
}

/// Секция номер `index` или секция по умолчанию.
///
/// Секций в теле может не быть вовсе — у документа без `w:sectPr` или собранного
/// вручную: тогда раскладка берёт A4 с полями в дюйм.
fn section_at(document: &Document, index: usize) -> Section {
    document
        .body
        .sections
        .get(index)
        .cloned()
        .unwrap_or_else(default_section)
}

/// Секция для страницы, начинающейся после `finished_pages` завершённых.
///
/// Номер новой страницы — `finished_pages + 1`, и прежний движок брал секцию
/// именно по нему. Порядок секций в теле документа — отдельная правка; этот
/// слайс переезжает на пагинатор, не меняя раскладку.
fn next_page_section(document: &Document, finished_pages: usize) -> Section {
    section_at(document, finished_pages + 1)
}

/// Разложить абзац.
///
/// Использует `LineBreaker` для разбивки текста на строки с учётом ширины страницы.
fn layout_paragraph(
    paragraph: &Paragraph,
    state: &mut LayoutState,
    options: &LayoutOptions,
    line_breaker: &mut LineBreaker,
) -> (f32, Vec<LayoutItem>) {
    // Collect text from all runs
    let text: String = paragraph
        .runs
        .iter()
        .filter_map(|inline| {
            if let crate::model::Inline::Run(run) = inline {
                Some(
                    run.content
                        .iter()
                        .filter_map(|content| {
                            if let crate::model::RunContent::Text(text) = content {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .concat(),
                )
            } else {
                None
            }
        })
        .collect::<String>();

    // Get font size from paragraph properties or use default
    // TODO: extract actual font size from run properties
    let font_size_half_points = HalfPoint::new(24); // 12pt default
    let default_font_id = FontId::default();
    let line_height = line_breaker.line_height_with_spacing(font_size_half_points, None);

    // Calculate available width
    let available_width = options.effective_width(state.column_width);

    // Break text into lines
    let line_ranges = line_breaker.break_lines(
        &text,
        default_font_id,
        font_size_half_points,
        available_width,
    );

    // Measure each line and calculate total height
    let mut total_height = 0.0;
    let mut items = Vec::new();
    let current_x = state.x;
    let mut current_y = state.y;

    for line_range in line_ranges {
        let line_text = &text[line_range];
        let line_width =
            line_breaker.measure_text(line_text, default_font_id, font_size_half_points);

        total_height += line_height;

        items.push(LayoutItem::Paragraph {
            node_id: paragraph.id,
            rect: Rect::new(current_x, current_y, line_width, line_height),
            text: line_text.to_string(),
            line_height,
            color: None,
        });

        current_y += line_height;
    }

    (total_height, items)
}

/// Разложить таблицу.
fn layout_table(
    table: &crate::model::Table,
    _state: &mut LayoutState,
    _options: &LayoutOptions,
    _fonts: &mut FontRegistry,
    available_width: f32,
) -> f32 {
    use crate::layout::tables::{compute_column_widths, compute_row_heights};

    // Compute column widths
    let _col_widths = compute_column_widths(table, available_width);

    // Compute row heights
    let row_heights = compute_row_heights(&table.rows);

    // Total height is sum of all row heights
    row_heights.iter().sum()
}

/// Ошибка раскладки.
#[derive(Debug, Clone)]
pub enum LayoutError {
    /// Не удалось измерить текст.
    TextMeasurementFailed,
    /// Переполнение страницы.
    PageOverflow,
    /// Неизвестная секция.
    UnknownSection,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::TextMeasurementFailed => write!(f, "Failed to measure text"),
            LayoutError::PageOverflow => write!(f, "Page overflow"),
            LayoutError::UnknownSection => write!(f, "Unknown section"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Создать секцию по умолчанию.
fn default_section() -> Section {
    Section {
        id: NodeId::new(0),
        properties: SectionProperties::default(),
        header_default: None,
        header_first: None,
        header_even: None,
        footer_default: None,
        footer_first: None,
        footer_even: None,
        title_pg: false,
        page_size: PageSize {
            width: Twips::new(11906),  // A4 width in twips
            height: Twips::new(16838), // A4 height in twips
        },
        orientation: Orientation::Portrait,
        margins: Margins {
            top: Twips::new(1440), // 1 inch
            right: Twips::new(1440),
            bottom: Twips::new(1440),
            left: Twips::new(1440),
            header: None,
            footer: None,
            gutter: None,
        },
        columns: Columns {
            count: 1,
            space: Twips::new(0),
            equal_width: true,
            separator: false,
            defs: vec![],
        },
    }
}

impl Default for SectionProperties {
    fn default() -> Self {
        Self {
            page_size: PageSize {
                width: Twips::new(11906),
                height: Twips::new(16838),
            },
            orientation: Orientation::Portrait,
            margins: Margins {
                top: Twips::new(1440),
                right: Twips::new(1440),
                bottom: Twips::new(1440),
                left: Twips::new(1440),
                header: None,
                footer: None,
                gutter: None,
            },
            columns: Columns {
                count: 1,
                space: Twips::new(0),
                equal_width: true,
                separator: false,
                defs: vec![],
            },
            title_pg: false,
            header_default: None,
            header_first: None,
            header_even: None,
            footer_default: None,
            footer_first: None,
            footer_even: None,
            section_type: None,
            unknown: vec![],
        }
    }
}

impl LayoutState {
    /// Состояние на начало страницы: полоса набора внутри её полей.
    ///
    /// Секция пока используется только для проверки числа колонок; поля и размеры
    /// берутся из уже собранной страницы, чтобы состояние и страница не разъезжались.
    #[must_use]
    pub fn for_page(page: &Page, section: &Section) -> Self {
        let column_width = page.width - page.margins.left - page.margins.right;
        if section.columns.count > 1 {
            // TODO (Спринт 10): многоколоночная раскладка — полосу набора предстоит
            // делить между колонками; все фикстуры одноколоночные.
        }
        Self {
            x: page.margins.left,
            y: page.margins.top,
            column_index: 0,
            column_width,
            column_x: page.margins.left,
            content_top: page.margins.top,
            content_bottom: page.height - page.margins.bottom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Body, Inline, Metadata, NumberingTable, Relationships, Run, RunContent, Settings,
        StyleTable,
    };

    /// Документ из одного абзаца с текстом.
    ///
    /// `sections` пуст намеренно: раскладка обязана взять секцию по умолчанию —
    /// так же, как для документа без заключительного `w:sectPr`.
    fn document_with_text(text: &str) -> Document {
        let paragraph = Paragraph {
            id: NodeId::new(1),
            runs: vec![Inline::Run(Run {
                id: NodeId::new(2),
                content: vec![RunContent::Text(text.to_owned())],
                ..Run::default()
            })],
            ..Paragraph::default()
        };
        Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::new(3),
                items: vec![BlockItem::Paragraph(paragraph)],
                sections: Vec::new(),
            },
            styles: StyleTable::default(),
            numbering: NumberingTable::default(),
            settings: Settings::default(),
            metadata: Metadata::default(),
            rels: Relationships::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            headers: std::collections::BTreeMap::new(),
            footers: std::collections::BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Блокер S1.1: абзац с текстом обязан попасть на страницу внутри полей.
    // Координаты копируются из полей страницы без арифметики — сравнение точное.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_paragraph_lands_inside_the_margins() {
        let text = "Hello, DOCX layout";
        let document = document_with_text(text);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 1, "одностраничный документ");
        let page = &layout.pages[0];
        let paragraphs: Vec<(&Rect, &String)> = page
            .items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { rect, text, .. } => Some((rect, text)),
                _ => None,
            })
            .collect();
        assert_eq!(
            paragraphs.len(),
            1,
            "ровно один абзац, получено {paragraphs:?}"
        );

        let (rect, laid_out) = paragraphs[0];
        assert_eq!(rect.x, page.margins.left, "абзац начинается от левого поля");
        assert_eq!(
            rect.y, page.margins.top,
            "абзац начинается от верхнего поля"
        );
        assert!(rect.width > 0.0, "ширина строки: {}", rect.width);
        assert_eq!(laid_out.as_str(), text);
    }
}
