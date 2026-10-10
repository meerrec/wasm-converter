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

use crate::layout::cascade::StyleCache;
use crate::layout::float::FloatElement;
use crate::layout::line_break::LineBreaker;
use crate::layout::pagination::Paginator;
use crate::layout::paragraph::{layout_paragraph, Line, ParagraphLayout};

use crate::model::raw::HalfPoint;
use crate::model::{
    BlockItem, BreakKind, Document, Inline, Margins, Orientation, PageSize, Paragraph, RunContent,
    Section, SectionProperties, Table,
};
use crate::{Columns, Twips};

/// Пикселей в типографском пункте: 96 DPI / 72 DPI = 4/3.
pub const PX_PER_POINT: f32 = 96.0 / 72.0;

/// Кегль по умолчанию, пока каскад не отдаёт размер знака: 12 pt.
///
/// Тот же, что берёт [`layout_paragraph`] для строк: иначе пустая строка
/// разошлась бы по высоте со строкой текста.
const DEFAULT_HALF_POINTS: i32 = 24;

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
    let mut placed_floats: Vec<crate::layout::float::FloatElement> = Vec::new();

    let first_section = section_at(document, 0);
    let mut paginator = Paginator::new(&first_section);
    let mut state = LayoutState::for_page(paginator.current_page(), &first_section);
    let mut cache = StyleCache::new();
    let default_font_id = FontId::default();
    // Секция, в которой идёт раскладка: `sections[i]` описывает секцию, которую
    // закрывает i-й по порядку конец секции (`collect_sections` в `document.rs`).
    let mut section_index = 0usize;

    for block in &document.body.items {
        match block {
            BlockItem::Paragraph(paragraph) => {
                // Курсор страницы — источник истины: состояние возвращается к нему перед
                // каждым абзацем, иначе элементы получают координаты от прошлого блока.
                state.x = state.column_x;
                state.y = paginator.current_y();

                // Layout paragraph
                let mut line_breaker = LineBreaker::new(fonts, default_font_id);
                let layout = layout_paragraph(
                    document,
                    paragraph,
                    &state,
                    options,
                    &mut cache,
                    &mut line_breaker,
                );

                // Пустой `w:p` всё равно занимает строку высотой шрифта: ноль
                // схлопнул бы абзацы и набрал страницу плотнее, чем в Word.
                let empty_line = Line {
                    text: String::new(),
                    x: state.x,
                    width: 0.0,
                    height: line_breaker
                        .line_height_with_spacing(HalfPoint::new(DEFAULT_HALF_POINTS), None),
                    color: None,
                };
                let lines: Vec<&Line> = if layout.lines.is_empty() {
                    vec![&empty_line]
                } else {
                    layout.lines.iter().collect()
                };

                place_lines(
                    &mut paginator,
                    paragraph,
                    &lines,
                    &layout,
                    &mut placed_floats,
                    &mut state,
                );

                // Абзац со встроенным `w:sectPr` закрывает секцию: он сам ещё
                // принадлежит ей, а следующая начинается после него.
                if paragraph.section_break.is_some() {
                    section_index += 1;
                    enter_section(
                        document,
                        section_index,
                        &mut paginator,
                        &mut placed_floats,
                        &mut state,
                    );
                }
            }
            BlockItem::Table(table) => {
                place_table(table, fonts, &mut paginator, &mut placed_floats, &mut state);
            }
            BlockItem::SectPr(_sect_pr) => {
                // Блочный `w:sectPr` закрывает последнюю секцию и описывает её
                // свойства, а не разрывает документ: новой секции за ним нет,
                // поэтому лишней пустой страницы не появляется.
                section_index += 1;
                enter_section(
                    document,
                    section_index,
                    &mut paginator,
                    &mut placed_floats,
                    &mut state,
                );
            }
            BlockItem::Unknown { .. } => {
                // Skip unknown
            }
        }
    }

    let pages = paginator.finish();

    let total_height = pages.iter().map(|page| page.height).sum();

    Ok(PageLayout {
        pages,
        total_height,
    })
}

