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
use crate::layout::fields::{resolve_paragraph, FieldPass};
use crate::layout::float::{emu_to_px, layout_floats_on_page, FloatElement, FloatKind};
use crate::layout::line_break::LineBreaker;
use crate::layout::pagination::{HeaderExtent, Paginator};
use crate::layout::paragraph::{layout_paragraph, Line, ParagraphLayout};
use crate::layout::tables::{
    compute_table_frame, layout_table_fragments, table_item, PageBand, TableRowLayout,
};

use crate::model::drawing::{Anchor, InlineImage, InlineOrAnchor, MediaPart};
use crate::model::raw::HalfPoint;
use crate::model::{
    BlockItem, BreakKind, CellBorders, Document, Inline, Margins, Orientation, PageSize, Paragraph,
    PartRef, RunContent, Section, SectionProperties, Shading, StyleTable, Table,
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
        /// Гарнитура (`w:rFonts`); `None` — гарнитура по умолчанию.
        font_family: Option<String>,
        /// Кегль (`w:sz`) в полупунктах; в пиксели переводит [`half_points_to_px`].
        size: HalfPoint,
        /// Полужирный (`w:b`).
        bold: bool,
        /// Курсив (`w:i`).
        italic: bool,
        /// Подчёркивание (`w:u`).
        underline: bool,
    },
    /// Таблица — фрагмент на одной странице.
    ///
    /// Таблица, не поместившаяся на странице, приходит несколькими элементами:
    /// по одному на страницу, каждый со своими координатами. Склейка фрагментов
    /// в один прямоугольник сломала бы и содержимое, и границы разрезанных строк.
    Table {
        /// Идентификатор узла.
        node_id: NodeId,
        /// Прямоугольник фрагмента.
        rect: Rect,
        /// Раскладка строк фрагмента.
        rows: Vec<TableRowLayout>,
        /// Плоский список ячеек фрагмента — в порядке отрисовки (см. [`TableRowLayout::cells`]).
        cells: Vec<TableCellLayout>,
    },
    /// Разрыв страницы.
    PageBreak,
    /// Разрыв колонки.
    ColumnBreak,
    /// Плавающий рисунок (`wp:anchor`).
    Float(FloatElement),
    /// Встроенный рисунок (`wp:inline`) в потоке.
    ///
    /// Рисунок занимает в потоке прямоугольник [`Rect`] и раздвигает содержимое
    /// вокруг себя, как это делает Word; место внутри строки текста (рисунок
    /// между словами) — отдельный долг, сейчас рисунок абзаца с текстом встаёт
    /// после его строк.
    Image {
        /// Идентификатор узла.
        node_id: NodeId,
        /// Прямоугольник положения.
        rect: Rect,
        /// Идентификатор байтов в [`crate::Document::media`]; `None` — байтов
        /// нет (битая часть, внешняя цель, часть выше предела): кадр рисует
        /// габариты плейсхолдером.
        image_id: Option<u32>,
    },
}

/// Раскладка одной ячейки таблицы.
#[derive(Debug, Clone)]
pub struct TableCellLayout {
    /// Прямоугольник положения.
    pub rect: Rect,
    /// Содержимое ячейки.
    pub content: Vec<LayoutItem>,
    /// Выравнивание содержимого по вертикали (`w:vAlign`).
    pub v_align: VAlign,
    /// Заливка ячейки (`w:shd`).
    pub shading: Option<Shading>,
    /// Эффективные границы ячейки: собственные поверх табличных.
    ///
    /// Рёбра разреза страницы пусты (`None`): линия разреза не рисуется.
    pub borders: Box<CellBorders>,
    /// Ячейка — продолжение вертикального объединения: её содержимое осталось
    /// в начале объединения, здесь только геометрия.
    pub v_merge_continue: bool,
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
    /// Верхнее поле — верхняя граница полосы набора.
    ///
    /// Больше `w:pgMar/@w:top`, когда колонтитул в поле не поместился: тело
    /// отодвигается под него ([`Paginator::set_header_extents`]), как в Word.
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
    /// Колонтитулы страниц: `chrome[i]` относится к `pages[i]`.
    ///
    /// Колонтитулы держатся рядом со страницами, а не в [`Page::items`]:
    /// `items` — поток содержимого, по нему считается высота и режется таблица,
    /// и попавший туда колонтитул сдвинул бы разбивку. Потребителю (canvas и PDF)
    /// списки нужны раздельно ещё и потому, что колонтитул рисуется слоем,
    /// отличным от тела.
    pub chrome: Vec<PageChrome>,
    /// Общая высота содержимого (без учёта полей и колонтитулов).
    pub total_height: f32,
}

/// Колонтитулы одной страницы: то, что печатается в её полях.
///
/// Элементы лежат в координатах страницы, как и [`Page::items`]: сдвиг в полосу
/// колонтитула делает раскладка, а не потребитель.
#[derive(Debug, Clone, Default)]
pub struct PageChrome {
    /// Элементы верхнего колонтитула (`w:headerReference`).
    pub header: Vec<LayoutItem>,
    /// Элементы нижнего колонтитула (`w:footerReference`).
    pub footer: Vec<LayoutItem>,
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
    let (mut layout, pass) = layout_pass(document, options, fonts, None);
    if !pass.saw_num_pages() {
        return Ok(layout);
    }

    // `NUMPAGES` — число страниц всего документа, и известно оно только после
    // первой раскладки: документ с этим полем раскладывается дважды. Второй
    // проход и возвращается — в нём поля несут вычисленные значения. Третий
    // нужен, только если подстановка сдвинула пагинацию: тогда число страниц
    // изменилось и его надо посчитать заново.
    let mut num_pages = page_count(&layout);
    for _ in 1..FIELD_PASSES {
        let (next, _) = layout_pass(document, options, fonts, Some(num_pages));
        let counted = page_count(&next);
        layout = next;
        if counted == num_pages {
            break;
        }
        num_pages = counted;
    }
    Ok(layout)
}

/// Предел проходов раскладки для документа с `NUMPAGES`.
///
/// Сходимость гарантирует счётчик страниц, а не число проходов: подстановка
/// номера в колонтитул пагинацию не двигает, и второй проход уже сходится.
/// Предел стоит на случай, когда номер в теле перевешивает строку туда-сюда.
const FIELD_PASSES: usize = 3;

/// Число страниц для `NUMPAGES`.
///
/// `w:pgNumType w:start` не учитывается: модель секции его не несёт, и `PAGE`
/// в раскладке — физический номер страницы.
fn page_count(layout: &PageLayout) -> u32 {
    u32::try_from(layout.pages.len()).unwrap_or(u32::MAX)
}

