//! Раскладка плавающих элементов DOCX.
//!
//! Плавающие элементы (floats) — это изображения и текстовые блоки, которые
//! позиционируются независимо от потока текста. В DOCX это:
//! - `wp:anchor` — плавающие изображения с обтеканием
//! - Text boxes (через `w:txbxContent`)
//!
//! Плавающие элементы могут:
//! - Обтекаться текстом (wrap)
//! - Быть под текстом (behindDoc)
//! - Иметь привязку к абзацам, страницам, символам
//!
//! # Спецификация
//! See: ECMA-376, Part 1, §17.5.2 (Floating Elements)

use crate::model::drawing::{
    AlignH, AlignV, Anchor, PositionH, PositionV, RelFromH, RelFromV, WrapKind,
};
use crate::{HalfPoint, Twips};

use super::engine::{Page, Rect};

/// Сколько раз элемент можно сдвинуть вниз при разрешении пересечений.
/// Ограничение защищает от зацикливания на вырожденных данных.
const MAX_COLLISION_SHIFTS: usize = 16;

/// Преобразовать EMU в пиксели.
/// `1 EMU = 1/914_400` дюйма, `1` пиксель = `1/96` дюйма.
/// Отношение: (`1/914_400`) / (`1/96`) = `96/914_400` = `1/9525`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn emu_to_px(emu: i64) -> f32 {
    emu as f32 / 9525.0
}

/// Преобразовать twips в пиксели.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn twips_to_px(twips: Twips) -> f32 {
    twips.value() as f32 / 15.0
}

/// Преобразовать полупункты в пиксели.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn half_points_to_px(half_points: HalfPoint) -> f32 {
    (half_points.value() as f32 / 2.0) * (96.0 / 72.0)
}

/// Плавающий элемент на странице.
#[derive(Debug, Clone)]
pub struct FloatElement {
    /// Идентификатор элемента.
    pub id: u64,
    /// Прямоугольник положения.
    pub rect: Rect,
    /// Тип плавающего элемента.
    pub kind: FloatKind,
    /// Обтекание.
    pub wrap: WrapKind,
    /// Под текстом.
    pub behind_text: bool,
    /// З-индекс (порядок рендеринга).
    pub z_index: u32,
}

/// Тип плавающего элемента.
#[derive(Debug, Clone)]
pub enum FloatKind {
    /// Плавающее изображение.
    Image {
        /// Идентификатор relationship.
        rel_id: String,
        /// Имя части.
        part: Option<String>,
    },
    /// Текстовый блок.
    TextBox {
        /// Содержимое (пока заглушка).
        content: String,
    },
}

/// Контекст для раскладки плавающих элементов.
#[derive(Debug)]
pub struct FloatLayoutContext<'a> {
    /// Текущая страница.
    pub page: &'a Page,
    /// Положение начала содержимого страницы (с учётом полей).
    pub content_start: f32,
    /// Текущая позиция Y в потоке текста.
    pub text_y: f32,
    /// Высота абзаца-якоря: низ вертикальной базы `paragraph`/`line`.
    pub paragraph_height: f32,
    /// Список уже размещённых плавающих элементов на странице.
    pub placed_floats: &'a mut Vec<FloatElement>,
}

