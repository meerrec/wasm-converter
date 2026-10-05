//! Видимая область: прокрутка, зум, DPR и перевод «раскладка → экран».
//!
//! `Viewport` — чистая геометрия окна (ADR-0003): раскладка листа и семантика
//! закреплений остаются в `xlsx`, а перевод координат «пиксели раскладки ↔
//! физические пиксели canvas» живёт здесь. `scale` — зум, умноженный на DPR:
//! сколько физических пикселей приходится на единицу раскладки.

use crate::geometry::{Point, Rect};

/// Окно просмотра.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Горизонтальная прокрутка, пиксели раскладки.
    pub x: f32,
    /// Вертикальная прокрутка, пиксели раскладки.
    pub y: f32,
    /// Ширина окна в физических пикселях canvas.
    pub w: f32,
    /// Высота окна в физических пикселях canvas.
    pub h: f32,
    /// Масштаб: зум, умноженный на плотность пикселей экрана.
    pub scale: f32,
}

impl Viewport {
    /// Создаёт окно; неположительный масштаб заменяется единицей.
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32, scale: f32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            scale: if scale > 0.0 { scale } else { 1.0 },
        }
    }

    /// Видимая часть `content` в координатах раскладки.
    ///
    /// Пересечение прокрученного окна с границами контента; прокрутка до
    /// начала контента зажимается к `content.x/content.y`.
    #[must_use]
    pub fn visible_range(&self, content: &Rect) -> Rect {
        let left = self.x.max(content.x);
        let top = self.y.max(content.y);
        let right = (left + self.w / self.scale).min(content.right());
        let bottom = (top + self.h / self.scale).min(content.bottom());
        Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    /// Минимальная прокрутка, при которой `target` целиком виден.
    ///
    /// Уже видимый прямоугольник окно не двигает; прямоугольник крупнее окна
    /// выравнивается по левому верхнему углу; прокрутка не уходит в минус —
    /// верхнюю границу контента знает вызывающий, а не окно.
    #[must_use]
    pub fn scroll_to(&self, target: Rect) -> Viewport {
        let view_w = self.w / self.scale;
        let view_h = self.h / self.scale;

        let dx = if target.w > view_w || target.x < self.x {
            target.x - self.x
        } else if target.right() > self.x + view_w {
            target.right() - (self.x + view_w)
        } else {
            0.0
        };
        let dy = if target.h > view_h || target.y < self.y {
            target.y - self.y
        } else if target.bottom() > self.y + view_h {
            target.bottom() - (self.y + view_h)
        } else {
            0.0
        };

        let mut next = *self;
        next.x = (self.x + dx).max(0.0);
        next.y = (self.y + dy).max(0.0);
        next
    }

    /// Точка раскладки → физические пиксели окна.
    #[must_use]
    pub fn layout_to_screen(&self, p: Point) -> Point {
        Point::new((p.x - self.x) * self.scale, (p.y - self.y) * self.scale)
    }

    /// Физические пиксели окна → точка раскладки.
    #[must_use]
    pub fn screen_to_layout(&self, p: Point) -> Point {
        Point::new(p.x / self.scale + self.x, p.y / self.scale + self.y)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new(0.0, 0.0, 800.0, 600.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 2000.0,
        h: 1500.0,
    };

    #[test]
    fn nonpositive_scale_is_clamped() {
        let viewport = Viewport::new(0.0, 0.0, 800.0, 600.0, 0.0);
        assert_eq!(viewport.scale, 1.0);
        let viewport = Viewport::new(0.0, 0.0, 800.0, 600.0, -2.0);
        assert_eq!(viewport.scale, 1.0);
    }

    #[test]
    fn visible_range_clamps_to_content() {
        let viewport = Viewport::new(-10.0, -10.0, 800.0, 600.0, 2.0);
        let range = viewport.visible_range(&CONTENT);
        assert_eq!(range, Rect::new(0.0, 0.0, 400.0, 300.0));

        let viewport = Viewport::new(1900.0, 1400.0, 800.0, 600.0, 2.0);
        let range = viewport.visible_range(&CONTENT);
        assert_eq!(range, Rect::new(1900.0, 1400.0, 100.0, 100.0));
    }

    #[test]
    fn screen_and_layout_are_inverse() {
        let viewport = Viewport::new(40.0, 25.0, 800.0, 600.0, 1.5);
        let p = Point::new(123.0, 456.0);
        let round = viewport.screen_to_layout(viewport.layout_to_screen(p));
        assert!((round.x - p.x).abs() < 1e-4);
        assert!((round.y - p.y).abs() < 1e-4);
    }

    #[test]
    fn scroll_to_does_not_move_visible_target() {
        let viewport = Viewport::new(100.0, 100.0, 800.0, 600.0, 1.0);
        let visible = Rect::new(200.0, 200.0, 100.0, 50.0);
        assert_eq!(viewport.scroll_to(visible), viewport);
    }

    #[test]
    fn scroll_to_moves_minimally() {
        let viewport = Viewport::new(100.0, 100.0, 800.0, 600.0, 1.0);
        // Цель справа-снизу: сдвиг ровно на недостающее.
        let target = Rect::new(950.0, 750.0, 100.0, 50.0);
        let next = viewport.scroll_to(target);
        assert_eq!(next, Viewport::new(250.0, 200.0, 800.0, 600.0, 1.0));

        // Цель слева-сверху от окна: сдвиг к её началу.
        let target = Rect::new(50.0, 50.0, 10.0, 10.0);
        let next = viewport.scroll_to(target);
        assert_eq!(next, Viewport::new(50.0, 50.0, 800.0, 600.0, 1.0));
    }

    #[test]
    fn scroll_to_aligns_oversized_target_and_clamps_to_zero() {
        let viewport = Viewport::new(100.0, 100.0, 800.0, 600.0, 1.0);
        let oversized = Rect::new(300.0, 300.0, 2000.0, 1000.0);
        assert_eq!(
            viewport.scroll_to(oversized),
            Viewport::new(300.0, 300.0, 800.0, 600.0, 1.0)
        );

        let target = Rect::new(-50.0, 0.0, 10.0, 10.0);
        assert_eq!(viewport.scroll_to(target).x, 0.0);
    }
}