/// Один проход раскладки: `num_pages` — число страниц из прошлого прохода
/// (`None` — их ещё не считали).
///
/// Проход безошибочный: `LayoutError` остаётся в публичной сигнатуре ради
/// потребителей, а падать раскладке нечем — неизвестные поля и битые
/// колонтитулы она пропускает.
fn layout_pass(
    document: &Document,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    num_pages: Option<u32>,
) -> (PageLayout, FieldPass) {
    let mut pass = FieldPass::new(num_pages);
    let mut placed_floats: Vec<crate::layout::float::FloatElement> = Vec::new();
    // Анкоры, которым не хватило места на их странице: очередь переживает
    // смену страницы, в отличие от `placed_floats`, которые она очищает.
    let mut deferred_floats: Vec<DeferredFloat> = Vec::new();

    let first_section = section_at(document, 0);
    let mut paginator = Paginator::new(&first_section);
    // Колонтитулы мерятся до тела: их низ — верхняя граница полосы набора
    // каждой страницы. Мерятся тем же проходом полей, что и сама раскладка, —
    // колонтитулы с `PAGE`/`NUMPAGES` считаются одинаково там и здесь.
    let extents = header_extents(document, options, fonts, &mut pass);
    paginator.set_header_extents(extents, document.settings.even_and_odd_headers);
    let mut state = LayoutState::for_page(paginator.current_page(), &first_section);
    let mut cache = StyleCache::new();
    // Секция, в которой идёт раскладка: `sections[i]` описывает секцию, которую
    // закрывает i-й по порядку конец секции (`collect_sections` в `document.rs`).
    let mut section_index = 0usize;

    for block in &document.body.items {
        if let BlockItem::SectPr(_sect_pr) = block {
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
            continue;
        }

        pass.on_page(paginator.current_page().number);
        layout_block(
            document,
            block,
            options,
            fonts,
            &mut cache,
            &mut paginator,
            &mut placed_floats,
            &mut deferred_floats,
            &mut state,
            &mut pass,
        );

        // Абзац со встроенным `w:sectPr` закрывает секцию: он сам ещё
        // принадлежит ей, а следующая начинается после него.
        if let BlockItem::Paragraph(paragraph) = block {
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
    }

    let (pages, page_sections) = paginator.finish_with_sections();
    let chrome = layout_chrome(document, &pages, &page_sections, options, fonts, &mut pass);

    let total_height = pages.iter().map(|page| page.height).sum();

    (
        PageLayout {
            pages,
            chrome,
            total_height,
        },
        pass,
    )
}

/// Разложить один блок потока в текущую страницу пагинатора.
///
/// Один путь для тела документа и для колонтитулов: абзац и таблица колонтитула
/// раскладываются теми же средствами, что и в теле, — вместе с встроенными
/// рисунками, плавающими элементами и нумерацией абзацев.
///
/// Здесь же вычисляются поля `PAGE`/`NUMPAGES` (см. [`crate::layout::fields`]):
/// подстановка идёт до раскладки абзаца, потому что абзац без неё не содержит
/// знаков номера, а строка меряется уже с ними.
#[allow(clippy::too_many_arguments)]
fn layout_block(
    document: &Document,
    block: &BlockItem,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    cache: &mut StyleCache,
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    deferred_floats: &mut Vec<DeferredFloat>,
    state: &mut LayoutState,
    pass: &mut FieldPass,
) {
    match block {
        BlockItem::Paragraph(paragraph) => {
            // Курсор и колонка пагинатора — источник истины: состояние
            // возвращается к ним перед каждым абзацем, иначе элементы получают
            // координаты от прошлого блока.
            sync_frame(paginator, state);

            let resolved = resolve_paragraph(paragraph, pass);
            let paragraph = resolved.as_ref().unwrap_or(paragraph);

            let mut line_breaker = LineBreaker::new(fonts, FontId::default());
            let layout = layout_paragraph(
                document,
                paragraph,
                state,
                options,
                cache,
                &mut line_breaker,
            );

            let empty_line = empty_paragraph_line(&mut line_breaker, state.x);
            let lines: Vec<&Line> = if layout.lines.is_empty() {
                vec![&empty_line]
            } else {
                layout.lines.iter().collect()
            };

            let placement = place_lines(
                paginator,
                paragraph,
                &lines,
                &layout,
                &document.media,
                placed_floats,
                state,
            );

            // Плавающие рисунки встают после строк абзаца: их база —
            // фрагмент, в который абзац лёг.
            place_floats(
                paginator,
                &layout.anchors,
                placement,
                &document.media,
                placed_floats,
                deferred_floats,
                state,
            );
        }
        BlockItem::Table(table) => {
            place_table(
                table,
                &document.styles,
                fonts,
                paginator,
                placed_floats,
                state,
            );
        }
        // Секции в поток колонтитула не попадают, чужие блоки раскладке не видны.
        BlockItem::SectPr(_) | BlockItem::Unknown { .. } => {}
    }
}

/// Строка-метка абзаца без текста: такой абзац всё равно занимает строку.
///
/// Высота берётся у [`LineBreaker`] по кеглю по умолчанию: каскада, откуда
/// `layout_paragraph` берёт стиль знака, у движка под рукой нет.
fn empty_paragraph_line(line_breaker: &mut LineBreaker, x: f32) -> Line {
    Line {
        text: String::new(),
        x,
        width: 0.0,
        height: line_breaker.line_height_with_spacing(HalfPoint::new(DEFAULT_HALF_POINTS), None),
        color: None,
        // Текста в строке нет: рисовать нечего — рисовать будет запасное лицо.
        font_family: None,
        size: HalfPoint::new(DEFAULT_HALF_POINTS),
        bold: false,
        italic: false,
        underline: false,
    }
}

/// Положить строки абзаца и его встроенные рисунки на страницы.
///
/// Абзац переполняет страницу строкой, а не целиком: что не влезло — уходит
/// на следующую. Явный `w:br w:type="page"` закрывает страницу до строк
/// абзаца, а `w:keepLines` и висячие строки переносят его целиком.
///
/// Встроенные рисунки идут тем же потоком: у абзаца без текста рисунок и есть
/// строка — в Word метка абзаца идёт за картинкой, — а у абзаца с текстом
/// рисунки встают после строк (место внутри строки — отдельный долг, см.
/// [`LayoutItem::Image`]).
fn place_lines(
    paginator: &mut Paginator,
    paragraph: &Paragraph,
    lines: &[&Line],
    layout: &ParagraphLayout,
    media: &[MediaPart],
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) -> ParagraphPlacement {
    let images = collect_inline_images(paragraph);
    let images_first = layout.lines.is_empty() && !images.is_empty();
    // Строки разложены по краю той колонки, где абзац начался; продолжаясь в
    // соседней, они сдвигаются вместе с ней — ширина колонок секции одна и та
    // же, поэтому переносить строки заново не приходится.
    let paragraph_column_x = state.column_x;
    if images_first {
        place_inline_images(paginator, &images, media, placed_floats, state);
    }

    // Явный разрыв стоит до строк абзаца: знак абзаца Word оставляет
    // уже на новой странице.
    if has_page_break(paragraph) {
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::PageBreak);
        break_page(paginator, placed_floats, state);
    }

    // Явный разрыв колонки: знак абзаца Word оставляет уже в следующей колонке.
    if has_column_break(paragraph) {
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::ColumnBreak);
        break_frame(paginator, placed_floats, state);
    }

    // Сколько строк остаётся на текущей странице; хвост — на следующей.
    let kept = lines_kept_on_page(paginator, lines, layout);

    // Абзац начинает страницу: интервал `before` пришёлся бы на разрыв,
    // а строка и так стоит у верхнего поля — Word интервал не рисует.
    // Разрыв уже сделан или будет сделан перед первой строкой: явный
    // `w:br w:type="page"`, `w:pageBreakBefore`, перенос абзаца целиком
    // (`kept == 0`) или страница продолжения, на которой ещё ничего нет.
    // Первая страница документа — не разрыв: там `before` остаётся (так же
    // кладёт LibreOffice — эталон фикстуры `basic/paragraph_spacing`).
    let starts_page = has_page_break(paragraph)
        || has_column_break(paragraph)
        || kept == 0
        || layout.page_break_before
        || (paginator.current_y() <= paginator.content_top()
            && (paginator.current_page().number > 1 || state.column_index > 0));

    // Верх фрагмента абзаца на странице, где он закончится: у разорванного
    // абзаца `relativeFrom="paragraph"` отсчитывается от полосы набора той
    // страницы, а не от начала абзаца на предыдущей.
    let mut fragment_top = paginator.current_y();
    let mut page_number = paginator.current_page().number;

    for (index, line) in lines.iter().enumerate() {
        if index == kept {
            break_frame(paginator, placed_floats, state);
            fragment_top = state.y;
        }
        let first = index == 0;
        let last = index + 1 == lines.len();
        // Интервалы идут в те же единицы, что и `advance`: место занимает
        // не только строка, но и промежуток вокруг неё.
        let mut height = line.height;
        if first && !starts_page {
            height += layout.space_before;
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

        // Интервал после остаётся под последней строкой и входит в курсор,
        // но только пока помещается в полосу: упёршийся в нижнюю границу
        // интервал Word отбрасывает — на разрыве страницы его не рисуют.
        let after = if last && paginator.fits(layout.space_after) {
            paginator.advance(layout.space_after);
            layout.space_after
        } else {
            0.0
        };
        // Верх строки: курсор стоит за её нижней границей, а интервал перед
        // учтён в `height`. Он сдвигает первую строку, интервал после остаётся
        // под последней — из курсора его нужно вычесть.
        let top = paginator.current_y() - line.height - after;
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::Paragraph {
                node_id: paragraph.id,
                rect: Rect::new(
                    line.x + (state.column_x - paragraph_column_x),
                    top,
                    line.width,
                    line.height,
                ),
                text: line.text.clone(),
                line_height: line.height,
                color: line.color,
                font_family: line.font_family.clone(),
                size: line.size,
                bold: line.bold,
                italic: line.italic,
                underline: line.underline,
            });
    }

    if !images_first {
        place_inline_images(paginator, &images, media, placed_floats, state);
    }

    ParagraphPlacement {
        top: fragment_top,
        height: paginator.current_y() - fragment_top,
    }
}

/// Положить в поток встроенные рисунки абзаца: один прямоугольник за другим.
///
/// Рисунок занимает место в полосе набора, а не висит поверх текста: следующий
/// за ним абзац сдвигается вниз. Рисунок, не влезающий на текущую страницу,
/// переезжает на следующую — но только если на текущей уже что-то есть: выше
/// полосы набора он остаётся здесь, как блок в [`Paginator::advance`], иначе
/// пустая страница зациклила бы раскладку.
fn place_inline_images(
    paginator: &mut Paginator,
    images: &[&InlineImage],
    media: &[MediaPart],
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    for image in images {
        let width = emu_to_px(image.extent.cx);
        let height = emu_to_px(image.extent.cy);
        // Вырожденный `wp:extent` — рисовать нечего: нулевой прямоугольник
        // только сдвинул бы поток на месте.
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        if !paginator.fits(height) && paginator.current_y() > paginator.content_top() {
            break_frame(paginator, placed_floats, state);
        }
        let rect = paginator.next_rect(width, height);
        paginator.advance(height);
        paginator.current_page_mut().items.push(LayoutItem::Image {
            node_id: image.id,
            rect,
            image_id: media_id(media, image.part.as_deref()),
        });
    }
}

/// Собрать встроенные рисунки абзаца в порядке появления.
///
/// Обход тот же, что у анкоров (`layout::paragraph::collect_anchors`), но здесь
/// нужны сами картинки: у `wp:inline` анкора нет, а место в потоке — есть.
fn collect_inline_images(paragraph: &Paragraph) -> Vec<&InlineImage> {
    let mut images = Vec::new();
    collect_images_in(&paragraph.runs, &mut images);
    images
}

/// Обойти `Inline` и собрать встроенные картинки: вложенные списки (гиперссылка,
/// поле) обходятся тем же порядком, что и текст в них.
fn collect_images_in<'a>(inlines: &'a [Inline], images: &mut Vec<&'a InlineImage>) {
    for inline in inlines {
        match inline {
            Inline::Drawing(drawing) => push_inline_image(drawing, images),
            Inline::Run(run) => {
                for content in &run.content {
                    if let RunContent::Drawing(drawing) = content {
                        push_inline_image(drawing, images);
                    }
                }
            }
            Inline::Hyperlink(hyperlink) => collect_images_in(&hyperlink.runs, images),
            Inline::Field(field) => collect_images_in(&field.result, images),
            _ => {}
        }
    }
}

