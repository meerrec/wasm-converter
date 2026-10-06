//! Геометрия страницы PDF и пересчёт пикселей раскладки XLSX в точки.
//!
//! Раскладка листа (`SheetLayout`) считает в пикселях при 96 dpi, PDF — в
//! точках при 72 pt, поэтому координаты переводятся делением на
//! [`PX_PER_POINT`]. Масштаб печати (`PageConfig::scale`) применяется здесь же,
//! как зум у canvas: сама раскладка о нём не знает.
//!
//! Лист длиннее страницы печатается несколькими страницами: у каждой свой
//! верх ([`PageGeometry::with_page_top_px`]), который вычитается из
//! вертикальных координат. Разбивка по строкам — [`crate::pagination`], повтор
//! заголовков и разбивка по столбцам — Спринт 7.

use doc_converter_xlsx::layout::PX_PER_POINT;
use printpdf::{Pt, Rect};

use crate::options::{PageConfig, PageOrientation, PageSize};

/// Точек на миллиметр (72 pt / 25.4 мм).
const PT_PER_MM: f32 = 72.0 / 25.4;

/// Размер страницы в точках с учётом ориентации.
#[must_use]
pub fn page_size_pt(cfg: &PageConfig) -> (f32, f32) {
    let (w_mm, h_mm) = match cfg.size {
        PageSize::A4 => (210.0, 297.0),
        PageSize::A3 => (297.0, 420.0),
        PageSize::Letter => (215.9, 279.4),
        PageSize::Legal => (215.9, 355.6),
        PageSize::Custom { w_mm, h_mm } => (w_mm, h_mm),
    };
    let (w_mm, h_mm) = match cfg.orientation {
        PageOrientation::Portrait => (w_mm, h_mm),
        PageOrientation::Landscape => (h_mm, w_mm),
    };
    (w_mm * PT_PER_MM, h_mm * PT_PER_MM)
}

/// Геометрия страницы: размеры, поля и область содержимого в точках.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    width_pt: f32,
    height_pt: f32,
    origin_x_pt: f32,
    origin_y_pt: f32,
    content_w_pt: f32,
    content_h_pt: f32,
    scale: f32,
    page_top_px: f32,
}

impl PageGeometry {
    /// Собрать геометрию из настроек страницы.
    ///
    /// Нулевой или отрицательный масштаб — заведомо испорченные настройки:
    /// такой документ был бы пустым, поэтому он трактуется как 100%.
    #[must_use]
    pub fn new(cfg: &PageConfig) -> Self {
        let (width_pt, height_pt) = page_size_pt(cfg);
        let margins = cfg.margins;
        let left = margins.left_mm * PT_PER_MM;
        let right = margins.right_mm * PT_PER_MM;
        let top = margins.top_mm * PT_PER_MM;
        let bottom = margins.bottom_mm * PT_PER_MM;
        Self {
            width_pt,
            height_pt,
            origin_x_pt: left,
            origin_y_pt: top,
            content_w_pt: (width_pt - left - right).max(0.0),
            content_h_pt: (height_pt - top - bottom).max(0.0),
            scale: if cfg.scale > 0.0 { cfg.scale } else { 1.0 },
            page_top_px: 0.0,
        }
    }

    /// Копия геометрии для страницы, начинающейся с `top_px` листа.
    ///
    /// Сдвиг только вертикальный: по горизонтали лист пока не делится
    /// (Спринт 7).
    #[must_use]
    pub fn with_page_top_px(self, top_px: f32) -> Self {
        Self {
            page_top_px: top_px,
            ..self
        }
    }

    /// Ширина страницы, точки.
    #[must_use]
    pub fn width_pt(&self) -> f32 {
        self.width_pt
    }

    /// Высота страницы, точки.
    #[must_use]
    pub fn height_pt(&self) -> f32 {
        self.height_pt
    }

    /// Левый край области содержимого, точки от левого края страницы.
    #[must_use]
    pub fn origin_x_pt(&self) -> f32 {
        self.origin_x_pt
    }

    /// Верхний край области содержимого, точки от верха страницы.
    #[must_use]
    pub fn origin_y_pt(&self) -> f32 {
        self.origin_y_pt
    }

    /// Масштаб печати.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Пиксели раскладки → точки PDF (96 dpi против 72 pt, с масштабом).
    #[must_use]
    pub fn px_to_pt(&self, px: f32) -> f32 {
        px / PX_PER_POINT * self.scale
    }

    /// Пиксели раскладки по вертикали → точки PDF от верха страницы.
    ///
    /// Отличие от [`PageGeometry::px_to_pt`] — вычет верха страницы: страница
    /// печатает свой отрезок листа начиная с области содержимого.
    #[must_use]
    pub fn y_px_to_pt(&self, px: f32) -> f32 {
        self.px_to_pt(px - self.page_top_px)
    }

    /// Сколько пикселей раскладки помещается в область содержимого.
    #[must_use]
    pub fn content_px(&self) -> (f32, f32) {
        let per_px = self.scale / PX_PER_POINT;
        (self.content_w_pt / per_px, self.content_h_pt / per_px)
    }

    /// Прямоугольник раскладки → прямоугольник в точках PDF.
    #[must_use]
    pub fn rect_to_pt(&self, rect: RectPx) -> RectPt {
        RectPt {
            x: self.origin_x_pt + self.px_to_pt(rect.x),
            y: self.origin_y_pt + self.y_px_to_pt(rect.y),
            w: self.px_to_pt(rect.w),
            h: self.px_to_pt(rect.h),
        }
    }
}

/// Прямоугольник в пикселях раскладки [`SheetLayout`](doc_converter_xlsx::SheetLayout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPx {
    /// Левый край.
    pub x: f32,
    /// Верхний край.
    pub y: f32,
    /// Ширина.
    pub w: f32,
    /// Высота.
    pub h: f32,
}

impl RectPx {
    /// Прямоугольник по краям.
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

/// Прямоугольник в точках PDF; начало координат — левый верхний угол страницы.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPt {
    /// Левый край.
    pub x: f32,
    /// Верхний край.
    pub y: f32,
    /// Ширина.
    pub w: f32,
    /// Высота.
    pub h: f32,
}

impl RectPt {
    /// Прямоугольник printpdf: у него начало координат — левый нижний угол.
    #[must_use]
    pub fn to_pdf(self, page_height_pt: f32) -> Rect {
        Rect {
            x: Pt(self.x),
            y: Pt(page_height_pt - self.y - self.h),
            width: Pt(self.w),
            height: Pt(self.h),
        }
    }
}