/// Раскладка одного плавающего элемента (Anchor).
///
/// При пересечении с уже размещёнными элементами элемент сдвигается вниз — под
/// нижнюю границу самого низкого из мешающих. Элемент, которому не хватило
/// места до нижней границы полосы набора, не размещается: его индекс вернёт
/// [`layout_floats_on_page`], и вызывающий перенесёт элемент на следующую страницу.
///
/// # Arguments
/// * `anchor` — плавающий элемент из модели
/// * `ctx` — контекст раскладки
///
/// # Returns
/// `Some(rect)` если элемент размещён на этой странице, `None` если не помещается
/// и нужно перенести на следующую страницу.
#[must_use]
pub fn layout_float(anchor: &Anchor, ctx: &FloatLayoutContext) -> Option<FloatElement> {
    // Вычислить размер элемента в пикселях
    let width = emu_to_px(anchor.extent.cx);
    let height = emu_to_px(anchor.extent.cy);

    // Вычислить позицию
    let (x, mut y) = compute_float_position(anchor, ctx, width, height);

    let content_bottom = ctx.page.height - ctx.page.margins.bottom;
    // Сдвиг только увеличивает `y`, поэтому дальше проверять нечего.
    if y + height > content_bottom {
        return None;
    }

    let mut shifts = 0;
    while let Some(lowest) =
        blocking_bottom(x, y, width, height, ctx.placed_floats, anchor.behind_text)
    {
        if shifts == MAX_COLLISION_SHIFTS {
            return None;
        }
        y = lowest;
        shifts += 1;
    }

    if y + height > content_bottom {
        return None;
    }

    Some(FloatElement {
        id: anchor.image.id.value(),
        rect: Rect::new(x, y, width, height),
        kind: FloatKind::Image {
            rel_id: anchor.image.rel_id.clone(),
            part: anchor.image.part.clone(),
        },
        wrap: anchor.wrap,
        behind_text: anchor.behind_text,
        // Порядковый номер размещения: 0 у первого, дальше по порядку обхода.
        // Порядок детерминирован, пока детерминирован порядок anchors.
        z_index: u32::try_from(ctx.placed_floats.len()).unwrap_or(u32::MAX),
    })
}

/// Вычислить позицию плавающего элемента.
///
/// # Спецификация
/// Position is calculated relative to the anchor point, which can be:
/// - A paragraph (relativeFrom = paragraph)
/// - A character (relativeFrom = character)
/// - A page (relativeFrom = page)
/// - A margin (relativeFrom = margin)
///
/// # Arguments
/// * `anchor` — плавающий элемент
/// * `ctx` — контекст раскладки
/// * `width` — ширина элемента в пикселях
/// * `height` — высота элемента в пикселях
///
/// # Returns
/// Координаты (x, y) в пикселях от верхнего левого угла страницы.
fn compute_float_position(
    anchor: &Anchor,
    ctx: &FloatLayoutContext,
    width: f32,
    height: f32,
) -> (f32, f32) {
    let x = compute_horizontal_position(&anchor.horizontal, ctx, width);
    let y = compute_vertical_position(&anchor.vertical, ctx, height);
    (x, y)
}

/// Отрезок базы отсчёта по горизонтали: `(начало, конец)`.
///
/// `rightMargin`/`outsideMargin` дают тот же отрезок полосы набора, что и
/// левые поля: сторону выбирает `align`, а не база.
fn horizontal_base(pos_h: &PositionH, page: &Page) -> (f32, f32) {
    match pos_h.relative_from {
        RelFromH::Page => (0.0, page.width),
        RelFromH::LeftMargin
        | RelFromH::RightMargin
        | RelFromH::Margin
        | RelFromH::InsideMargin
        | RelFromH::OutsideMargin
        | RelFromH::Character
        | RelFromH::Column
        | RelFromH::Other(_) => (page.margins.left, page.width - page.margins.right),
    }
}

/// Отрезок базы отсчёта по вертикали: `(начало, конец)`.
fn vertical_base(pos_v: &PositionV, ctx: &FloatLayoutContext) -> (f32, f32) {
    let page = ctx.page;
    match pos_v.relative_from {
        RelFromV::Page => (0.0, page.height),
        // `paragraph`/`line` — границы абзаца-якоря, а не страница.
        RelFromV::Paragraph | RelFromV::Line => (ctx.text_y, ctx.text_y + ctx.paragraph_height),
        RelFromV::TopMargin
        | RelFromV::BottomMargin
        | RelFromV::Margin
        | RelFromV::InsideMargin
        | RelFromV::OutsideMargin
        | RelFromV::Other(_) => (page.margins.top, page.height - page.margins.bottom),
    }
}