/// Добавить встроенную картинку `w:drawing`; плавающая идёт другим путём.
fn push_inline_image<'a>(drawing: &'a InlineOrAnchor, images: &mut Vec<&'a InlineImage>) {
    if let Some(inline) = &drawing.inline {
        images.push(inline);
    }
}

/// Идентификатор байтов медиачасти в реестре документа.
///
/// `None` — часть не прочитана (внешняя цель, отсутствующая или битая часть,
/// часть выше предела): кадр нарисует габариты плейсхолдером.
fn media_id(media: &[MediaPart], part: Option<&str>) -> Option<u32> {
    let part = part?;
    media
        .iter()
        .find(|entry| entry.part == part)
        .map(|entry| entry.id)
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
    media: &[MediaPart],
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
        for mut float in placed_floats[placed_before..].iter().cloned() {
            // Байты ищутся здесь, а не в `layout_float`: анкор несёт имя части,
            // но не документ, в котором лежит медиареестр.
            if let FloatKind::Image { part, image_id, .. } = &mut float.kind {
                *image_id = media_id(media, part.as_deref());
            }
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

/// Положить таблицу: строки режет полоса набора, фрагменты ложатся по страницам.
///
/// Разрез строки, повтор `w:tblHeader` и `w:cantSplit` посчитаны в
/// [`layout_table_fragments`]: здесь остаётся только разложить готовые фрагменты
/// по страницам. Фрагмент уже несёт координаты той полосы, на которую лёг, —
/// курсор лишь продвигается за ним.
fn place_table(
    table: &Table,
    styles: &StyleTable,
    fonts: &mut FontRegistry,
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    // Таблица ложится в текущую колонку: её левый край и ширину задаёт она.
    sync_frame(paginator, state);
    let frame = compute_table_frame(table, state.column_x, state.column_width);
    let first = PageBand {
        top: paginator.current_y(),
        height: (paginator.content_bottom() - paginator.current_y()).max(0.0),
    };
    // Геометрия продолжений одна на всю таблицу: перенос не меняет секцию,
    // а смена секции закрывает таблицу разрывом потока.
    let next = PageBand {
        top: state.content_top,
        height: (state.content_bottom - state.content_top).max(0.0),
    };
    let fragments = layout_table_fragments(table, &frame, first, next, fonts, styles);

    for fragment in fragments {
        // Пустая страница — предел: фрагмент не влез в неё, но новых страниц
        // под него не будет, и разрыв оставил бы лист с одной таблицей за полем.
        //
        // Продолжение таблицы переходит на новую страницу, а не в соседнюю
        // колонку: ячейки фрагмента разложены по левому краю той колонки, в
        // которой таблица начиналась, и сдвиг одной рамки увёл бы их от неё.
        if fragment.starts_new_page && paginator.current_y() > paginator.content_top() {
            break_page(paginator, placed_floats, state);
        }
        paginator.advance(fragment.height);
        paginator
            .current_page_mut()
            .items
            .push(table_item(table, &frame, fragment));
    }
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
    // Переполнение в многоколоночной секции ведёт в соседнюю колонку той же
    // страницы, а не сразу на новую. Пустая колонка — предел: элемент выше
    // полосы набора остаётся в ней и выходит за нижнюю границу, иначе разрыв
    // зациклил бы раскладку. Явный разрыв страницы колонку не выбирает.
    if !page_break
        && !paginator.fits(height)
        && paginator.current_y() > paginator.content_top()
        && paginator.next_column()
    {
        placed_floats.clear();
        sync_frame(paginator, state);
    }
    if paginator.add_item(height, page_break) {
        return;
    }
    reset_page_state(paginator, placed_floats, state);
    if !paginator.add_item(height, false) {
        paginator.advance(height);
    }
}

/// Продолжить поток: соседняя колонка той же страницы, а если её нет — новая.
///
/// Так ведёт себя и переполнение полосы набора, и `w:br w:type="column"`:
/// колонка заполняется до низа, затем идёт следующая, и только после последней
/// начинается новая страница с первой колонки.
fn break_frame(
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    if paginator.next_column() {
        // Обтекание принадлежит колонке: рисунок из прежней ширину строк в
        // новой не режет — там он стоит в стороне от её текста.
        placed_floats.clear();
        sync_frame(paginator, state);
        return;
    }
    break_page(paginator, placed_floats, state);
}

/// Начать новую страницу: обтекание и координаты живут в её пределах.
///
/// Новая страница всегда открывается первой колонкой: так ведут себя явный
/// `w:br w:type="page"`, `w:pageBreakBefore` и разрыв секции.
fn break_page(
    paginator: &mut Paginator,
    placed_floats: &mut Vec<FloatElement>,
    state: &mut LayoutState,
) {
    paginator.push_page();
    reset_page_state(paginator, placed_floats, state);
}

/// Ширины колонок полосы набора шириной `band` и промежуток между ними.
///
/// Равные колонки (`w:equalWidth`, умолчание Word) делят полосу за вычетом
/// промежутков между ними. Явные `w:col` (`w:equalWidth="0"`) несут
/// собственные ширины: подгонять их сумму под полосу Word не берётся, и
/// раскладка тоже — остаток полосы остаётся пустым. Промежутки берутся у
/// секции (`w:space`): индивидуальные `w:space` колонок уходят в исключение.
// Колонок в секции единицы: точность f32 для их счёта заведомо достаточна.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn column_geometry(section: &Section, band: f32) -> (Vec<f32>, f32) {
    let columns = &section.columns;
    let gap = twips_to_px(columns.space);
    let count = columns.count.max(1) as usize;
    if count > 1 && !columns.equal_width && columns.defs.len() == count {
        let widths = columns
            .defs
            .iter()
            .map(|def| twips_to_px(def.width))
            .collect();
        return (widths, gap);
    }
    if count > 1 {
        let width = ((band - gap * (count - 1) as f32) / count as f32).max(0.0);
        return (vec![width; count], gap);
    }
    (vec![band], gap)
}

/// Подтянуть координаты состояния к текущей колонке пагинатора.
///
/// Колонку выбирает пагинатор (он же ведёт курсор), состояние лишь несёт её
/// координаты раскладывающим функциям: строки, таблицы и рисунки берут левый
/// край и ширину из `state`.
fn sync_frame(paginator: &Paginator, state: &mut LayoutState) {
    state.column_index = paginator.column_index();
    state.column_x = paginator.content_left();
    state.column_width = paginator.content_width();
    state.x = state.column_x;
    state.y = paginator.current_y();
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
    // Разрыв всегда открывает первую колонку — состояние берёт её у пагинатора.
    sync_frame(paginator, state);
}

/// Сколько строк абзаца остаётся на текущей странице.
///
/// Абзац, влезающий целиком, отдаёт все строки; `0` означает, что он уходит
/// на следующую страницу весь — так ведут себя `w:keepLines` и висячие строки:
/// ни одна строка при этом не остаётся на прежней странице.
fn lines_kept_on_page(paginator: &Paginator, lines: &[&Line], layout: &ParagraphLayout) -> usize {
    let mut height = layout.space_before;
    let mut fit = 0;
    for line in lines {
        height += line.height;
        // `space_after` в проверку не входит: упёршийся в нижнюю границу
        // интервал Word отбрасывает, а не переносит строку на новую страницу.
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

/// Есть ли в абзаце явный разрыв колонки (`w:br w:type="column"`).
fn has_column_break(paragraph: &Paragraph) -> bool {
    paragraph.runs.iter().any(|inline| match inline {
        Inline::Break(BreakKind::Column) => true,
        Inline::Run(run) => run
            .content
            .iter()
            .any(|content| matches!(content, RunContent::Break(BreakKind::Column))),
        _ => false,
    })
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
        paginator.push_page_with_section(section, index);
        reset_page_state(paginator, placed_floats, state);
    } else {
        paginator.set_section(section, index);
        *state = LayoutState::for_page(paginator.current_page(), section);
    }
}

/// Полоса, в которой печатается колонтитул.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChromeBand {
    /// Верхний колонтитул: от `w:header` вниз, содержимое растёт от этой линии.
    Header,
    /// Нижний колонтитул: от `w:footer` вверх, содержимое прижато к нижнему краю.
    Footer,
}

/// Разложить колонтитулы всех страниц документа.
///
/// `page_sections[i]` — индекс секции страницы `pages[i]` (см.
/// [`Paginator::finish_with_sections`]): по самой странице первую страницу
/// секции и её чётность не восстановить — страница знает только общий номер.
fn layout_chrome(
    document: &Document,
    pages: &[Page],
    page_sections: &[usize],
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    pass: &mut FieldPass,
) -> Vec<PageChrome> {
    let even_and_odd = document.settings.even_and_odd_headers;
    let mut chrome = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let section_index = page_sections.get(index).copied().unwrap_or_default();
        let section = section_at(document, section_index);
        // Страница начинает секцию, если предыдущая принадлежит другой.
        let first_in_section = index == 0 || page_sections.get(index - 1) != Some(&section_index);
        let even_page = even_and_odd && page.number.is_multiple_of(2);
        // `PAGE` в колонтитуле — номер той страницы, на которой он напечатан:
        // полоса колонтитула ведёт свой пагинатор с первой страницы, и без
        // этого номера поле показало бы единицу на каждой странице.
        pass.on_page(page.number);

        let header = chrome_part(&section, ChromeBand::Header, first_in_section, even_page);
        let header = layout_chrome_part(
            document,
            header,
            &section,
            page,
            ChromeBand::Header,
            options,
            fonts,
            pass,
        );
        let footer = chrome_part(&section, ChromeBand::Footer, first_in_section, even_page);
        let footer = layout_chrome_part(
            document,
            footer,
            &section,
            page,
            ChromeBand::Footer,
            options,
            fonts,
            pass,
        );
        chrome.push(PageChrome { header, footer });
    }
    chrome
}