/// Положить строки абзаца на страницы: строка за строкой.
///
/// Абзац переполняет страницу строкой, а не целиком: что не влезло — уходит
/// на следующую. Явный `w:br w:type="page"` закрывает страницу до строк
/// абзаца, а `w:keepLines` и висячие строки переносят его целиком.
fn place_lines(
    paginator: &mut Paginator,
    paragraph: &Paragraph,
    lines: &[&Line],
    layout: &ParagraphLayout,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    // Явный разрыв стоит до строк абзаца: знак абзаца Word оставляет
    // уже на новой странице.
    if has_page_break(paragraph) {
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::PageBreak);
        break_page(paginator, placed_floats, state);
    }

    // Сколько строк остаётся на текущей странице; хвост — на следующей.
    let kept = lines_kept_on_page(paginator, lines, layout);

    for (index, line) in lines.iter().enumerate() {
        if index == kept {
            break_page(paginator, placed_floats, state);
        }
        let first = index == 0;
        let last = index + 1 == lines.len();
        // Интервалы идут в те же единицы, что и `advance`: место занимает
        // не только строка, но и промежуток вокруг неё.
        let mut height = line.height;
        if first {
            height += layout.space_before;
        }
        if last {
            height += layout.space_after;
        }
        // `w:pageBreakBefore` — разрыв перед абзацем, флагом в `add_item`;
        // после уже сделанного разрыва он не нужен: страница и так новая.
        let needs_break = first && layout.page_break_before && kept > 0;
        place(paginator, height, needs_break, placed_floats, state);

        // Верх строки: курсор стоит за её нижней границей, а интервал перед
        // абзацем уже учтён в `height` — он сдвигает только эту строку.
        let top = paginator.current_y() - line.height;
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::Paragraph {
                node_id: paragraph.id,
                rect: Rect::new(line.x, top, line.width, line.height),
                text: line.text.clone(),
                line_height: line.height,
                color: line.color,
            });
    }
}

/// Положить таблицу: не поместилась — уходит на следующую страницу.
fn place_table(
    table: &Table,
    fonts: &mut FontRegistry,
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    let table_layout = crate::layout::tables::layout_table(
        table,
        state.column_x,
        paginator.current_y(),
        state.column_width,
        fonts,
    );

    // TODO: Add table to current page
    place(paginator, table_layout.height, false, placed_floats, state);
}

/// Положить на страницу фрагмент высотой `height` и продвинуть курсор.
///
/// `add_item` на разрыве открывает новую страницу, но высоту не учитывает:
/// фрагмент встаёт в её полосу набора, поэтому положить его нужно ещё раз.
/// Если он не помещается и там, второго разрыва подряд не будет — фрагмент
/// остаётся на новой странице и выходит за нижнее поле.
fn place(
    paginator: &mut Paginator,
    height: f32,
    page_break: bool,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    if paginator.add_item(height, page_break) {
        return;
    }
    reset_page_state(paginator, placed_floats, state);
    if !paginator.add_item(height, false) {
        paginator.advance(height);
    }
}

/// Начать новую страницу: обтекание и координаты живут в её пределах.
fn break_page(
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    paginator.push_page();
    reset_page_state(paginator, placed_floats, state);
}

/// Вернуть состояние к началу полосы набора текущей страницы.
///
/// Единственная точка сброса: смена страницы где угодно обязана пройти здесь,
/// иначе обтекание предыдущей страницы переехало бы на следующую.
fn reset_page_state(
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    placed_floats.clear();
    *state = LayoutState::for_page(paginator.current_page(), paginator.current_section());
}

/// Сколько строк абзаца остаётся на текущей странице.
///
/// Абзац, влезающий целиком, отдаёт все строки; `0` означает, что он уходит
/// на следующую страницу весь — так ведут себя `w:keepLines` и висячие строки:
/// ни одна строка при этом не остаётся на прежней странице.
fn lines_kept_on_page(paginator: &Paginator, lines: &[&Line], layout: &ParagraphLayout) -> usize {
    let mut height = layout.space_before;
    let mut fit = 0;
    for (index, line) in lines.iter().enumerate() {
        height += line.height;
        if index + 1 == lines.len() {
            height += layout.space_after;
        }
        if !paginator.fits(height) {
            break;
        }
        fit += 1;
    }

    if fit == lines.len() {
        return fit;
    }
    // На пустой странице делить нечего: разрыв зациклил бы раскладку, поэтому
    // абзац начинается здесь, даже если строка выходит за нижнее поле.
    if paginator.current_y() <= paginator.content_top() {
        return fit.max(1);
    }
    if layout.keep_lines {
        return 0;
    }

    let mut kept = fit;
    if layout.widow_control {
        // Word оставляет не меньше двух строк с каждой стороны разрыва.
        if kept == 1 {
            kept = 0;
        } else if lines.len() - kept == 1 {
            kept -= 1;
            if kept == 1 {
                kept = 0;
            }
        }
    }
    kept
}

