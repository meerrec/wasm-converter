//! Движок раскладки DOCX: преобразование модели в страницы.
//!
//! Обходит блоки тела документа по порядку, раскладывает абзацы и таблицы, а
//! постраничную разбивку ведёт [`Paginator`]:
//! он же — единственный источник текущей позиции Y. Текст измеряется средствами
//! `doc-converter-render`, чтобы точки разрыва не разъезжались между canvas
//! и PDF (ADR-0005).

use doc_converter_core::NodeId;
use doc_converter_render::{
    font::{FontId, FontRegistry},
    text_measure,
};

use crate::layout::cascade::StyleCache;
use crate::layout::float::{layout_floats_on_page, FloatElement};
use crate::layout::line_break::LineBreaker;
use crate::layout::pagination::Paginator;
use crate::layout::paragraph::{layout_paragraph, Line, ParagraphLayout};

use crate::model::drawing::Anchor;
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
    /// Плавающий рисунок (`wp:anchor`).
    Float(FloatElement),
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
    // Анкоры, которым не хватило места на их странице: очередь переживает
    // смену страницы, в отличие от `placed_floats`, которые она очищает.
    let mut deferred_floats: Vec<DeferredFloat> = Vec::new();

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

                let placement = place_lines(
                    &mut paginator,
                    paragraph,
                    &lines,
                    &layout,
                    &mut placed_floats,
                    &mut state,
                );

                // Плавающие рисунки встают после строк абзаца: их база —
                // фрагмент, в который абзац лёг.
                place_floats(
                    &mut paginator,
                    &layout.anchors,
                    placement,
                    &mut placed_floats,
                    &mut deferred_floats,
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
) -> ParagraphPlacement {
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

    // Верх фрагмента абзаца на странице, где он закончится: у разорванного
    // абзаца `relativeFrom="paragraph"` отсчитывается от полосы набора той
    // страницы, а не от начала абзаца на предыдущей.
    let mut fragment_top = paginator.current_y();
    let mut page_number = paginator.current_page().number;

    for (index, line) in lines.iter().enumerate() {
        if index == kept {
            break_page(paginator, placed_floats, state);
            fragment_top = state.y;
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

        // Строка, открывшая новую страницу, начинает на ней фрагмент абзаца.
        // `state.y` — полоса набора этой страницы: её ставит `reset_page_state`.
        if paginator.current_page().number != page_number {
            page_number = paginator.current_page().number;
            fragment_top = state.y;
        }

        // Верх строки: курсор стоит за её нижней границей, а интервалы абзаца
        // учтены в `height`. Интервал перед сдвигает первую строку, интервал
        // после остаётся под последней — из `height` его нужно вычесть.
        let after = if last { layout.space_after } else { 0.0 };
        let top = paginator.current_y() - line.height - after;
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

    ParagraphPlacement {
        top: fragment_top,
        height: paginator.current_y() - fragment_top,
    }
}

/// Куда лёг абзац: база его плавающих элементов.
#[derive(Debug, Clone, Copy)]
struct ParagraphPlacement {
    /// Верх фрагмента абзаца на странице, где он закончился.
    top: f32,
    /// Высота этого фрагмента.
    height: f32,
}

/// Анкор, ждущий следующей страницы.
#[derive(Debug, Clone)]
struct DeferredFloat {
    /// Плавающий рисунок из модели.
    anchor: Anchor,
    /// Высота абзаца-якоря: конец его базы по вертикали.
    paragraph_height: f32,
}

/// Разместить анкоры абзаца и накопленную очередь отложенных.
///
/// Не поместившиеся переезжают на следующую страницу: `placed_floats` разрыв
/// очищает, а очередь — нет. Очередь заканчивается, когда страница пуста:
/// элемент выше полосы набора не влезет и в следующие, а разрыв под него
/// зациклил бы раскладку.
fn place_floats(
    paginator: &mut Paginator,
    anchors: &[Anchor],
    placement: ParagraphPlacement,
    placed_floats: &mut Vec<FloatElement>,
    deferred_floats: &mut Vec<DeferredFloat>,
    state: &mut LayoutState,
) {
    let mut queue: Vec<DeferredFloat> = std::mem::take(deferred_floats);
    // Анкоры текущего абзаца встают в конец: отложенные ждут дольше всех.
    queue.extend(anchors.iter().map(|anchor| DeferredFloat {
        anchor: anchor.clone(),
        paragraph_height: placement.height,
    }));

    let mut text_y = placement.top;
    loop {
        let placed_before = placed_floats.len();
        queue = place_floats_on_page(paginator, queue, text_y, placed_floats);
        for float in placed_floats[placed_before..].iter().cloned() {
            paginator
                .current_page_mut()
                .items
                .push(LayoutItem::Float(float));
        }
        if queue.is_empty() {
            break;
        }
        // Пустая страница — предел: элемент выше полосы набора, новых страниц
        // под него не будет, и он останется в очереди до конца документа.
        if paginator.current_y() <= paginator.content_top() {
            break;
        }
        break_page(paginator, placed_floats, state);
        // На новой странице абзац-якорь начинается от её полосы набора.
        text_y = paginator.content_top();
    }

    *deferred_floats = queue;
}

/// Попробовать положить очередь на текущую страницу.
///
/// Каждый анкор примеряется отдельно: у отложенных базы разные, а
/// [`layout_floats_on_page`] размещает пачку с общей базой. Не поместившиеся
/// возвращаются — очередь доберётся до них на следующей странице.
fn place_floats_on_page(
    paginator: &Paginator,
    queue: Vec<DeferredFloat>,
    text_y: f32,
    placed_floats: &mut Vec<FloatElement>,
) -> Vec<DeferredFloat> {
    let mut deferred = Vec::new();
    for item in queue {
        let not_fit = layout_floats_on_page(
            std::slice::from_ref(&item.anchor),
            paginator.current_page(),
            text_y,
            item.paragraph_height,
            placed_floats,
        );
        if !not_fit.is_empty() {
            deferred.push(item);
        }
    }
    deferred
}

/// Положить таблицу: не поместилась — уходит на следующую страницу.
fn place_table(
    table: &Table,
    fonts: &mut FontRegistry,
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    let left = state.column_x;
    let top = paginator.current_y();
    let table_layout =
        crate::layout::tables::layout_table(table, left, top, state.column_width, fonts);

    // `place` переносит таблицу целиком, если та не поместилась: курсор после
    // него стоит под таблицей на той странице, куда она в итоге легла.
    place(paginator, table_layout.height, false, placed_floats, state);

    // Ячейки разложены от прежнего верха страницы; на новой странице сдвиг
    // тот же, что у самой таблицы, — иначе содержимое осталось бы на прежнем месте.
    let table_top = paginator.current_y() - table_layout.height;
    let dy = table_top - top;
    let cells = table_layout
        .rows
        .into_iter()
        .flat_map(|row| row.cells)
        .map(|cell| TableCellLayout {
            rect: cell.rect.offset(0.0, dy),
            content: cell.content,
        })
        .collect();

    paginator.current_page_mut().items.push(LayoutItem::Table {
        node_id: table.id,
        rect: Rect::new(left, table_top, table_layout.width, table_layout.height),
        cells,
    });
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
        // При `kept == 0` сдвигать нечего: абзац уже уходит целиком, а `kept - 1`
        // ушло бы в минус по `usize` (абзац из одной строки).
        if kept == 1 {
            kept = 0;
        } else if kept > 0 && lines.len() - kept == 1 {
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
    use crate::model::raw::{ParagraphSpacing, Toggle};
    use crate::model::{
        Body, BreakKind, Extent, Inline, InlineImage, InlineOrAnchor, Metadata, NumberingTable,
        Paragraph, PositionH, PositionV, RelFromH, RelFromV, Relationships, Run, RunContent,
        SectionType, Settings, StyleTable, WrapKind,
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

    /// Метрики строки тела: высота строки и интервалы абзаца из каскада стилей.
    ///
    /// Интервалы приходят из `docDefaults`, поэтому измеряются по раскладке —
    /// литералом их не задать.
    #[derive(Clone, Copy)]
    struct BodyMetrics {
        line_height: f32,
        space_before: f32,
        space_after: f32,
    }

    impl BodyMetrics {
        /// Шаг однострочного абзаца: строка и оба её интервала.
        fn pitch(self) -> f32 {
            self.space_before + self.line_height + self.space_after
        }
    }

    /// Нижняя граница полосы набора фикстурной страницы A4.
    fn content_bottom() -> f32 {
        twips_to_px(Twips::new(A4_HEIGHT_TWIPS)) - twips_to_px(Twips::new(FIXTURE_MARGIN_TWIPS))
    }

    /// Измерить метрики по двум однострочным абзацам: расстояние между их
    /// строками — это высота строки плюс интервалы абзаца.
    fn body_metrics() -> BodyMetrics {
        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        let layout = layout_of(&document_in_section(section, 2));
        let page = &layout.pages[0];
        let rows: Vec<(f32, f32)> = page
            .items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph {
                    rect, line_height, ..
                } => Some((rect.y, *line_height)),
                _ => None,
            })
            .collect();
        let space_before = rows[0].0 - page.margins.top;
        let line_height = rows[0].1;
        BodyMetrics {
            line_height,
            space_before,
            space_after: rows[1].0 - rows[0].0 - line_height - space_before,
        }
    }

    /// Вместимость полосы набора в однострочных абзацах — вместе с интервалами.
    fn a4_capacity() -> usize {
        let metrics = body_metrics();
        let mut y = twips_to_px(Twips::new(FIXTURE_MARGIN_TWIPS));
        let mut count = 0;
        while y + metrics.pitch() <= content_bottom() {
            y += metrics.pitch();
            count += 1;
        }
        count
    }

    /// Сколько строк абзаца помещается после `filler` однострочных абзацев.
    ///
    /// Считается так же, как `lines_kept_on_page`: `space_before` уходит в высоту
    /// первой строки, `space_after` — последней. Правила переноса (висячие строки,
    /// `w:keepLines`) сюда не входят — их проверяют тесты ниже.
    fn lines_fitting_after(filler: usize, lines: usize) -> usize {
        let metrics = body_metrics();
        let mut y = twips_to_px(Twips::new(FIXTURE_MARGIN_TWIPS));
        for _ in 0..filler {
            y += metrics.pitch();
        }

        let mut fit = 0;
        // Высоты набираются так же, как в `lines_kept_on_page`.
        let mut height = metrics.space_before;
        for index in 1..=lines {
            height += metrics.line_height;
            if index == lines {
                height += metrics.space_after;
            }
            if y + height > content_bottom() {
                break;
            }
            fit = index;
        }
        fit
    }

    /// Все элементы-абзацы документа.
    fn total_paragraphs(layout: &PageLayout) -> usize {
        layout.pages.iter().map(paragraphs_on).sum()
    }

    /// Абзац с флагами `w:pPr`: `keep_lines` — `w:keepLines`, `widow_control` — `w:widowControl`.
    ///
    /// Тумблеры ставятся явно в обоих положениях: `None` каскад разрешил бы
    /// по-своему (`w:widowControl` по умолчанию включён), и тест проверял бы
    /// не то, что задумано.
    fn paragraph_with_flags(
        id: u64,
        text: &str,
        keep_lines: bool,
        widow_control: bool,
    ) -> Paragraph {
        let mut p = paragraph(id, text);
        p.ppr.keep_lines = Some(toggle(keep_lines));
        p.ppr.widow_control = Some(toggle(widow_control));
        p
    }

    /// Тумблер в явном положении.
    fn toggle(on: bool) -> Toggle {
        if on {
            Toggle::On
        } else {
            Toggle::Off
        }
    }

    /// Абзац с интервалами `w:spacing` в twips — их разрешает каскад.
    fn paragraph_with_spacing(id: u64, text: &str, before: i32, after: i32) -> Paragraph {
        let mut p = paragraph(id, text);
        p.ppr.spacing = Some(ParagraphSpacing {
            before: Some(Twips::new(before)),
            after: Some(Twips::new(after)),
            ..ParagraphSpacing::default()
        });
        p
    }

    /// Прямоугольники строк сверху вниз.
    fn line_rects(page: &Page) -> Vec<Rect> {
        page.items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    /// Блокер S6: страницу переполняет строка, а не абзац целиком.
    #[test]
    fn a_line_past_the_page_bottom_splits_the_flow() {
        let capacity = a4_capacity();
        assert!(
            capacity > 2,
            "полоса набора вмещает {capacity} абзацев — мало"
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

    /// Интервал после абзаца остаётся под последней строкой, а не над ней.
    #[test]
    fn the_space_after_a_paragraph_stays_below_its_last_line() {
        let document = document_with(
            vec![BlockItem::Paragraph(paragraph_with_spacing(
                1, "hello", 240, 120,
            ))],
            Vec::new(),
        );
        let layout = layout_of(&document);
        let page = &layout.pages[0];
        let rects = line_rects(page);

        let expected = page.margins.top + twips_to_px(Twips::new(240));
        assert!(
            (rects[0].y - expected).abs() < 0.01,
            "строка — на верхнем поле плюс интервал перед абзацем ({expected} px), получено {}",
            rects[0].y
        );
    }

    /// Шаг строк абзаца ровный: интервал после не подтягивает последнюю строку.
    #[test]
    fn the_lines_of_a_paragraph_keep_an_even_pitch() {
        let document = document_with(
            vec![BlockItem::Paragraph(paragraph_with_spacing(
                1,
                "aaa\nbbb\nccc",
                0,
                120,
            ))],
            Vec::new(),
        );
        let layout = layout_of(&document);
        let rects = line_rects(&layout.pages[0]);

        assert_eq!(rects.len(), 3, "абзац из трёх строк");
        let line_height = rects[0].height;
        let tops: Vec<f32> = rects.iter().map(|rect| rect.y).collect();
        for pair in rects.windows(2) {
            assert!(
                (pair[1].y - pair[0].y - line_height).abs() < 0.01,
                "шаг строк равен их высоте {line_height}, а не строке с интервалом: {tops:?}"
            );
        }
    }

    /// Фикстура `basic/paragraph_spacing`: у первого абзаца `w:before=240`
    /// (16 px) — строка стоит на верхнем поле плюс интервал перед; `w:after`
    /// её не сдвигает. Эталон `LibreOffice` для этой фикстуры: 68.7 pt = 91.6 px.
    #[test]
    fn paragraph_spacing_fixture_keeps_the_first_line_under_its_before() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx/basic/paragraph_spacing.docx");
        assert!(path.exists(), "нет фикстуры {}", path.display());
        let document = crate::open(std::fs::read(&path).expect("фикстура читается"))
            .expect("фикстура разбирается");

        let layout = layout_of(&document);
        let rects = line_rects(&layout.pages[0]);

        let expected = layout.pages[0].margins.top + twips_to_px(Twips::new(240));
        assert!(
            (rects[0].y - expected).abs() < 0.01,
            "первый абзац — на поле плюс 240 twips ({expected} px), получено {}",
            rects[0].y
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
    /// абзацев влезает в полосу набора, решает каскад стилей.
    fn document_with_filler(filler: usize, trailing: Paragraph) -> Document {
        assert!(
            a4_capacity() > filler,
            "полоса набора меньше {filler} абзацев"
        );

        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        let mut document = document_in_section(section, filler);
        document.body.items.push(BlockItem::Paragraph(trailing));
        document
    }

    /// Раскладка документа: тесты ниже падают с сообщением, а не с паникой внутри.
    fn layout_of(document: &Document) -> PageLayout {
        let mut fonts = FontRegistry::new(64);
        layout_document(document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать")
    }

    /// Трёхстрочный абзац переносится построчно: что влезло — внизу, остаток — сверху.
    #[test]
    fn a_paragraph_splits_between_pages_line_by_line() {
        // Висячие строки выключены явно: тест про перенос строк, а не про них.
        let splitter = paragraph_with_flags(1, "aaa\nbbb\nccc", false, false);
        // Место под хвост считаем от той же геометрии, что и `lines_kept_on_page`.
        let filler = a4_capacity() - 2;
        let fits = lines_fitting_after(filler, 3);
        assert_eq!(fits, 2, "под абзац остаётся ровно две строки");

        let document = document_with_filler(filler, splitter);
        let layout = layout_of(&document);

        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            filler + fits,
            "начало абзаца остаётся на первой странице"
        );
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            3 - fits,
            "остаток абзаца — на второй"
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

    /// По умолчанию `w:widowControl` включён: абзацу, которому из четырёх строк
    /// не хватает места под три, остаётся две — 3/1 не допускается.
    #[test]
    fn widow_control_by_default_splits_two_and_two() {
        let filler = a4_capacity() - 3;
        let fits = lines_fitting_after(filler, 4);
        assert_eq!(fits, 3, "под абзац остаётся три строки");

        let document = document_with_filler(filler, paragraph(1, "aaa\nbbb\nccc\nddd"));
        let layout = layout_of(&document);

        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            filler + 2,
            "одна строка не остаётся внизу страницы"
        );
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            2,
            "одна строка не остаётся вверху страницы"
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

    /// Раскладка фикстуры с единственной таблицей.
    fn fixture_table_item(name: &str) -> (Rect, Vec<TableCellLayout>) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx")
            .join(name);
        let document = crate::open(std::fs::read(&path).expect("фикстура читается"))
            .expect("фикстура разбирается");
        let layout = layout_of(&document);

        let tables: Vec<(Rect, Vec<TableCellLayout>)> = layout
            .pages
            .iter()
            .flat_map(|page| page.items.iter())
            .filter_map(|item| match item {
                LayoutItem::Table { rect, cells, .. } => Some((*rect, cells.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(tables.len(), 1, "в фикстуре {name} ровно одна таблица");
        tables.into_iter().next().expect("таблица уже проверена")
    }

    /// Тексты строк ячейки по порядку.
    fn cell_texts(cell: &TableCellLayout) -> Vec<String> {
        cell.content
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Содержимое ячейки не выходит за её прямоугольник.
    fn content_inside(cell: &TableCellLayout) -> bool {
        cell.content.iter().all(|item| match item {
            LayoutItem::Paragraph { rect, .. } => {
                rect.x + 0.01 >= cell.rect.x
                    && rect.y + 0.01 >= cell.rect.y
                    && rect.x + rect.width <= cell.rect.x + cell.rect.width + 0.01
                    && rect.y + rect.height <= cell.rect.y + cell.rect.height + 0.01
            }
            _ => true,
        })
    }

    /// Таблица попадает на страницу элементом с ячейками и их текстом.
    ///
    /// Поля фикстур — 1134 twips = 75.6 px, таков же левый край полосы набора.
    #[test]
    fn a_two_by_two_table_lands_with_its_cells() {
        let (rect, cells) = fixture_table_item("tables/simple_2x2.docx");

        assert_eq!(cells.len(), 4, "в таблице 2×2 четыре ячейки");
        assert!(
            (rect.x - 75.6).abs() < 0.01,
            "таблица начинается на левом поле: {}",
            rect.x
        );

        let texts: Vec<Vec<String>> = cells.iter().map(cell_texts).collect();
        assert_eq!(
            texts,
            vec![vec!["A1"], vec!["A2"], vec!["B1"], vec!["B2"]],
            "ячейки идут построчно, каждая со своим текстом"
        );
        for cell in &cells {
            assert!(
                !cell.content.is_empty(),
                "у непустой ячейки есть содержимое"
            );
            assert!(content_inside(cell), "содержимое не выходит за ячейку");
        }
    }

    /// Таблица 3×3 отдаёт все девять ячеек, а не только высоту.
    #[test]
    fn a_three_by_three_table_lands_with_nine_cells() {
        let (_, cells) = fixture_table_item("tables/simple_3x3.docx");

        assert_eq!(cells.len(), 9, "в таблице 3×3 девять ячеек");
        assert!(
            cells.iter().all(content_inside),
            "содержимое каждой ячейки — внутри её прямоугольника"
        );
    }

    /// Таблица идёт за текстом: её ячейки несут заголовок `H1`/`H2` и строку `C1`/`C2`.
    #[test]
    fn a_table_after_a_paragraph_keeps_its_cells() {
        let (rect, cells) = fixture_table_item("complex/text_and_table.docx");

        assert_eq!(cells.len(), 4, "в таблице 2×2 четыре ячейки");
        assert!(
            (rect.x - 75.6).abs() < 0.01,
            "таблица начинается на левом поле: {}",
            rect.x
        );
        let texts: Vec<Vec<String>> = cells.iter().map(cell_texts).collect();
        assert_eq!(
            texts,
            vec![vec!["H1"], vec!["H2"], vec!["C1"], vec!["C2"]],
            "заголовок и строка данных — в своих ячейках"
        );
    }

    /// Таблица, не помещающаяся на странице, уходит на следующую целиком.
    #[test]
    fn a_table_that_does_not_fit_moves_to_the_next_page_whole() {
        use crate::model::CellVAlign;
        use crate::{Cell, CellBorders, CellMargins, GridCol, Row, TableLayout, TableLook};

        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        // Строк текста на странице — без одной: таблице остаётся меньше строки.
        let mut document = document_in_section(section, a4_capacity() - 1);
        document.body.items.push(BlockItem::Table(Table {
            id: NodeId::new(900),
            style_ref: None,
            grid: vec![GridCol {
                width: Twips::new(6000),
            }],
            rows: vec![Row {
                id: NodeId::new(901),
                cells: vec![Cell {
                    id: NodeId::new(902),
                    grid_span: 1,
                    v_merge: None,
                    width: None,
                    margins: CellMargins::default(),
                    v_align: CellVAlign::Top,
                    borders: Box::new(CellBorders::default()),
                    shading: None,
                    items: vec![
                        BlockItem::Paragraph(paragraph(903, "row one")),
                        BlockItem::Paragraph(paragraph(904, "row two")),
                    ],
                }],
                height: None,
                cant_split: false,
                header: false,
            }],
            layout: TableLayout::Autofit,
            width: None,
            borders: crate::TableBorders::default(),
            look: TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: CellMargins::default(),
        }));

        let layout = layout_of(&document);

        assert_eq!(
            layout.pages.len(),
            2,
            "таблица не влезла — страниц стало две"
        );
        assert!(
            !layout.pages[0]
                .items
                .iter()
                .any(|item| matches!(item, LayoutItem::Table { .. })),
            "на первой странице таблицы нет: она ушла целиком"
        );
        let second = &layout.pages[1];
        let (rect, cells) = second
            .items
            .iter()
            .find_map(|item| match item {
                LayoutItem::Table { rect, cells, .. } => Some((*rect, cells)),
                _ => None,
            })
            .expect("таблица — на второй странице");
        assert_eq!(cells.len(), 1, "ячейка таблицы доехала вместе с таблицей");
        assert!(
            (rect.y - second.margins.top).abs() < 0.01,
            "таблица начинается от верхнего поля новой страницы: {}",
            rect.y
        );
        assert!(
            rect.y + rect.height <= second.height - second.margins.bottom + 0.01,
            "таблица влезает в полосу набора второй страницы"
        );
    }

    /// Путь к фикстуре от корня репозитория.
    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/docx")
            .join(name)
    }

    /// Разложить фикстуру; её отсутствие роняет тест, а не пропускает его.
    fn layout_fixture(name: &str) -> PageLayout {
        let path = fixture_path(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|_| panic!("нет фикстуры {}", path.display()));
        layout_of(&crate::open(bytes).expect("фикстура разбирается"))
    }

    /// Плавающие элементы всех страниц в порядке рендеринга.
    fn floats_of(layout: &PageLayout) -> Vec<&FloatElement> {
        layout
            .pages
            .iter()
            .flat_map(|page| page.items.iter())
            .filter_map(|item| match item {
                LayoutItem::Float(float) => Some(float),
                _ => None,
            })
            .collect()
    }

    /// Сравнить пиксельный размер: `f32` из EMU на равенство не проверяем.
    fn assert_px(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 0.01,
            "{what}: ожидалось {expected} px, получено {actual} px"
        );
    }

    /// Анкор `cx`×`cy` EMU, привязанный к абзацу; `offset_v` — сдвиг по вертикали.
    fn anchored(id: u64, cx: i64, cy: i64, offset_v: i64) -> Anchor {
        Anchor {
            id: NodeId::new(id),
            extent: Extent { cx, cy },
            horizontal: PositionH {
                relative_from: RelFromH::Column,
                align: None,
                offset: Some(0),
                percent: None,
            },
            vertical: PositionV {
                relative_from: RelFromV::Paragraph,
                align: None,
                offset: Some(offset_v),
                percent: None,
            },
            wrap: WrapKind::Square,
            behind_text: false,
            image: InlineImage {
                id: NodeId::new(id),
                rel_id: format!("rId{id}"),
                part: None,
                name: None,
                description: None,
                extent: Extent { cx, cy },
            },
        }
    }

    /// Абзац из одного run'а с плавающими рисунками.
    fn paragraph_with_floats(id: u64, anchors: Vec<Anchor>) -> Paragraph {
        Paragraph {
            id: NodeId::new(id),
            runs: vec![Inline::Run(Run {
                id: NodeId::new(id + 1),
                content: anchors
                    .into_iter()
                    .map(|anchor| {
                        RunContent::Drawing(InlineOrAnchor {
                            id: anchor.id,
                            inline: None,
                            anchor: Some(anchor),
                        })
                    })
                    .collect(),
                ..Run::default()
            })],
            ..Paragraph::default()
        }
    }

    /// Сайдекары `images/*.json` дают размеры в EMU; 914400 EMU = 96 px.
    #[test]
    fn fixture_anchors_become_floats_on_the_page() {
        let cases = [
            (
                "images/anchor_wrap_square.docx",
                120.0,
                90.0,
                WrapKind::Square,
                false,
            ),
            (
                "images/anchor_behind_text.docx",
                144.0,
                96.0,
                WrapKind::None,
                true,
            ),
            (
                "images/anchor_top_and_bottom.docx",
                288.0,
                72.0,
                WrapKind::TopAndBottom,
                false,
            ),
            (
                "images/anchor_wrap_tight.docx",
                72.0,
                72.0,
                WrapKind::Tight,
                false,
            ),
        ];

        for (name, width, height, wrap, behind_text) in cases {
            let layout = layout_fixture(name);
            let floats = floats_of(&layout);
            assert_eq!(floats.len(), 1, "{name}: ровно один плавающий рисунок");
            let float = floats[0];
            assert_px(float.rect.width, width, name);
            assert_px(float.rect.height, height, name);
            assert_eq!(float.wrap, wrap, "{name}: обтекание");
            assert_eq!(float.behind_text, behind_text, "{name}: под текстом");
        }
    }

    /// Встроенные рисунки остаются в потоке: элементов `Float` не появляется.
    #[test]
    fn fixture_inline_drawings_stay_in_the_flow() {
        for name in ["images/inline.docx", "images/multiple_sizes.docx"] {
            let layout = layout_fixture(name);
            assert!(
                floats_of(&layout).is_empty(),
                "{name}: встроенные рисунки не плавают"
            );
        }
    }

    /// Два пересекающихся анкора: второй сдвигается под нижнюю границу первого.
    #[test]
    fn overlapping_anchors_shift_the_second_down() {
        // 952500×476250 EMU — это 100×50 px, как в тестах `float`.
        let document = document_with(
            vec![BlockItem::Paragraph(paragraph_with_floats(
                1,
                vec![
                    anchored(2, 952_500, 476_250, 0),
                    anchored(4, 952_500, 476_250, 0),
                ],
            ))],
            Vec::new(),
        );

        let layout = layout_of(&document);
        let floats = floats_of(&layout);

        assert_eq!(floats.len(), 2, "оба анкора на странице");
        let (first, second) = (floats[0], floats[1]);
        assert_eq!(first.id, 2, "первым — анкор, появившийся раньше");
        assert_eq!(second.id, 4);
        assert_px(second.rect.x, first.rect.x, "сдвиг только по вертикали");
        assert_px(
            second.rect.y,
            first.rect.y + first.rect.height,
            "второй анкор — под первым",
        );
    }

    /// Анкор, которому не хватило места внизу страницы, переезжает на следующую.
    #[test]
    fn an_anchor_that_does_not_fit_moves_to_the_next_page() {
        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        let mut items: Vec<BlockItem> = (0..a4_capacity() - 1)
            .map(|index| {
                let id = u64::try_from(index).unwrap_or(u64::MAX) * 2 + 1;
                BlockItem::Paragraph(paragraph(id, &format!("line {index}")))
            })
            .collect();
        // Анкор-абзац встаёт последней строкой страницы: его рисунку (50 px)
        // места под строкой уже не остаётся.
        let anchor_id = 10_000;
        items.push(BlockItem::Paragraph(paragraph_with_floats(
            anchor_id,
            vec![anchored(7, 952_500, 476_250, 0)],
        )));

        let layout = layout_of(&document_with(items, vec![section]));

        assert_eq!(
            layout.pages.len(),
            2,
            "под плавающий рисунок — вторая страница"
        );
        assert!(
            !layout.pages[0]
                .items
                .iter()
                .any(|item| matches!(item, LayoutItem::Float(_))),
            "на первой странице рисунка нет: он не поместился"
        );
        assert!(
            layout.pages[0].items.iter().any(|item| matches!(
                item,
                LayoutItem::Paragraph { node_id, .. } if *node_id == NodeId::new(anchor_id)
            )),
            "абзац-якорь остался на первой странице"
        );

        let second = &layout.pages[1];
        let floats: Vec<&FloatElement> = second
            .items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Float(float) => Some(float),
                _ => None,
            })
            .collect();
        assert_eq!(floats.len(), 1, "рисунок переехал целиком");
        assert_px(
            floats[0].rect.y,
            second.margins.top,
            "на новой странице рисунок встаёт от её полосы набора",
        );
    }
}