/// Какой колонтитул печатается: выбор между `default`, `first` и `even`.
///
/// `first` — на первой странице секции и только при `w:titlePg`; `even` —
/// на чётных страницах и только при `w:evenAndOddHeaders`. Флаг сам по себе
/// колонтитула не создаёт: отсутствующая часть откатывается к `default`, как
/// это делает Word (`w:titlePg` без части `first` оставляет первой странице
/// обычный колонтитул).
fn chrome_part(
    section: &Section,
    band: ChromeBand,
    first_in_section: bool,
    even_page: bool,
) -> Option<&PartRef> {
    let (default, first, even) = match band {
        ChromeBand::Header => (
            section.header_default.as_ref(),
            section.header_first.as_ref(),
            section.header_even.as_ref(),
        ),
        ChromeBand::Footer => (
            section.footer_default.as_ref(),
            section.footer_first.as_ref(),
            section.footer_even.as_ref(),
        ),
    };

    if first_in_section && section.title_pg {
        first.or(default)
    } else if even_page {
        even.or(default)
    } else {
        default
    }
}

/// Разложить один колонтитул в координатах страницы.
///
/// Содержимое колонтитула раскладывается тем же [`layout_block`], что и тело:
/// разложенная полоса ([`layout_chrome_items`]) сдвигается на своё место.
#[allow(clippy::too_many_arguments)]
fn layout_chrome_part(
    document: &Document,
    part: Option<&PartRef>,
    section: &Section,
    page: &Page,
    band: ChromeBand,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    pass: &mut FieldPass,
) -> Vec<LayoutItem> {
    let mut items = layout_chrome_items(document, part, section, band, options, fonts, pass).items;
    if items.is_empty() {
        return items;
    }

    let dy = match band {
        // Верхний колонтитул печатается от расстояния `w:header`.
        ChromeBand::Header => page.margins.header,
        // Нижний прижат к линии `w:footer`: на неё приходится его нижний край.
        ChromeBand::Footer => {
            let bottom = items.iter().map(item_bottom).fold(0.0, f32::max);
            page.height - page.margins.footer - bottom
        }
    };
    translate_items(&mut items, dy);
    items
}

/// Разложенная полоса колонтитула.
struct ChromeBandLayout {
    /// Элементы в координатах полосы: `(0, 0)` — её верхний левый угол.
    items: Vec<LayoutItem>,
    /// Высота занятого полосой места — насколько содержимое колонтитула
    /// отстоит от её верха. Это не то же, что низ последнего элемента: за
    /// последним абзацем остаётся его отбивка (`w:spacing/@w:after`), и Word
    /// с `LibreOffice` её считают — тело отодвигается и на неё тоже.
    height: f32,
}

/// Разложить один колонтитул в координатах его полосы.
///
/// Полоса — вся страница с полями тела, снятыми: иначе колонтитул не поместился
/// бы в промежуток до полосы набора. Начало координат — верхний левый угол
/// полосы; на страницу её кладёт [`layout_chrome_part`].
fn layout_chrome_items(
    document: &Document,
    part: Option<&PartRef>,
    section: &Section,
    band: ChromeBand,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    pass: &mut FieldPass,
) -> ChromeBandLayout {
    let empty = ChromeBandLayout {
        items: Vec::new(),
        height: 0.0,
    };
    let Some(part) = part else {
        return empty;
    };
    // Части нет в пакете (битая ссылка) — страница без колонтитула, а не паника.
    let content = match band {
        ChromeBand::Header => document.headers.get(&part.target),
        ChromeBand::Footer => document.footers.get(&part.target),
    };
    let Some(content) = content else {
        return empty;
    };

    let band_section = chrome_band_section(section);
    let mut paginator = Paginator::new(&band_section);
    let mut state = LayoutState::for_page(paginator.current_page(), &band_section);
    let mut cache = StyleCache::new();
    let mut placed_floats = Vec::new();
    let mut deferred_floats = Vec::new();
    let mut height = 0.0;
    for block in &content.body.items {
        layout_block(
            document,
            block,
            options,
            fonts,
            &mut cache,
            &mut paginator,
            &mut placed_floats,
            &mut deferred_floats,
            &mut state,
            pass,
        );
        // Курсор — высота содержимого вместе с отбивкой за последним блоком.
        // Берётся только с первой страницы полосы: колонтитул выше страницы —
        // вырожденный случай, и продолжение на второй странице его места не
        // уменьшает.
        if paginator.current_page().number == 1 {
            height = paginator.current_y();
        }
    }

    // Колонтитул выше страницы — вырожденный случай, которого Word не создаёт:
    // берём то, что легло на первую страницу полосы.
    let (band_pages, _) = paginator.finish_with_sections();
    let mut items = band_pages
        .into_iter()
        .next()
        .map_or_else(Vec::new, |band_page| band_page.items);
    // Разрыв страницы в колонтитуле бессмыслен: он не начинает новую страницу,
    // а лишь сдвинул бы содержимое.
    items.retain(|item| !matches!(item, LayoutItem::PageBreak | LayoutItem::ColumnBreak));
    ChromeBandLayout { items, height }
}

/// Секция полосы колонтитула: поля тела сняты, колонок нет.
///
/// Колонтитул печатается поверх страницы целиком: колонки секции его не делят,
/// иначе он встал бы в первую из них.
fn chrome_band_section(section: &Section) -> Section {
    let mut band_section = section.clone();
    band_section.margins.top = Twips::new(0);
    band_section.margins.bottom = Twips::new(0);
    band_section.margins.header = Some(Twips::new(0));
    band_section.margins.footer = Some(Twips::new(0));
    band_section.columns.count = 1;
    band_section
}

/// Низы верхних колонтитулов по секциям: ниже них начинается тело.
///
/// Колонтитул, которому мало верхнего поля, сдвигает полосу набора вниз — так
/// делают Word и `LibreOffice`, и без этого первое содержимое страницы легло бы
/// поверх строк колонтитула. Низ считается до раскладки тела: от него зависит
/// верхняя граница полосы набора каждой страницы ([`Paginator`]), а сама полоса
/// колонтитула от тела не зависит. Вариант колонтитула выбирается теми же
/// флагами, что и в [`chrome_part`], — `first` на первой странице секции при
/// `w:titlePg`, `even` на чётной при `w:evenAndOddHeaders`.
fn header_extents(
    document: &Document,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    pass: &mut FieldPass,
) -> Vec<HeaderExtent> {
    document
        .body
        .sections
        .iter()
        .map(|section| HeaderExtent {
            default: header_extent(document, section, false, false, options, fonts, pass),
            first: header_extent(document, section, true, false, options, fonts, pass),
            even: header_extent(document, section, false, true, options, fonts, pass),
        })
        .collect()
}

/// Низ полосы одного верхнего колонтитула в координатах страницы.
///
/// `0.0` — колонтитула нет: тело начинается от верхнего поля. Номер страницы
/// для полей (`PAGE`) при измерении — первый: полоса набора от его подстановки
/// не зависит, а до раскладки тела другого номера у колонтитула и нет.
fn header_extent(
    document: &Document,
    section: &Section,
    first_in_section: bool,
    even_page: bool,
    options: &LayoutOptions,
    fonts: &mut FontRegistry,
    pass: &mut FieldPass,
) -> f32 {
    let part = chrome_part(section, ChromeBand::Header, first_in_section, even_page);
    let band = layout_chrome_items(
        document,
        part,
        section,
        ChromeBand::Header,
        options,
        fonts,
        pass,
    );
    if band.items.is_empty() {
        return 0.0;
    }
    let bottom = band
        .items
        .iter()
        .map(item_bottom)
        .fold(band.height, f32::max);
    // `w:header` — расстояние до полосы, её низ отстоит от начала страницы
    // на эту величину плюс высоту содержимого.
    MarginsLayout::from_margins(&section.margins).header + bottom
}

/// Нижняя граница элемента в координатах полосы колонтитула.
fn item_bottom(item: &LayoutItem) -> f32 {
    match item {
        LayoutItem::Paragraph { rect, .. }
        | LayoutItem::Image { rect, .. }
        | LayoutItem::Table { rect, .. } => rect.y + rect.height,
        LayoutItem::Float(float) => float.rect.y + float.rect.height,
        LayoutItem::PageBreak | LayoutItem::ColumnBreak => 0.0,
    }
}