/// Вычислить горизонтальную позицию.
fn compute_horizontal_position(
    pos_h: &PositionH,
    ctx: &FloatLayoutContext,
    element_width: f32,
) -> f32 {
    resolve_position(
        horizontal_base(pos_h, ctx.page),
        element_width,
        horizontal_edge(pos_h.align.as_ref()),
        pos_h.offset,
        pos_h.percent,
    )
}

/// Вычислить вертикальную позицию.
fn compute_vertical_position(
    pos_v: &PositionV,
    ctx: &FloatLayoutContext,
    element_height: f32,
) -> f32 {
    resolve_position(
        vertical_base(pos_v, ctx),
        element_height,
        vertical_edge(pos_v.align.as_ref()),
        pos_v.offset,
        pos_v.percent,
    )
}

/// Край базы, к которому прижимается элемент при выравнивании.
#[derive(Clone, Copy)]
enum AlignEdge {
    /// Начало отрезка (`left`/`top`).
    Start,
    /// Центр отрезка (`center`).
    Center,
    /// Конец отрезка (`right`/`bottom`).
    End,
}

fn horizontal_edge(align: Option<&AlignH>) -> Option<AlignEdge> {
    align.map(|align| match align {
        AlignH::Center => AlignEdge::Center,
        AlignH::Right | AlignH::Outside => AlignEdge::End,
        AlignH::Left | AlignH::Inside | AlignH::Other(_) => AlignEdge::Start,
    })
}

fn vertical_edge(align: Option<&AlignV>) -> Option<AlignEdge> {
    align.map(|align| match align {
        AlignV::Center => AlignEdge::Center,
        AlignV::Bottom | AlignV::Outside => AlignEdge::End,
        AlignV::Top | AlignV::Inside | AlignV::Other(_) => AlignEdge::Start,
    })
}

/// Вычислить позицию внутри базы-отрезка.
///
/// Формула общая для обеих осей: `offset` отсчитывается от начала отрезка,
/// `percent` — доля его длины. Ловушка вертикали: процент берётся от длины базы
/// (высоты полосы набора или страницы), а не от координаты `y`, — база задана
/// отрезком именно поэтому.
#[allow(clippy::cast_precision_loss)]
fn resolve_position(
    base: (f32, f32),
    size: f32,
    edge: Option<AlignEdge>,
    offset: Option<i64>,
    percent: Option<i32>,
) -> f32 {
    let (start, end) = base;
    if let Some(edge) = edge {
        match edge {
            AlignEdge::Start => start,
            AlignEdge::Center => (start + end - size) / 2.0,
            AlignEdge::End => end - size,
        }
    } else if let Some(offset) = offset {
        start + emu_to_px(offset)
    } else if let Some(percent) = percent {
        start + (percent as f32 / 100_000.0) * (end - start)
    } else {
        start
    }
}

/// Нижняя граница самого низкого элемента, который мешает размещению.
///
/// Мешающим считается перекрывающийся элемент, если хотя бы один из двух
/// нарисован перед текстом: передний перекрывает новый, а два элемента за
/// текстом не должны накладываться друг на друга. Элемент за текстом не
/// вытесняет передние — новый передний может лечь поверх него.
fn blocking_bottom(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    placed_floats: &[FloatElement],
    new_behind_text: bool,
) -> Option<f32> {
    let new_rect = Rect::new(x, y, width, height);
    placed_floats
        .iter()
        .filter(|f| f.rect.intersects(&new_rect) && (!f.behind_text || new_behind_text))
        .map(|f| f.rect.y + f.rect.height)
        .reduce(f32::max)
}

/// Создать контекст для раскладки плавающих элементов.
#[must_use]
pub fn create_float_context<'a>(
    page: &'a Page,
    text_y: f32,
    paragraph_height: f32,
    placed_floats: &'a mut Vec<FloatElement>,
) -> FloatLayoutContext<'a> {
    FloatLayoutContext {
        page,
        content_start: page.margins.top,
        text_y,
        paragraph_height,
        placed_floats,
    }
}