/// Есть ли в абзаце явный разрыв страницы (`w:br w:type="page"`).
fn has_page_break(paragraph: &Paragraph) -> bool {
    paragraph.runs.iter().any(|inline| match inline {
        Inline::Break(BreakKind::Page) => true,
        Inline::Run(run) => run
            .content
            .iter()
            .any(|content| matches!(content, RunContent::Break(BreakKind::Page))),
        _ => false,
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

/// Перейти к секции `index`: с разрывом страницы или без него.
///
/// Тип перехода задаёт сама секция (`w:type` описывает, как она начинается):
/// `Continuous` меняет геометрию на текущей странице, остальные типы начинают
/// новую. Если концы секций исчерпаны, текущая секция — последняя, и переходить
/// некуда: так закрывается финальный `w:sectPr` тела.
fn enter_section(
    document: &Document,
    index: usize,
    paginator: &mut Paginator,
    placed_floats: &mut Vec<crate::layout::float::FloatElement>,
    state: &mut LayoutState,
) {
    let Some(section) = document.body.sections.get(index) else {
        return;
    };
    let needs_break = Paginator::needs_section_break(
        section.properties.section_type,
        paginator.current_page().number,
    );

    if needs_break {
        paginator.push_page_with_section(section);
        reset_page_state(paginator, placed_floats, state);
    } else {
        paginator.set_section(section);
        *state = LayoutState::for_page(paginator.current_page(), section);
    }
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
    use crate::model::raw::Toggle;
    use crate::model::{
        Body, BreakKind, Inline, Metadata, NumberingTable, Paragraph, Relationships, Run,
        RunContent, SectionType, Settings, StyleTable,
    };

    /// Абзац из одного run'а; идентификаторы произвольные — раскладка их не сверяет.
    fn paragraph(id: u64, text: &str) -> Paragraph {
        Paragraph {
            id: NodeId::new(id),
            runs: vec![Inline::Run(Run {
                id: NodeId::new(id + 1),
                content: vec![RunContent::Text(text.to_owned())],
                ..Run::default()
            })],
            ..Paragraph::default()
        }
    }

    /// Документ из готовых блоков и секций — так же, как их собрал бы парсер.
    fn document_with(items: Vec<BlockItem>, sections: Vec<Section>) -> Document {
        Document {
            id: NodeId::ROOT,
            body: Body {
                id: NodeId::new(3),
                items,
                sections,
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

    /// Документ из одного абзаца с текстом.
    ///
    /// `sections` пуст намеренно: раскладка обязана взять секцию по умолчанию —
    /// так же, как для документа без заключительного `w:sectPr`.
    fn document_with_text(text: &str) -> Document {
        document_with(vec![BlockItem::Paragraph(paragraph(1, text))], Vec::new())
    }

    /// Документ из `count` однострочных абзацев в секции `section`.
    fn document_in_section(section: Section, count: usize) -> Document {
        let items = (0..count)
            .map(|index| {
                // Сдвиг на единицу: идентификатор абзаца не должен совпасть с run'ом.
                let id = u64::try_from(index).unwrap_or(u64::MAX) * 2 + 1;
                BlockItem::Paragraph(paragraph(id, &format!("line {index}")))
            })
            .collect();
        document_with(items, vec![section])
    }

    /// Секция с заданными размерами и типом разрыва (`w:type`).
    fn section_of(
        width_twips: i32,
        height_twips: i32,
        section_type: Option<SectionType>,
    ) -> Section {
        let mut section = default_section();
        let page_size = PageSize {
            width: Twips::new(width_twips),
            height: Twips::new(height_twips),
        };
        section.page_size = page_size.clone();
        section.properties.page_size = page_size;
        section.properties.section_type = section_type;
        section
    }

    /// Секция без полей заданной высоты: полоса набора — вся страница.
    ///
    /// Высота [`TWO_LINE_PAGE_TWIPS`] выбрана так, чтобы сумма высот двух строк
    /// в f32 совпала с нижней границей полосы бит в бит: иначе тест ловил бы
    /// округление, а не правило «ровно по нижнему краю — помещается».
    fn section_without_margins(height_twips: i32) -> Section {
        let mut section = section_of(11_906, height_twips, None);
        let margins = Margins {
            top: Twips::new(0),
            right: Twips::new(0),
            bottom: Twips::new(0),
            left: Twips::new(0),
            header: None,
            footer: None,
            gutter: None,
        };
        section.margins = margins.clone();
        section.properties.margins = margins;
        section
    }

    /// Число элементов-абзацев на странице.
    fn paragraphs_on(page: &Page) -> usize {
        page.items
            .iter()
            .filter(|item| matches!(item, LayoutItem::Paragraph { .. }))
            .count()
    }

    /// Две строки по 19.2 px: 2 × 19.2 = 38.4 px = 576 twips.
    const TWO_LINE_PAGE_TWIPS: i32 = 576;

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

    /// Блокер S4: контент ровно по нижнему краю полосы остаётся на странице.
    #[test]
    fn content_exactly_at_the_content_bottom_stays_on_the_page() {
        let document = document_in_section(section_without_margins(TWO_LINE_PAGE_TWIPS), 2);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            2,
            "каждый абзац — одна строка"
        );
        assert_eq!(
            layout.pages.len(),
            1,
            "две строки ровно по нижнюю границу остаются на первой странице"
        );
    }

    /// Одна строка сверх полосы набора начинает вторую страницу.
    #[test]
    fn a_line_past_the_content_bottom_starts_a_second_page() {
        let document = document_in_section(section_without_margins(TWO_LINE_PAGE_TWIPS), 3);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 2, "третья строка не помещается");
        assert_eq!(paragraphs_on(&layout.pages[0]), 2);
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            1,
            "лишняя строка уходит на вторую страницу"
        );
    }

    /// Секция с одинаковыми полями: полоса набора — страница без верхнего и нижнего поля.
    fn section_with_margins(width_twips: i32, height_twips: i32, margin_twips: i32) -> Section {
        let mut section = section_of(width_twips, height_twips, None);
        let margins = Margins {
            top: Twips::new(margin_twips),
            right: Twips::new(margin_twips),
            bottom: Twips::new(margin_twips),
            left: Twips::new(margin_twips),
            header: None,
            footer: None,
            gutter: None,
        };
        section.margins = margins.clone();
        section.properties.margins = margins;
        section
    }

    /// Размеры и поля A4 из фикстур: 1134 twips = 75.6 px.
    const A4_WIDTH_TWIPS: i32 = 11_906;
    const A4_HEIGHT_TWIPS: i32 = 16_838;
    const FIXTURE_MARGIN_TWIPS: i32 = 1_134;

    /// Высота строки текста: её задаёт кегль из каскада стилей, а не литерал
    /// в тесте — кегль уже менялся, и полоса набора менялась вместе с ним.
    fn body_line_height() -> f32 {
        let document = document_with_text("hello");
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                LayoutItem::Paragraph { line_height, .. } => Some(*line_height),
                _ => None,
            })
            .expect("абзац с текстом даёт строку")
    }

    /// Сколько строк высотой `line_height` влезает в полосу набора страницы.
    ///
    /// Считается тем же накоплением и с тем же нестрогим сравнением, что
    /// `Paginator::fits`: иначе тест ловил бы округление f32, а не правило.
    fn line_capacity(height_twips: i32, margin_twips: i32, line_height: f32) -> usize {
        let content_bottom =
            twips_to_px(Twips::new(height_twips)) - twips_to_px(Twips::new(margin_twips));
        let mut y = twips_to_px(Twips::new(margin_twips));
        let mut count = 0;
        while y + line_height <= content_bottom {
            y += line_height;
            count += 1;
        }
        count
    }

    /// Все элементы-абзацы документа.
    fn total_paragraphs(layout: &PageLayout) -> usize {
        layout.pages.iter().map(paragraphs_on).sum()
    }

    /// Абзац с флагами `w:pPr`: `keep_lines` — `w:keepLines`, `widow_control` — `w:widowControl`.
    fn paragraph_with_flags(
        id: u64,
        text: &str,
        keep_lines: bool,
        widow_control: bool,
    ) -> Paragraph {
        let mut p = paragraph(id, text);
        p.ppr.keep_lines = keep_lines.then_some(Toggle::On);
        p.ppr.widow_control = widow_control.then_some(Toggle::On);
        p
    }

    /// Блокер S6: страницу переполняет строка, а не абзац целиком.
    #[test]
    fn a_line_past_the_page_bottom_splits_the_flow() {
        let line_height = body_line_height();
        let capacity = line_capacity(A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS, line_height);
        assert!(
            capacity > 2,
            "полоса набора вмещает {capacity} строк — мало"
        );

        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        let document = document_in_section(section, capacity + 1);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(
            layout.pages.len(),
            2,
            "строка сверх полосы набора начинает вторую страницу"
        );
        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            capacity,
            "первая страница набирается до нижней границы"
        );
        assert_eq!(total_paragraphs(&layout), capacity + 1);
    }

    /// Пустой абзац занимает строку высотой шрифта, а не ноль.
    #[test]
    fn an_empty_paragraph_takes_one_line() {
        let document = document_with(
            vec![
                BlockItem::Paragraph(paragraph(1, "")),
                BlockItem::Paragraph(paragraph(3, "Non-empty")),
                BlockItem::Paragraph(paragraph(5, "")),
            ],
            Vec::new(),
        );
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(
            total_paragraphs(&layout),
            3,
            "каждый абзац оставляет строку, включая пустые"
        );
        let heights: Vec<f32> = layout.pages[0]
            .items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { line_height, .. } => Some(*line_height),
                _ => None,
            })
            .collect();
        assert!(
            heights.iter().all(|height| *height > 0.0),
            "высота строки не ноль: {heights:?}"
        );
        assert!(
            (heights[0] - heights[1]).abs() < 0.01,
            "пустая строка — той же высоты, что строка текста: {heights:?}"
        );
    }

    /// Явный `w:br w:type="page"` начинает новую страницу.
    #[test]
    fn an_explicit_page_break_starts_a_new_page() {
        let mut breaker = paragraph(3, "");
        breaker.runs.push(Inline::Run(Run {
            id: NodeId::new(4),
            content: vec![RunContent::Break(BreakKind::Page)],
            ..Run::default()
        }));
        let document = document_with(
            vec![
                BlockItem::Paragraph(paragraph(1, "Before the break")),
                BlockItem::Paragraph(breaker),
                BlockItem::Paragraph(paragraph(5, "After the page break")),
            ],
            Vec::new(),
        );
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(
            layout.pages.len(),
            2,
            "разрыв добавляет ровно одну страницу"
        );
        assert!(
            layout.pages[0]
                .items
                .iter()
                .any(|item| matches!(item, LayoutItem::PageBreak)),
            "разрыв отмечен на странице, которую он закрывает"
        );
        assert!(
            layout.pages[1].items.iter().any(|item| matches!(
                item,
                LayoutItem::Paragraph { text, .. } if text == "After the page break"
            )),
            "текст после разрыва — на второй странице"
        );
    }

    /// Документ из `filler` однострочных абзацев и завершающего абзаца.
    ///
    /// `filler` берётся от вместимости страницы, а не из литерала: сколько
    /// строк в полосе набора, решает кегль — его задаёт каскад стилей.
    fn document_with_filler(filler: usize, trailing: Paragraph) -> Document {
        let line_height = body_line_height();
        let capacity = line_capacity(A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS, line_height);
        assert!(capacity > filler, "полоса набора: {capacity} строк");

        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        let mut document = document_in_section(section, filler);
        document.body.items.push(BlockItem::Paragraph(trailing));
        document
    }

    /// Вместимость полосы набора в строках — от той же высоты строки, что и в раскладке.
    fn a4_capacity() -> usize {
        line_capacity(A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS, body_line_height())
    }

    /// Раскладка документа: тесты ниже падают с сообщением, а не с паникой внутри.
    fn layout_of(document: &Document) -> PageLayout {
        let mut fonts = FontRegistry::new(64);
        layout_document(document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать")
    }

    /// Трёхстрочный абзац без флагов переносится построчно: что влезло — внизу,
    /// остаток — на новой странице.
    #[test]
    fn a_paragraph_splits_between_pages_line_by_line() {
        // Под абзац остаётся полоса ровно в две строки, третья уходит на новую.
        let filler = a4_capacity() - 2;
        let document = document_with_filler(filler, paragraph(1, "aaa\nbbb\nccc"));

        let layout = layout_of(&document);

        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            filler + 2,
            "две строки абзаца остаются на первой странице"
        );
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            1,
            "третья строка — на второй"
        );
    }

    /// `w:keepLines`: абзац, не влезающий целиком, уходит на следующую страницу весь.
    #[test]
    fn keep_lines_moves_a_paragraph_to_the_next_page_whole() {
        let filler = a4_capacity() - 2;
        let kept = paragraph_with_flags(1, "aaa\nbbb\nccc", true, false);
        let document = document_with_filler(filler, kept);

        let layout = layout_of(&document);

        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            filler,
            "ни одной строки абзаца на первой странице"
        );
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            3,
            "абзац целиком — на второй"
        );
    }

    /// `w:widowControl`: одна строка не остаётся ни в конце, ни в начале страницы.
    #[test]
    fn widow_control_moves_a_lone_line_to_the_next_page() {
        // Две строки из трёх влезли бы, третья осталась бы одна на новой странице.
        let filler = a4_capacity() - 2;
        let widowed = paragraph_with_flags(1, "aaa\nbbb\nccc", false, true);
        let document = document_with_filler(filler, widowed);

        let layout = layout_of(&document);

        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            filler,
            "абзац не начинается"
        );
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            3,
            "абзац целиком — на второй"
        );
    }

    /// `w:widowControl` убирает и одинокую строку внизу страницы.
    #[test]
    fn widow_control_moves_an_orphan_line_too() {
        // Остаётся полоса ровно в одну строку: она была бы сиротой внизу.
        let filler = a4_capacity() - 1;
        let widowed = paragraph_with_flags(1, "aaa\nbbb\nccc", false, true);
        let document = document_with_filler(filler, widowed);

        let layout = layout_of(&document);

        assert_eq!(paragraphs_on(&layout.pages[0]), filler);
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            3,
            "абзац целиком — на второй"
        );
    }

    /// Фикстура `basic/breaks_and_tabs`: явный разрыв начинает вторую страницу.
    #[test]
    fn breaks_and_tabs_fixture_starts_a_second_page() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx/basic/breaks_and_tabs.docx");
        assert!(path.exists(), "нет фикстуры {}", path.display());
        let document = crate::open(std::fs::read(&path).expect("фикстура читается"))
            .expect("фикстура разбирается");

        let layout = layout_of(&document);

        assert_eq!(layout.pages.len(), 2, "разрыв в фикстуре начинает страницу");
        assert!(
            layout.pages[1].items.iter().any(|item| matches!(
                item,
                LayoutItem::Paragraph { text, .. } if text == "After the page break"
            )),
            "текст после разрыва — на второй странице"
        );
    }

    /// Фикстура `edge_cases/empty_paragraphs`: строку оставляет каждый абзац.
    #[test]
    fn empty_paragraphs_fixture_keeps_every_paragraph() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx/edge_cases/empty_paragraphs.docx");
        assert!(path.exists(), "нет фикстуры {}", path.display());
        let document = crate::open(std::fs::read(&path).expect("фикстура читается"))
            .expect("фикстура разбирается");

        let layout = layout_of(&document);

        assert_eq!(
            total_paragraphs(&layout),
            3,
            "в фикстуре три абзаца, включая пустые"
        );
    }

    /// `Continuous` меняет геометрию, не начиная новую страницу.
    #[test]
    fn a_continuous_section_changes_geometry_without_a_page_break() {
        // Первая секция — книжная A4; вторая объявлена `Continuous`: содержимое
        // продолжается на той же странице, но лист уже альбомный.
        let portrait = section_of(11_906, 16_838, Some(SectionType::NextPage));
        let landscape = section_of(16_838, 11_906, Some(SectionType::Continuous));
        let mut first = paragraph(1, "First section");
        first.section_break = Some(Box::new(portrait.properties.clone()));
        let document = document_with(
            vec![
                BlockItem::Paragraph(first),
                BlockItem::Paragraph(paragraph(3, "Second section")),
                BlockItem::SectPr(landscape.properties.clone()),
            ],
            vec![portrait, landscape],
        );
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 1, "Continuous не начинает страницу");
        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            2,
            "оба абзаца остались на той же странице"
        );
        assert!(
            (layout.pages[0].width - twips_to_px(Twips::new(16_838))).abs() < 0.01,
            "ширина страницы — уже из второй секции: {}",
            layout.pages[0].width
        );
    }
}
