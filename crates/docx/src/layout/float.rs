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
    /// Список уже размещённых плавающих элементов на странице.
    pub placed_floats: &'a mut Vec<FloatElement>,
}

/// Раскладка одного плавающего элемента (Anchor).
///
/// # Arguments
/// * `anchor` — плавающий элемент из модели
/// * `ctx` — контекст раскладки
/// * `text_width` — ширина области текста на странице
///
/// # Returns
/// `Some(rect)` если элемент размещён на этой странице, `None` если не помещается
/// и нужно перенести на следующую страницу.
#[must_use]
pub fn layout_float(
    anchor: &Anchor,
    ctx: &FloatLayoutContext,
    text_width: f32,
) -> Option<FloatElement> {
    // Вычислить размер элемента в пикселях
    let width = emu_to_px(anchor.extent.cx);
    let height = emu_to_px(anchor.extent.cy);

    // Вычислить позицию
    let (x, y) = compute_float_position(anchor, ctx, width, height, text_width);

    // Проверить, помещается ли на странице
    if y + height > ctx.page.height - ctx.page.margins.bottom {
        return None;
    }

    // Проверить пересечение с другими плавающими элементами
    if has_intersection(x, y, width, height, ctx.placed_floats) {
        // Пробуем сдвинуть вниз
        // TODO: более сложная стратегия избегания пересечений
        return None;
    }

    Some(FloatElement {
        id: 0, // TODO: использовать реальный ID
        rect: Rect::new(x, y, width, height),
        kind: FloatKind::Image {
            rel_id: anchor.image.rel_id.clone(),
            part: anchor.image.part.clone(),
        },
        wrap: anchor.wrap,
        behind_text: anchor.behind_text,
        z_index: 0, // TODO: вычислять z-индекс
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
/// * `text_width` — ширина области текста
///
/// # Returns
/// Координаты (x, y) в пикселях от верхнего левого угла страницы.
fn compute_float_position(
    anchor: &Anchor,
    ctx: &FloatLayoutContext,
    width: f32,
    height: f32,
    text_width: f32,
) -> (f32, f32) {
    let x = compute_horizontal_position(&anchor.horizontal, ctx, width, text_width);
    let y = compute_vertical_position(&anchor.vertical, ctx, height);
    (x, y)
}

/// Вычислить горизонтальную позицию.
fn compute_horizontal_position(
    pos_h: &PositionH,
    ctx: &FloatLayoutContext,
    element_width: f32,
    text_width: f32,
) -> f32 {
    let base_x = match pos_h.relative_from {
        RelFromH::LeftMargin
        | RelFromH::Margin
        | RelFromH::Character
        | RelFromH::InsideMargin
        | RelFromH::Column
        | RelFromH::Other(_) => ctx.page.margins.left,
        RelFromH::RightMargin | RelFromH::OutsideMargin => ctx.page.width - ctx.page.margins.right,
        RelFromH::Page => ctx.page.width / 2.0,
    };

    compute_offset(pos_h, base_x, element_width, text_width)
}

/// Вычислить вертикальную позицию.
fn compute_vertical_position(
    pos_v: &PositionV,
    ctx: &FloatLayoutContext,
    element_height: f32,
) -> f32 {
    let base_y = match pos_v.relative_from {
        RelFromV::TopMargin | RelFromV::InsideMargin => ctx.page.margins.top,
        RelFromV::BottomMargin | RelFromV::OutsideMargin => {
            ctx.page.height - ctx.page.margins.bottom
        }
        RelFromV::Page => ctx.page.height / 2.0,
        RelFromV::Margin | RelFromV::Paragraph | RelFromV::Line | RelFromV::Other(_) => ctx.text_y,
    };

    compute_vertical_offset(pos_v, base_y, element_height)
}

/// Вычислить горизонтальное смещение от базы.
#[allow(clippy::cast_precision_loss)]
fn compute_offset(pos: &PositionH, base_x: f32, element_width: f32, text_width: f32) -> f32 {
    if let Some(ref align) = pos.align {
        match align {
            AlignH::Left | AlignH::Inside | AlignH::Other(_) => base_x,
            AlignH::Center => base_x - element_width / 2.0,
            AlignH::Right | AlignH::Outside => base_x - element_width,
        }
    } else if let Some(offset) = pos.offset {
        base_x + emu_to_px(offset)
    } else if let Some(percent) = pos.percent {
        // Процент от ширины области текста
        base_x + (percent as f32 / 100_000.0) * text_width
    } else {
        base_x
    }
}

/// Вычислить вертикальное смещение от базы.
#[allow(clippy::cast_precision_loss)]
fn compute_vertical_offset(pos: &PositionV, base_y: f32, element_height: f32) -> f32 {
    if let Some(ref align) = pos.align {
        match align {
            AlignV::Top | AlignV::Inside | AlignV::Other(_) => base_y,
            AlignV::Center => base_y - element_height / 2.0,
            AlignV::Bottom | AlignV::Outside => base_y - element_height,
        }
    } else if let Some(offset) = pos.offset {
        base_y + emu_to_px(offset)
    } else if let Some(percent) = pos.percent {
        // Процент от высоты страницы
        base_y + (percent as f32 / 100_000.0) * base_y
    } else {
        base_y
    }
}

/// Проверить пересечение с уже размещёнными плавающими элементами.
fn has_intersection(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    placed_floats: &[FloatElement],
) -> bool {
    let new_rect = Rect::new(x, y, width, height);
    placed_floats.iter().any(|f| {
        f.rect.intersects(&new_rect)
            // Если элемент под текстом, пересечение допустимо
            && !f.behind_text
    })
}

/// Создать контекст для раскладки плавающих элементов.
#[must_use]
pub fn create_float_context<'a>(
    page: &'a Page,
    text_y: f32,
    placed_floats: &'a mut Vec<FloatElement>,
) -> FloatLayoutContext<'a> {
    FloatLayoutContext {
        page,
        content_start: page.margins.top,
        text_y,
        placed_floats,
    }
}

/// Раскладка всех плавающих элементов на странице.
///
/// # Arguments
/// * `anchors` — список плавающих элементов
/// * `page` — текущая страница
/// * `text_y` — текущая позиция в тексте
/// * `text_width` — ширина области текста
/// * `placed_floats` — списк уже размещённых элементов (будет пополнен)
///
/// # Returns
/// Список элементов, которые не поместились на странице (нужно перенести).
pub fn layout_floats_on_page(
    anchors: &[Anchor],
    page: &Page,
    text_y: f32,
    text_width: f32,
    placed_floats: &mut Vec<FloatElement>,
) -> Vec<usize> {
    let deferred = Vec::new();

    for anchor in anchors {
        let ctx = create_float_context(page, text_y, placed_floats);
        if let Some(float_elem) = layout_float(anchor, &ctx, text_width) {
            placed_floats.push(float_elem);
        } else {
            // TODO: track which index was deferred
            // For now, we'll just skip adding to deferred
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
        let page = Page {
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
        };

        let anchor = Anchor {
            id: NodeId::new(0),
            extent: Extent { cx: 100, cy: 50 },
            horizontal: PositionH {
                relative_from: RelFromH::LeftMargin,
                align: Some(AlignH::Left),
                offset: None,
                percent: None,
            },
            vertical: PositionV {
                relative_from: RelFromV::TopMargin,
                align: Some(AlignV::Top),
                offset: None,
                percent: None,
            },
            wrap: WrapKind::Square,
            behind_text: false,
            image: InlineImage {
                id: NodeId::new(0),
                rel_id: "rId1".to_string(),
                part: None,
                name: None,
                description: None,
                extent: Extent { cx: 100, cy: 50 },
            },
        };

        let mut placed_floats = vec![];
        let ctx = FloatLayoutContext {
            page: &page,
            content_start: page.margins.top,
            text_y: page.margins.top,
            placed_floats: &mut placed_floats,
        };

        let float_elem = layout_float(&anchor, &ctx, 700.0);

        assert!(float_elem.is_some());
        let float_elem = float_elem.unwrap();
        assert!((float_elem.rect.x - 50.0).abs() < 0.01);
        assert!((float_elem.rect.y - 50.0).abs() < 0.01);
        assert!((float_elem.rect.width - emu_to_px(100)).abs() < 0.01);
        assert!((float_elem.rect.height - emu_to_px(50)).abs() < 0.01);
    }
}