/// Раскладка всех плавающих элементов на странице.
///
/// # Arguments
/// * `anchors` — список плавающих элементов
/// * `page` — текущая страница
/// * `text_y` — текущая позиция в тексте
/// * `paragraph_height` — высота абзаца-якоря
/// * `placed_floats` — список уже размещённых элементов (будет пополнен)
///
/// # Returns
/// Индексы элементов из `anchors`, которые не поместились на странице
/// (нужно перенести на следующую).
pub fn layout_floats_on_page(
    anchors: &[Anchor],
    page: &Page,
    text_y: f32,
    paragraph_height: f32,
    placed_floats: &mut Vec<FloatElement>,
) -> Vec<usize> {
    let mut deferred = Vec::new();

    for (index, anchor) in anchors.iter().enumerate() {
        let ctx = create_float_context(page, text_y, paragraph_height, placed_floats);
        if let Some(float_elem) = layout_float(anchor, &ctx) {
            placed_floats.push(float_elem);
        } else {
            deferred.push(index);
        }
    }

    deferred
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::MarginsLayout;
    use crate::model::drawing::{
        AlignH, AlignV, Extent, PositionH, PositionV, RelFromH, RelFromV, WrapKind,
    };
    use crate::model::InlineImage;
    use crate::{HalfPoint, Twips};
    use doc_converter_core::NodeId;

    /// Высота абзаца-якоря в тестах: база `paragraph`/`line` — `[text_y, text_y + 40]`.
    const TEST_PARAGRAPH_HEIGHT: f32 = 40.0;
    /// Высота и ширина рисунка в тестах столкновений: 100x50 px.
    const TEST_EXTENT: (i64, i64) = (952_500, 476_250);

    /// Страница 800x1100 с полями 50: полоса набора 700x1000.
    fn test_page() -> Page {
        Page {
            number: 1,
            width: 800.0,
            height: 1100.0,
            items: vec![],
            margins: MarginsLayout {
                top: 50.0,
                right: 50.0,
                bottom: 50.0,
                left: 50.0,
                header: 0.0,
                footer: 0.0,
                gutter: 0.0,
            },
        }
    }

    fn make_anchor(horizontal: PositionH, vertical: PositionV, cx: i64, cy: i64) -> Anchor {
        Anchor {
            id: NodeId::new(7),
            extent: Extent { cx, cy },
            horizontal,
            vertical,
            wrap: WrapKind::Square,
            behind_text: false,
            image: InlineImage {
                id: NodeId::new(42),
                rel_id: "rId1".to_string(),
                part: None,
                name: None,
                description: None,
                extent: Extent { cx, cy },
            },
        }
    }

    fn pos_offset_h(relative_from: RelFromH, offset: i64) -> PositionH {
        PositionH {
            relative_from,
            align: None,
            offset: Some(offset),
            percent: None,
        }
    }

    fn pos_offset_v(relative_from: RelFromV, offset: i64) -> PositionV {
        PositionV {
            relative_from,
            align: None,
            offset: Some(offset),
            percent: None,
        }
    }

    fn pos_align_h(relative_from: RelFromH, align: AlignH) -> PositionH {
        PositionH {
            relative_from,
            align: Some(align),
            offset: None,
            percent: None,
        }
    }

    fn pos_align_v(relative_from: RelFromV, align: AlignV) -> PositionV {
        PositionV {
            relative_from,
            align: Some(align),
            offset: None,
            percent: None,
        }
    }

    fn pos_percent_h(relative_from: RelFromH, percent: i32) -> PositionH {
        PositionH {
            relative_from,
            align: None,
            offset: None,
            percent: Some(percent),
        }
    }

    fn pos_percent_v(relative_from: RelFromV, percent: i32) -> PositionV {
        PositionV {
            relative_from,
            align: None,
            offset: None,
            percent: Some(percent),
        }
    }

    /// Разместить элемент и добавить его к размещённым (для тестов столкновений).
    fn place_float(
        anchor: &Anchor,
        page: &Page,
        text_y: f32,
        placed: &mut Vec<FloatElement>,
    ) -> Option<FloatElement> {
        let ctx = create_float_context(page, text_y, TEST_PARAGRAPH_HEIGHT, placed);
        let float_elem = layout_float(anchor, &ctx)?;
        placed.push(float_elem.clone());
        Some(float_elem)
    }

    #[test]
    fn test_emu_to_px() {
        // 914400 EMU = 1 inch
        let emu_1_inch = 914_400_i64;
        let px = emu_to_px(emu_1_inch);
        // 1 inch = 96 px
        assert!((px - 96.0).abs() < 0.01);
    }

    #[test]
    fn test_twips_to_px() {
        // 1440 twips = 1 inch
        let twips_1_inch = Twips::new(1440);
        let px = twips_to_px(twips_1_inch);
        // 1 inch = 96 px
        assert!((px - 96.0).abs() < 0.01);
    }

    #[test]
    fn test_half_points_to_px() {
        // 24 half-points = 12 points
        let half_points = HalfPoint::new(24);
        let px = half_points_to_px(half_points);
        // 12 pt * (96/72) = 16 px
        assert!((px - 16.0).abs() < 0.01);
    }

    #[test]
    fn test_float_position_left_margin() {
        let page = test_page();
        let anchor = make_anchor(
            pos_align_h(RelFromH::LeftMargin, AlignH::Left),
            pos_align_v(RelFromV::TopMargin, AlignV::Top),
            100,
            50,
        );

        let mut placed_floats = vec![];
        let float_elem = place_float(&anchor, &page, page.margins.top, &mut placed_floats);

        assert!(float_elem.is_some());
        let float_elem = float_elem.unwrap();
        // Выравнивание `left`/`top` прижимает к началу полосы набора.
        assert!((float_elem.rect.x - page.margins.left).abs() < 0.01);
        assert!((float_elem.rect.y - page.margins.top).abs() < 0.01);
        assert!((float_elem.rect.width - emu_to_px(100)).abs() < 0.01);
        assert!((float_elem.rect.height - emu_to_px(50)).abs() < 0.01);
        assert_eq!(float_elem.id, 42);
    }

    /// `anchor_wrap_square.docx`: `relativeFrom="column"`, `posOffset=457200` EMU
    /// (48 px) и `relativeFrom="paragraph"`, `posOffset=228600` EMU (24 px).
    #[test]
    fn test_float_position_column_and_paragraph_fixture() {
        let page = test_page();
        let anchor = make_anchor(
            pos_offset_h(RelFromH::Column, 457_200),
            pos_offset_v(RelFromV::Paragraph, 228_600),
            1_143_000,
            857_250,
        );

        let mut placed_floats = vec![];
        let text_y = 200.0;
        let float_elem = place_float(&anchor, &page, text_y, &mut placed_floats).unwrap();

        assert!((float_elem.rect.x - (page.margins.left + 48.0)).abs() < 0.01);
        assert!((float_elem.rect.y - (text_y + 24.0)).abs() < 0.01);
        assert!((float_elem.rect.width - 120.0).abs() < 0.01);
        assert!((float_elem.rect.height - 90.0).abs() < 0.01);
    }

    /// `anchor_top_and_bottom.docx`: `relativeFrom="page"`, `posOffset=914400` EMU
    /// (96 px) — отсчёт от края страницы, а не от поля.
    #[test]
    fn test_float_position_page_fixture() {
        let page = test_page();
        let anchor = make_anchor(
            pos_offset_h(RelFromH::Page, 914_400),
            pos_offset_v(RelFromV::Paragraph, 0),
            2_743_200,
            685_800,
        );

        let mut placed_floats = vec![];
        let float_elem = place_float(&anchor, &page, 50.0, &mut placed_floats).unwrap();

        assert!((float_elem.rect.x - 96.0).abs() < 0.01);
        assert!((float_elem.rect.y - 50.0).abs() < 0.01);
    }

    /// Процент — доля длины базы-отрезка, а не координаты: для вертикали это
    /// высота полосы набора, а не `y` (прежняя формула давала `50 + 0.25*50`).
    #[test]
    fn test_float_percent_uses_base_length() {
        let page = test_page();
        let mut placed_floats = vec![];

        let vertical = make_anchor(
            pos_offset_h(RelFromH::Column, 0),
            pos_percent_v(RelFromV::TopMargin, 25_000),
            100,
            50,
        );
        let float_elem = place_float(&vertical, &page, 50.0, &mut placed_floats).unwrap();
        // 50 + 0.25 * (1100 - 50 - 50) = 300.
        assert!((float_elem.rect.y - 300.0).abs() < 0.01);

        let horizontal = make_anchor(
            pos_percent_h(RelFromH::Page, 50_000),
            pos_offset_v(RelFromV::TopMargin, 0),
            100,
            50,
        );
        let float_elem = place_float(&horizontal, &page, 50.0, &mut placed_floats).unwrap();
        // 0 + 0.5 * 800 = 400.
        assert!((float_elem.rect.x - 400.0).abs() < 0.01);
    }

    #[test]
    fn test_float_collision_shifts_down() {
        let page = test_page();
        let mut placed_floats = vec![];
        let anchor = make_anchor(
            pos_offset_h(RelFromH::Column, 0),
            pos_offset_v(RelFromV::TopMargin, 0),
            TEST_EXTENT.0,
            TEST_EXTENT.1,
        );

        let first = place_float(&anchor, &page, 50.0, &mut placed_floats).unwrap();
        assert!((first.rect.y - 50.0).abs() < 0.01);
        assert_eq!(first.z_index, 0);

        let second = place_float(&anchor, &page, 50.0, &mut placed_floats).unwrap();
        // Не `None`, а сдвиг вниз: под нижнюю границу первого.
        assert!((second.rect.y - (first.rect.y + first.rect.height)).abs() < 0.01);
        assert!((second.rect.x - first.rect.x).abs() < 0.01);
        assert_eq!(second.z_index, 1);
    }

    /// Элемент за текстом не вытесняет передний, но два элемента за текстом
    /// друг друга не перекрывают.
    #[test]
    fn test_float_behind_text_does_not_displace_front() {
        let page = test_page();
        let mut placed_floats = vec![];
        let mut behind = make_anchor(
            pos_offset_h(RelFromH::Column, 0),
            pos_offset_v(RelFromV::TopMargin, 0),
            TEST_EXTENT.0,
            TEST_EXTENT.1,
        );
        behind.behind_text = true;
        place_float(&behind, &page, 50.0, &mut placed_floats).unwrap();

        let front = make_anchor(
            pos_offset_h(RelFromH::Column, 0),
            pos_offset_v(RelFromV::TopMargin, 0),
            TEST_EXTENT.0,
            TEST_EXTENT.1,
        );
        let front_elem = place_float(&front, &page, 50.0, &mut placed_floats).unwrap();
        assert!((front_elem.rect.y - 50.0).abs() < 0.01);

        let behind_elem = place_float(&behind, &page, 50.0, &mut placed_floats).unwrap();
        assert!((behind_elem.rect.y - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_layout_floats_on_page_defers_overflow() {
        let page = test_page();
        let mut placed_floats = vec![];
        // Три полосы по 500 px: [50, 550], [550, 1050], третьей места нет.
        let anchor = make_anchor(
            pos_offset_h(RelFromH::Column, 0),
            pos_offset_v(RelFromV::TopMargin, 0),
            TEST_EXTENT.0,
            4_762_500,
        );
        let anchors = vec![anchor.clone(), anchor.clone(), anchor];

        let deferred = layout_floats_on_page(
            &anchors,
            &page,
            50.0,
            TEST_PARAGRAPH_HEIGHT,
            &mut placed_floats,
        );

        assert_eq!(placed_floats.len(), 2);
        assert_eq!(deferred, vec![2]);
        assert!((placed_floats[1].rect.y - 550.0).abs() < 0.01);
        assert_eq!(placed_floats[1].z_index, 1);
    }
}