/// Сдвинуть элементы по вертикали: из полосы колонтитула — на страницу.
///
/// Сдвиг рекурсивный: содержимое ячейки таблицы колонтитула — такие же элементы.
fn translate_items(items: &mut [LayoutItem], dy: f32) {
    for item in items {
        match item {
            LayoutItem::Paragraph { rect, .. } | LayoutItem::Image { rect, .. } => rect.y += dy,
            LayoutItem::Float(float) => float.rect.y += dy,
            LayoutItem::Table {
                rect, cells, rows, ..
            } => {
                rect.y += dy;
                for cell in cells.iter_mut() {
                    cell.rect.y += dy;
                    translate_items(&mut cell.content, dy);
                }
                for row in rows.iter_mut() {
                    for cell in &mut row.cells {
                        cell.rect.y += dy;
                        translate_items(&mut cell.content, dy);
                    }
                }
            }
            LayoutItem::PageBreak | LayoutItem::ColumnBreak => {}
        }
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
    /// Состояние на начало полосы набора: её первая колонка внутри полей.
    ///
    /// Колонку дальше ведёт пагинатор и отдаёт её координаты состоянию при
    /// переходе (см. `sync_frame`); здесь состояние получает первую — разрыв
    /// страницы всегда открывает именно её. Поля и размеры берутся из уже
    /// собранной страницы, чтобы состояние и страница не разъезжались.
    #[must_use]
    pub fn for_page(page: &Page, section: &Section) -> Self {
        let band = page.width - page.margins.left - page.margins.right;
        let (widths, _) = column_geometry(section, band);
        let column_width = widths.first().copied().unwrap_or(band);
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

    /// Состояние абзаца внутри ячейки таблицы: одна колонка шириной `width`.
    ///
    /// Абзац ячейки раскладывается тем же [`layout_paragraph`], что и абзац тела,
    /// и от состояния ему нужна только полоса набора — левая граница и ширина.
    /// Вертикаль строк ведёт таблица: координаты, которые абзац вернёт, читаются
    /// как смещения от содержимого ячейки, а не от страницы.
    ///
    /// [`layout_paragraph`]: super::paragraph::layout_paragraph
    #[must_use]
    pub(crate) fn for_cell(width: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            column_index: 0,
            column_width: width,
            column_x: 0.0,
            content_top: 0.0,
            content_bottom: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::raw::{ParagraphSpacing, Toggle};
    use crate::model::{
        Body, BreakKind, ColumnDef, Columns, Extent, Field, FieldKind, HeaderFooter, Inline,
        InlineImage, InlineOrAnchor, Metadata, NumberingTable, Paragraph, PartRef, PositionH,
        PositionV, RelFromH, RelFromV, Relationships, Run, RunContent, SectionType, Settings,
        StyleTable, WrapKind,
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
            media: Vec::new(),
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

    /// Одна строка 19.2 px: колонка такой высоты вмещает ровно строку.
    const ONE_LINE_PAGE_TWIPS: i32 = 288;

    /// Промежуток между колонками тестов: 0.5 дюйма, как у Word по умолчанию.
    const COLUMN_GAP_TWIPS: i32 = 720;

    /// Секция с равными колонками: A4 без полей, высота — `height_twips`.
    fn section_with_columns(count: u32, height_twips: i32) -> Section {
        let mut section = section_without_margins(height_twips);
        section.columns = Columns {
            count,
            space: Twips::new(COLUMN_GAP_TWIPS),
            equal_width: true,
            separator: false,
            defs: Vec::new(),
        };
        section.properties.columns = section.columns.clone();
        section
    }

    /// Прямоугольники абзацев страницы в порядке отрисовки.
    fn paragraph_rects(page: &Page) -> Vec<Rect> {
        page.items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    /// Левый край второй колонки секции из двух равных колонок.
    fn second_column_x(page: &Page) -> f32 {
        let gap = twips_to_px(Twips::new(COLUMN_GAP_TWIPS));
        page.margins.left + (page.width - page.margins.left - page.margins.right - gap) / 2.0 + gap
    }

    /// Абзац с явным разрывом колонки (`w:br w:type="column"`).
    fn paragraph_with_column_break(id: u64) -> Paragraph {
        let mut paragraph = paragraph(id, "after the column break");
        paragraph.runs.push(Inline::Break(BreakKind::Column));
        paragraph
    }

    /// Абзац с явным разрывом страницы (`w:br w:type="page"`).
    fn paragraph_with_page_break(id: u64) -> Paragraph {
        let mut paragraph = paragraph(id, "after the page break");
        paragraph.runs.push(Inline::Run(Run {
            id: NodeId::new(id + 2),
            content: vec![RunContent::Break(BreakKind::Page)],
            ..Run::default()
        }));
        paragraph
    }

    /// Блокер Спринта 10: абзацы заполняют первую колонку, затем вторую той же
    /// страницы — вторая колонка не открывает новую страницу.
    #[allow(clippy::float_cmp)]
    #[test]
    fn columns_fill_the_first_column_then_the_second_on_one_page() {
        let document = document_in_section(section_with_columns(2, ONE_LINE_PAGE_TWIPS), 2);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 1, "обе колонки — на одной странице");
        let page = &layout.pages[0];
        let rects = paragraph_rects(page);
        assert_eq!(rects.len(), 2, "по абзацу в колонку");
        assert_eq!(
            rects[0].x, page.margins.left,
            "первый абзац — в первой колонке"
        );
        assert_eq!(
            rects[1].x,
            second_column_x(page),
            "второй абзац ушёл во вторую колонку, а не на новую страницу"
        );
        assert_eq!(rects[1].y, page.margins.top, "колонка начинается сверху");
    }

    /// Текст сверх всех колонок страницы начинается на новой странице с первой
    /// колонки: её и открывает разрыв.
    #[allow(clippy::float_cmp)]
    #[test]
    fn text_beyond_the_last_column_starts_a_new_page() {
        let document = document_in_section(section_with_columns(2, ONE_LINE_PAGE_TWIPS), 3);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(
            layout.pages.len(),
            2,
            "третьему абзацу нужна вторая страница"
        );
        assert_eq!(paragraph_rects(&layout.pages[0]).len(), 2, "колонки заняты");
        let second = paragraph_rects(&layout.pages[1]);
        assert_eq!(second.len(), 1);
        assert_eq!(
            second[0].x, layout.pages[1].margins.left,
            "новая страница начинается с первой колонки"
        );
        assert_eq!(second[0].y, layout.pages[1].margins.top);
    }

    /// `w:br w:type="column"` переводит текст в соседнюю колонку той же
    /// страницы и оставляет отметку разрыва в потоке.
    #[allow(clippy::float_cmp)]
    #[test]
    fn an_explicit_column_break_moves_to_the_next_column() {
        let items = vec![
            BlockItem::Paragraph(paragraph(1, "left column")),
            BlockItem::Paragraph(paragraph_with_column_break(3)),
        ];
        let document = document_with(items, vec![section_with_columns(2, TWO_LINE_PAGE_TWIPS)]);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(
            layout.pages.len(),
            1,
            "разрыв колонки страницу не открывает"
        );
        let page = &layout.pages[0];
        assert!(
            page.items
                .iter()
                .any(|item| matches!(item, LayoutItem::ColumnBreak)),
            "разрыв колонки отмечен в потоке"
        );
        let rects = paragraph_rects(page);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].x, page.margins.left, "текст до разрыва — слева");
        assert_eq!(
            rects[1].x,
            second_column_x(page),
            "текст после разрыва — во второй колонке"
        );
    }

    /// Разрыв колонки в последней колонке открывает новую страницу.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_column_break_in_the_last_column_starts_a_new_page() {
        let items = vec![
            BlockItem::Paragraph(paragraph(1, "left column")),
            BlockItem::Paragraph(paragraph_with_column_break(3)),
            BlockItem::Paragraph(paragraph(5, "next page")),
        ];
        let document = document_with(items, vec![section_with_columns(2, ONE_LINE_PAGE_TWIPS)]);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 2, "колонки кончились — нужна страница");
        let second = paragraph_rects(&layout.pages[1]);
        assert_eq!(second.len(), 1);
        assert_eq!(
            second[0].x, layout.pages[1].margins.left,
            "новая страница начинается с первой колонки"
        );
    }

    /// Явный разрыв страницы открывает первую колонку новой страницы, а не
    /// соседнюю: колонка при нём не выбирается.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_page_break_starts_the_first_column_of_a_new_page() {
        let items = vec![
            BlockItem::Paragraph(paragraph(1, "first page")),
            BlockItem::Paragraph(paragraph_with_page_break(3)),
        ];
        let document = document_with(items, vec![section_with_columns(2, TWO_LINE_PAGE_TWIPS)]);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 2, "разрыв страницы страницу открывает");
        let second = paragraph_rects(&layout.pages[1]);
        assert_eq!(second.len(), 1);
        assert_eq!(
            second[0].x, layout.pages[1].margins.left,
            "разрыв страницы ведёт в первую колонку"
        );
        assert_eq!(second[0].y, layout.pages[1].margins.top);
    }

    /// Одноколоночная секция от многоколоночной раскладки не меняется: полоса —
    /// вся страница, обе строки на ней.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_single_column_section_keeps_the_whole_band() {
        let document = document_in_section(section_with_columns(1, TWO_LINE_PAGE_TWIPS), 2);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 1);
        let page = &layout.pages[0];
        let rects = paragraph_rects(page);
        assert_eq!(rects.len(), 2);
        assert!(
            rects.iter().all(|rect| rect.x == page.margins.left),
            "одна полоса по левому полю: {rects:?}"
        );
    }

    /// `w:equalWidth="0"`: ширины колонок берутся из `w:col` как записаны, а
    /// промежуток — из `w:space` секции.
    #[allow(clippy::float_cmp)]
    #[test]
    fn explicit_column_defs_set_each_column_width() {
        let mut section = section_with_columns(2, ONE_LINE_PAGE_TWIPS);
        let gap = Twips::new(COLUMN_GAP_TWIPS);
        section.columns.equal_width = false;
        section.columns.defs = vec![
            ColumnDef {
                width: Twips::new(2000),
                space: gap,
            },
            ColumnDef {
                width: Twips::new(4000),
                space: gap,
            },
        ];
        section.properties.columns = section.columns.clone();
        let document = document_in_section(section, 2);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        let page = &layout.pages[0];
        let rects = paragraph_rects(page);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].x, page.margins.left);
        assert_eq!(
            rects[1].x,
            page.margins.left + twips_to_px(Twips::new(2000)) + twips_to_px(gap),
            "вторая колонка — за первой ровно на её ширину и промежуток"
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
    /// Считается так же, как `lines_kept_on_page`: в высоту входит `space_before`
    /// первой строки, а `space_after` — нет (Word отбрасывает его на разрыве).
    /// Правила переноса (висячие строки, `w:keepLines`) сюда не входят — их
    /// проверяют тесты ниже.
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

    /// `space_after`, упёршийся в нижнюю границу, не переносит строку на
    /// следующую страницу: Word отбрасывает интервал на разрыве, и строка
    /// остаётся на своей странице, ровно по нижнему краю полосы.
    #[test]
    fn the_space_after_at_the_content_bottom_is_dropped() {
        // Полоса ровно в две строки: заполнитель оставляет под последнюю
        // строку абзаца ровно одну строку, а интервалу в 24 pt (480 twips)
        // места уже нет.
        let document = document_with(
            vec![
                BlockItem::Paragraph(paragraph(1, "filler")),
                BlockItem::Paragraph(paragraph_with_spacing(3, "last", 0, 480)),
                BlockItem::Paragraph(paragraph(5, "next")),
            ],
            vec![section_without_margins(TWO_LINE_PAGE_TWIPS)],
        );
        let layout = layout_of(&document);

        assert_eq!(layout.pages.len(), 2, "третья строка не помещается");
        assert_eq!(
            paragraphs_on(&layout.pages[0]),
            2,
            "строка с интервалом после остаётся на первой странице"
        );
        let rects = line_rects(&layout.pages[0]);
        let bottom = layout.pages[0].height - layout.pages[0].margins.bottom;
        assert!(
            ((rects[1].y + rects[1].height) - bottom).abs() < 0.01,
            "строка стоит ровно по нижнему краю полосы: {} при границе {bottom}",
            rects[1].y + rects[1].height
        );
        assert_eq!(
            paragraphs_on(&layout.pages[1]),
            1,
            "следующий абзац — на второй странице"
        );
    }

    /// Интервал после, помещающийся в полосу, остаётся: его отбрасывают
    /// только на разрыве, и следующую строку он вытесняет заслуженно.
    #[test]
    fn the_space_after_that_fits_still_pushes_the_next_line() {
        // Полоса в три строки (3 × 19.2 px = 864 twips): после второй строки
        // интервал в 8 pt (160 twips) ещё помещается, следующей строке места
        // уже нет — она уходит на вторую страницу.
        let document = document_with(
            vec![
                BlockItem::Paragraph(paragraph(1, "filler")),
                BlockItem::Paragraph(paragraph_with_spacing(3, "last", 0, 160)),
                BlockItem::Paragraph(paragraph(5, "next")),
            ],
            vec![section_without_margins(864)],
        );
        let layout = layout_of(&document);

        assert_eq!(layout.pages.len(), 2, "интервал вытесняет третью строку");
        assert_eq!(paragraphs_on(&layout.pages[0]), 2);
        assert_eq!(paragraphs_on(&layout.pages[1]), 1);
    }

    /// Абзац, начавший страницу переносом, интервал `before` не добавляет:
    /// он пришёлся бы на разрыв, а строка и так стоит у верхнего поля.
    #[test]
    fn a_paragraph_moved_to_the_next_page_drops_its_space_before() {
        // Полоса в две строки: заполнитель оставляет ровно строку, а перед
        // абзацем стоит интервал в 24 pt (480 twips) — он не помещается,
        // и абзац уходит на вторую страницу целиком.
        let document = document_with(
            vec![
                BlockItem::Paragraph(paragraph(1, "filler")),
                BlockItem::Paragraph(paragraph_with_spacing(3, "second", 480, 0)),
            ],
            vec![section_without_margins(TWO_LINE_PAGE_TWIPS)],
        );
        let layout = layout_of(&document);

        assert_eq!(
            layout.pages.len(),
            2,
            "абзац не помещается на первой странице"
        );
        assert_eq!(paragraphs_on(&layout.pages[0]), 1);
        let rects = line_rects(&layout.pages[1]);
        assert!(
            (rects[0].y - layout.pages[1].margins.top).abs() < 0.01,
            "строка — у верхнего поля, без интервала перед: {}",
            rects[0].y
        );
    }

    /// Абзац, открывающий пустую страницу разрыва секции, тоже не добавляет
    /// `before` — его интервал пришёлся бы на разрыв.
    #[test]
    fn a_paragraph_opening_a_fresh_page_drops_its_space_before() {
        let mut page = section_without_margins(TWO_LINE_PAGE_TWIPS);
        page.properties.section_type = Some(SectionType::NextPage);
        let mut closing = paragraph(1, "first section");
        closing.section_break = Some(Box::new(page.properties.clone()));
        let document = document_with(
            vec![
                BlockItem::Paragraph(closing),
                BlockItem::Paragraph(paragraph_with_spacing(3, "second", 480, 0)),
                BlockItem::SectPr(page.properties.clone()),
            ],
            vec![page.clone(), page],
        );
        let layout = layout_of(&document);

        assert_eq!(
            layout.pages.len(),
            2,
            "разрыв секции `NextPage` начинает вторую страницу"
        );
        let rects = line_rects(&layout.pages[1]);
        assert!(
            (rects[0].y - layout.pages[1].margins.top).abs() < 0.01,
            "первая строка страницы — у верхнего поля, без интервала перед: {}",
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

    /// Синтетическая таблица: одна колонка, `rows` однострочных строк.
    ///
    /// `cant_split` — `w:cantSplit` всех строк: такая строка не режется
    /// разрывом страницы, а уходит на следующую целиком.
    fn synthetic_table(id: u64, rows: usize, cant_split: bool) -> Table {
        use crate::model::CellVAlign;
        use crate::{Cell, CellBorders, CellMargins, GridCol, Row, TableLayout, TableLook};

        let rows = (0..rows)
            .map(|index| {
                // Три идентификатора на строку: строка, ячейка, абзац.
                let base = id + 10 + u64::try_from(index).unwrap_or(u64::MAX) * 3;
                Row {
                    id: NodeId::new(base),
                    cells: vec![Cell {
                        id: NodeId::new(base + 1),
                        grid_span: 1,
                        v_merge: None,
                        width: None,
                        margins: CellMargins::default(),
                        v_align: CellVAlign::Top,
                        borders: Box::new(CellBorders::default()),
                        shading: None,
                        items: vec![BlockItem::Paragraph(paragraph(
                            base + 2,
                            &format!("row {index}"),
                        ))],
                    }],
                    height: None,
                    cant_split,
                    header: false,
                }
            })
            .collect();

        Table {
            id: NodeId::new(id),
            style_ref: None,
            grid: vec![GridCol {
                width: Twips::new(6000),
            }],
            rows,
            layout: TableLayout::Autofit,
            width: None,
            borders: crate::TableBorders::default(),
            look: TableLook::default(),
            jc: None,
            indent: None,
            cell_margins: CellMargins::default(),
        }
    }

    /// Первый элемент-таблица страницы: прямоугольник, строки и ячейки фрагмента.
    fn table_fragment(page: &Page) -> Option<(&Rect, &[TableRowLayout], &[TableCellLayout])> {
        page.items.iter().find_map(|item| match item {
            LayoutItem::Table {
                rect, rows, cells, ..
            } => Some((rect, rows.as_slice(), cells.as_slice())),
            _ => None,
        })
    }

    /// Строка, которой остатка страницы не хватило, режется разрывом.
    #[test]
    fn a_row_that_does_not_fit_is_split_at_the_page_break() {
        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        // Строк текста на странице — без одной: строке таблицы остаётся меньше строки.
        let mut document = document_in_section(section, a4_capacity() - 1);
        let mut table = synthetic_table(900, 1, false);
        table.rows[0].cells[0].items = vec![
            BlockItem::Paragraph(paragraph(903, "row one")),
            BlockItem::Paragraph(paragraph(904, "row two")),
        ];
        document.body.items.push(BlockItem::Table(table));

        let layout = layout_of(&document);

        assert_eq!(
            layout.pages.len(),
            2,
            "остаток строки ушёл на вторую страницу"
        );
        let first_page = &layout.pages[0];
        let (rect, rows, cells) =
            table_fragment(first_page).expect("начало строки — на первой странице");
        assert!(rows[0].continues_below, "строка разрезана разрывом");
        assert!(
            rect.y + rect.height <= first_page.height - first_page.margins.bottom + 0.01,
            "фрагмент не заходит за нижнее поле: {}",
            rect.y + rect.height
        );

        let second_page = &layout.pages[1];
        let (rect, rows, second_cells) =
            table_fragment(second_page).expect("остаток строки — на второй странице");
        assert!(rows[0].continued_above, "строка продолжается сверху");
        assert!(
            (rect.y - second_page.margins.top).abs() < 0.01,
            "продолжение начинается от верхнего поля: {}",
            rect.y
        );
        assert!(
            rect.y + rect.height <= second_page.height - second_page.margins.bottom + 0.01,
            "продолжение влезает в полосу набора"
        );

        let mut texts: Vec<String> = cells.iter().flat_map(cell_texts).collect();
        texts.extend(second_cells.iter().flat_map(cell_texts));
        assert_eq!(
            texts,
            vec!["row one", "row two"],
            "оба блока строки разложены, пусть и по разным фрагментам"
        );
    }

    /// Строка с `w:cantSplit`, не помещающаяся в остаток, уходит на следующую целиком.
    #[test]
    fn a_cannot_split_row_moves_to_the_next_page_whole() {
        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        // Строк текста на странице — без одной: таблице остаётся меньше строки.
        let mut document = document_in_section(section, a4_capacity() - 1);
        let mut table = synthetic_table(900, 1, true);
        table.rows[0].cells[0].items = vec![
            BlockItem::Paragraph(paragraph(903, "row one")),
            BlockItem::Paragraph(paragraph(904, "row two")),
        ];
        document.body.items.push(BlockItem::Table(table));

        let layout = layout_of(&document);

        assert_eq!(
            layout.pages.len(),
            2,
            "таблица не влезла — страниц стало две"
        );
        assert!(
            table_fragment(&layout.pages[0]).is_none(),
            "на первой странице таблицы нет: она ушла целиком"
        );
        let second_page = &layout.pages[1];
        let (rect, rows, cells) =
            table_fragment(second_page).expect("таблица — на второй странице");
        assert_eq!(cells.len(), 1, "ячейка таблицы доехала вместе с таблицей");
        assert!(!rows[0].continued_above, "строка не разрезана");
        assert!(
            (rect.y - second_page.margins.top).abs() < 0.01,
            "таблица начинается от верхнего поля новой страницы: {}",
            rect.y
        );
        assert!(
            rect.y + rect.height <= second_page.height - second_page.margins.bottom + 0.01,
            "таблица влезает в полосу набора второй страницы"
        );
    }

    /// Длинная таблица разбивается на страницы, не выходя за полосу набора.
    #[test]
    fn a_sixty_row_table_spans_pages_within_the_content_band() {
        let section = section_with_margins(A4_WIDTH_TWIPS, A4_HEIGHT_TWIPS, FIXTURE_MARGIN_TWIPS);
        let mut document = document_in_section(section, 0);
        document
            .body
            .items
            .push(BlockItem::Table(synthetic_table(700, 60, false)));

        let layout = layout_of(&document);

        assert!(
            layout.pages.len() >= 2,
            "60 строк не помещаются на одну страницу: {}",
            layout.pages.len()
        );
        let mut rows = 0usize;
        for page in &layout.pages {
            let content_bottom = page.height - page.margins.bottom;
            for item in &page.items {
                let LayoutItem::Table {
                    rect,
                    rows: fragment_rows,
                    cells,
                    ..
                } = item
                else {
                    continue;
                };
                rows += fragment_rows.len();
                assert!(
                    rect.y + rect.height <= content_bottom + 0.01,
                    "фрагмент таблицы не заходит за нижнее поле: {}",
                    rect.y + rect.height
                );
                for cell in cells {
                    assert!(
                        cell.rect.y + cell.rect.height <= content_bottom + 0.01,
                        "ячейка не ниже полосы набора: {}",
                        cell.rect.y + cell.rect.height
                    );
                }
            }
        }
        assert_eq!(rows, 60, "все строки разложены по фрагментам");
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

    /// Элементы встроенных рисунков в порядке страниц и следования.
    fn images_of(layout: &PageLayout) -> Vec<ImageLayout> {
        layout
            .pages
            .iter()
            .flat_map(|page| page.items.iter())
            .filter_map(|item| match item {
                LayoutItem::Image {
                    node_id,
                    rect,
                    image_id,
                } => Some(ImageLayout {
                    node_id: *node_id,
                    rect: *rect,
                    image_id: *image_id,
                }),
                _ => None,
            })
            .collect()
    }

    /// Элемент встроенного рисунка: что и где легло в поток.
    #[derive(Debug, Clone, Copy)]
    struct ImageLayout {
        node_id: NodeId,
        rect: Rect,
        image_id: Option<u32>,
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

    /// Встроенные рисунки остаются в потоке: элементов `Float` не появляется,
    /// а место в полосе набора занимает [`LayoutItem::Image`].
    #[test]
    fn fixture_inline_drawings_stay_in_the_flow() {
        for name in ["images/inline.docx", "images/multiple_sizes.docx"] {
            let layout = layout_fixture(name);
            assert!(
                floats_of(&layout).is_empty(),
                "{name}: встроенные рисунки не плавают"
            );
            let images = images_of(&layout);
            assert!(
                !images.is_empty(),
                "{name}: у встроенных рисунков есть место в потоке"
            );
            // Узлы элементов — узлы картинок модели: по ним рисунок находят
            // выделение и потребитель кадра.
            let mut ids: Vec<NodeId> = images.iter().map(|image| image.node_id).collect();
            ids.dedup();
            assert_eq!(ids.len(), images.len(), "{name}: у рисунка свой узел");
        }
    }

    /// Размеры встроенных рисунков — из `wp:extent` (914400 EMU = 96 px):
    /// `images/multiple_sizes.json` даёт 457200×457200, 1371600×685800 и
    /// 685800×1371600 EMU.
    #[test]
    fn fixture_inline_images_take_their_extent_sizes() {
        let layout = layout_fixture("images/multiple_sizes.docx");
        let images = images_of(&layout);
        let expected = [(48.0, 48.0), (144.0, 72.0), (72.0, 144.0)];

        assert_eq!(images.len(), expected.len(), "по элементу на рисунок");
        for (image, (width, height)) in images.iter().zip(expected) {
            assert_px(image.rect.width, width, "ширина");
            assert_px(image.rect.height, height, "высота");
        }
        // Рисунки занимают место друг за другом: следующий ниже предыдущего.
        assert!(
            images[0].rect.y + images[0].rect.height <= images[1].rect.y + 0.01,
            "второй рисунок — под первым"
        );
        assert!(
            images[1].rect.y + images[1].rect.height <= images[2].rect.y + 0.01,
            "третий рисунок — под вторым"
        );
    }

    /// Раскладка ссылается на байты медиачасти: у элемента рисунка тот же
    /// идентификатор, что и у записи [`crate::Document::media`].
    #[test]
    fn inline_images_reference_the_media_registry() {
        let path = fixture_path("images/inline.docx");
        let bytes =
            std::fs::read(&path).unwrap_or_else(|_| panic!("нет фикстуры {}", path.display()));
        let document = crate::open(bytes).expect("фикстура разбирается");
        let layout = layout_of(&document);

        let media = document.media();
        assert_eq!(media.len(), 1, "у рисунка одна медиачасть");
        let images = images_of(&layout);
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].image_id,
            Some(media[0].id),
            "кадр адресует те же байты, что лежат в модели"
        );
        assert_eq!(media[0].part, "word/media/inline.png");
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

    /// Первый элемент-абзац раскладки.
    fn first_paragraph_item(layout: &PageLayout) -> &LayoutItem {
        layout
            .pages
            .iter()
            .flat_map(|page| page.items.iter())
            .find(|item| matches!(item, LayoutItem::Paragraph { .. }))
            .expect("в раскладке есть элемент-абзац")
    }

    /// `formatting/heading_1.docx`: начертание и кегль доезжают до элемента
    /// раскладки — рендер нарисует заголовок жирным 16 pt.
    #[test]
    fn fixture_heading_item_carries_the_run_style() {
        let heading = layout_fixture("formatting/heading_1.docx");
        let LayoutItem::Paragraph {
            font_family,
            size,
            bold,
            italic,
            underline,
            ..
        } = first_paragraph_item(&heading)
        else {
            unreachable!("first_paragraph_item отдаёт абзац")
        };

        assert!(*bold, "жирный из стиля Heading1");
        assert!(!*italic, "курсива в стиле нет");
        assert!(!*underline, "подчёркивания в стиле нет");
        assert_eq!(*size, HalfPoint::new(32), "16 pt из стиля");
        assert_px(
            half_points_to_px(size.value()),
            16.0 * 96.0 / 72.0,
            "кегль в пикселях",
        );
        assert_eq!(
            font_family.as_deref(),
            Some("Calibri"),
            "гарнитура docDefaults"
        );
    }

    /// `simple/one_paragraph.docx`: обычный текст остаётся нежирным, а кегль
    /// в элементе — `w:sz` из `docDefaults`, переведённый в пиксели.
    #[test]
    fn fixture_plain_item_stays_unstyled() {
        let plain = layout_fixture("simple/one_paragraph.docx");
        let LayoutItem::Paragraph {
            size,
            bold,
            italic,
            underline,
            ..
        } = first_paragraph_item(&plain)
        else {
            unreachable!("first_paragraph_item отдаёт абзац")
        };

        assert!(!*bold, "w:b не задан");
        assert!(!*italic, "w:i не задан");
        assert!(!*underline, "w:u не задан");
        assert_eq!(*size, HalfPoint::new(22), "11 pt из docDefaults");
        assert_px(
            half_points_to_px(size.value()),
            11.0 * 96.0 / 72.0,
            "кегль в пикселях",
        );
    }

    /// Абзац из одного поля; `cached` — результат между `separate` и `end`.
    ///
    /// Так абзац и собирает парсер: поле — отдельный inline-элемент со своим
    /// кэшированным результатом, а не текст.
    fn field_paragraph(id: u64, instruction: &str, cached: &str) -> Paragraph {
        Paragraph {
            id: NodeId::new(id),
            runs: vec![Inline::Field(Field {
                id: NodeId::new(id + 1),
                kind: FieldKind::Complex,
                instruction: instruction.to_owned(),
                result: vec![Inline::Run(Run {
                    id: NodeId::new(id + 2),
                    content: vec![RunContent::Text(cached.to_owned())],
                    ..Run::default()
                })],
                dirty: false,
            })],
            ..Paragraph::default()
        }
    }

    /// Документ из блоков на двухстрочной странице: две строки — страница.
    fn two_line_document(items: Vec<BlockItem>) -> Document {
        document_with(items, vec![section_without_margins(TWO_LINE_PAGE_TWIPS)])
    }

    /// Тексты абзацев страницы в порядке раскладки.
    fn page_texts(page: &Page) -> Vec<String> {
        page.items
            .iter()
            .filter_map(|item| match item {
                LayoutItem::Paragraph { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Документ на две страницы с полями: `PAGE` — номер своей страницы,
    /// `NUMPAGES` — число страниц всего документа.
    fn two_page_document_with_fields() -> Document {
        two_line_document(vec![
            BlockItem::Paragraph(field_paragraph(1, " PAGE ", "99")),
            BlockItem::Paragraph(paragraph(3, "one")),
            BlockItem::Paragraph(paragraph(5, "two")),
            BlockItem::Paragraph(field_paragraph(7, " PAGE ", "99")),
        ])
    }

    // Блокер S-слайса: номер страницы обязан совпасть со страницей, где стоит поле.
    #[test]
    fn a_page_field_names_the_page_it_stands_on() {
        let document = two_page_document_with_fields();
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 2);
        assert_eq!(
            page_texts(&layout.pages[0]),
            ["1", "one"],
            "кэшированное `99` замещается номером первой страницы"
        );
        assert_eq!(page_texts(&layout.pages[1]), ["two", "2"]);
    }

    #[test]
    fn num_pages_counts_the_whole_document() {
        let document = two_line_document(vec![
            BlockItem::Paragraph(paragraph(1, "one")),
            BlockItem::Paragraph(paragraph(3, "two")),
            BlockItem::Paragraph(field_paragraph(5, " NUMPAGES ", "99")),
            BlockItem::Paragraph(paragraph(7, "four")),
        ]);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 2, "две строки на страницу");
        assert_eq!(
            page_texts(&layout.pages[1]),
            ["2", "four"],
            "`NUMPAGES` — число страниц документа, а не кэш"
        );
    }

    #[test]
    fn an_unknown_field_is_not_resolved() {
        let document = two_line_document(vec![
            BlockItem::Paragraph(field_paragraph(1, " DATE ", "01.01.2026")),
            BlockItem::Paragraph(paragraph(3, "one")),
        ]);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("неизвестное поле раскладку не роняет");

        assert_eq!(
            page_texts(&layout.pages[0]),
            ["", "one"],
            "поле `DATE` раскладке неизвестно и остаётся неразвёрнутым"
        );
    }

    #[test]
    fn a_document_without_page_fields_is_laid_out_as_is() {
        let document = two_line_document(vec![
            BlockItem::Paragraph(paragraph(1, "one")),
            BlockItem::Paragraph(paragraph(3, "two")),
        ]);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(page_texts(&layout.pages[0]), ["one", "two"]);
    }

    /// Верхнее поле тестовых секций: 1440 twips = 96 px, как у [`default_section`].
    const TOP_MARGIN_TWIPS: i32 = 1440;

    /// Поле до верхнего колонтитула (`w:header`): 709 twips = 47.27 px — как в фикстурах.
    const HEADER_GAP_TWIPS: i32 = 709;

    /// Имя части колонтитула `default` в карте `Document::headers`.
    const DEFAULT_HEADER_PART: &str = "word/header1.xml";

    /// Имя части колонтитула `first` — своя, иначе карта не различила бы варианты.
    const FIRST_HEADER_PART: &str = "word/header2.xml";

    /// Секция A4 с верхним полем 96 px и полем колонтитула 47.27 px.
    fn section_with_header_gap() -> Section {
        let mut section = default_section();
        section.margins.header = Some(Twips::new(HEADER_GAP_TWIPS));
        section.properties.margins = section.margins.clone();
        section
    }

    /// Колонтитул из `lines` однострочных абзацев: строка — 19.2 px.
    fn header_of(part: &str, lines: usize) -> HeaderFooter {
        let items = (0..lines)
            .map(|index| {
                let id = u64::try_from(index).unwrap_or(u64::MAX) * 2 + 1;
                BlockItem::Paragraph(paragraph(id, &format!("header line {index}")))
            })
            .collect();
        HeaderFooter {
            id: NodeId::new(900),
            part: part.to_owned(),
            body: Body {
                id: NodeId::new(901),
                items,
                sections: Vec::new(),
            },
        }
    }

    /// Ссылка на колонтитул: раскладке нужен только `target`.
    fn header_ref(part: &str) -> PartRef {
        PartRef {
            rel_id: "rIdHeader".to_owned(),
            target: part.to_owned(),
            external: false,
        }
    }

    /// Дать единственной секции документа колонтитул `default` из `lines` абзацев.
    fn with_default_header(mut document: Document, lines: usize) -> Document {
        document.headers.insert(
            DEFAULT_HEADER_PART.to_owned(),
            header_of(DEFAULT_HEADER_PART, lines),
        );
        document.body.sections[0].header_default = Some(header_ref(DEFAULT_HEADER_PART));
        document
    }

    /// Низ полосы колонтитула: максимум низов её элементов.
    fn chrome_bottom(items: &[LayoutItem]) -> f32 {
        items
            .iter()
            .map(item_bottom)
            .reduce(f32::max)
            .unwrap_or(0.0)
    }

    /// Высокий колонтитул отодвигает тело: верхнее поле растёт до его низа.
    #[test]
    fn a_tall_header_pushes_the_body_below_it() {
        // Четыре строки по 19.2 px: 47.27 + 76.8 = 124.07 px — выше поля в 96 px.
        let document = with_default_header(document_in_section(section_with_header_gap(), 1), 4);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert_eq!(layout.pages.len(), 1, "тело — один абзац");
        let page = &layout.pages[0];
        let field = twips_to_px(Twips::new(TOP_MARGIN_TWIPS));
        let bottom = chrome_bottom(&layout.chrome[0].header);
        assert!(
            bottom > field,
            "колонтитул должен не помещаться в поле: {bottom} <= {field}"
        );
        assert!(
            page.margins.top > field,
            "верхнее поле должно вырасти: {} <= {field}",
            page.margins.top
        );
        assert!(
            page.margins.top >= bottom,
            "верхнее поле не может кончаться выше низа колонтитула: {} < {bottom}",
            page.margins.top
        );
        let first = paragraph_rects(page)[0];
        assert!(
            first.y >= bottom,
            "первый абзац тела поверх колонтитула: {} < {bottom}",
            first.y
        );
        assert!(
            first.y >= page.margins.top - 0.01,
            "первый абзац тела не выше полосы набора: {} < {}",
            first.y,
            page.margins.top
        );
    }

    /// Короткий колонтитул ничего не двигает: поле — из `w:pgMar` на всех страницах.
    #[test]
    fn a_short_header_leaves_the_top_margin_alone() {
        // Одна строка: 47.27 + 19.2 = 66.47 px — помещается в поле в 96 px.
        let document = with_default_header(document_in_section(section_with_header_gap(), 150), 1);
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert!(
            layout.pages.len() > 1,
            "нужно несколько страниц, получено {}",
            layout.pages.len()
        );
        let field = twips_to_px(Twips::new(TOP_MARGIN_TWIPS));
        for page in &layout.pages {
            assert_px(page.margins.top, field, "верхнее поле");
            let first = paragraph_rects(page)[0];
            assert_px(first.y, field, "первый абзац от верхнего поля");
        }
    }

    /// `w:titlePg`: страница секции отодвигается под свой вариант колонтитула —
    /// первая под `first`, вторая под `default`, хотя поле у них одно.
    #[test]
    fn a_title_page_header_moves_only_the_first_page() {
        let mut section = section_with_header_gap();
        section.title_pg = true;
        section.header_default = Some(header_ref(DEFAULT_HEADER_PART));
        section.header_first = Some(header_ref(FIRST_HEADER_PART));
        let mut document = document_in_section(section, 150);
        document.headers.insert(
            DEFAULT_HEADER_PART.to_owned(),
            header_of(DEFAULT_HEADER_PART, 1),
        );
        document.headers.insert(
            FIRST_HEADER_PART.to_owned(),
            header_of(FIRST_HEADER_PART, 4),
        );
        let mut fonts = FontRegistry::new(64);

        let layout = layout_document(&document, &LayoutOptions::default(), &mut fonts)
            .expect("раскладка не должна падать");

        assert!(
            layout.pages.len() > 1,
            "нужны две страницы, получено {}",
            layout.pages.len()
        );
        assert_eq!(
            layout.chrome[0].header.len(),
            4,
            "на первой странице — колонтитул `first`"
        );
        assert_eq!(
            layout.chrome[1].header.len(),
            1,
            "на второй странице — колонтитул `default`"
        );
        let first_bottom = chrome_bottom(&layout.chrome[0].header);
        assert!(
            layout.pages[0].margins.top >= first_bottom,
            "верхнее поле первой страницы не может кончаться выше низа колонтитула: {} < {first_bottom}",
            layout.pages[0].margins.top
        );
        assert!(
            paragraph_rects(&layout.pages[0])[0].y >= first_bottom,
            "тело первой страницы поверх колонтитула `first`"
        );
        // `default` в поле помещается — поле второй страницы остаётся из `w:pgMar`.
        let field = twips_to_px(Twips::new(TOP_MARGIN_TWIPS));
        assert_px(
            layout.pages[1].margins.top,
            field,
            "верхнее поле второй страницы",
        );
        assert_px(
            paragraph_rects(&layout.pages[1])[0].y,
            field,
            "тело второй страницы от поля",
        );
        assert!(
            layout.pages[0].margins.top > layout.pages[1].margins.top,
            "колонтитул `first` выше `default`: {} <= {}",
            layout.pages[0].margins.top,
            layout.pages[1].margins.top
        );
    }
}
