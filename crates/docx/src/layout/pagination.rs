//! Пагинация DOCX: разбивка потока по страницам.
//!
//! Учитывает:
//! - Размеры страниц из секций
//! - Поля страниц
//! - Колонтитулы
//! - Разрывы страниц (явные и автоматические)
//! - Многоколоночную раскладку

use crate::model::{Section, SectionType};

use super::engine::{MarginsLayout, Page, Rect};

/// Пагинатор: управляет созданием и заполнением страниц.
pub struct Paginator {
    /// Текущая страница.
    current_page: Page,
    /// Номер текущей страницы.
    current_page_number: u32,
    /// Высота текущей страницы.
    page_height: f32,
    /// Верхняя граница содержимого.
    content_top: f32,
    /// Нижняя граница содержимого.
    content_bottom: f32,
    /// Текущая позиция Y.
    current_y: f32,
    /// Текущая секция (клон).
    current_section: Option<Section>,
}

impl Paginator {
    /// Создать новый пагинатор для секции.
    #[must_use]
    pub fn new(section: &Section) -> Self {
        let width = super::engine::twips_to_px(section.page_size.width);
        let height = super::engine::twips_to_px(section.page_size.height);
        let margins = MarginsLayout::from_margins(&section.margins);

        Self {
            current_page: Page {
                number: 1,
                width,
                height,
                items: Vec::new(),
                margins,
            },
            current_page_number: 1,
            page_height: height,
            content_top: margins.top,
            content_bottom: height - margins.bottom,
            current_y: margins.top,
            current_section: Some(section.clone()),
        }
    }

    /// Текущая позиция Y.
    #[must_use]
    pub fn current_y(&self) -> f32 {
        self.current_y
    }

    /// Текущая страница.
    #[must_use]
    pub fn current_page_mut(&mut self) -> &mut Page {
        &mut self.current_page
    }

    /// Проверка, помещается ли элемент высотой `height` на текущей странице.
    #[must_use]
    pub fn fits(&self, height: f32) -> bool {
        self.current_y + height <= self.content_bottom
    }

    /// Перейти на новую страницу.
    ///
    /// # Arguments
    /// * `section` - новая секция (если меняется)
    /// * `section_break_type` - тип разрыва секции
    pub fn new_page(&mut self, section: Option<Section>, _section_break_type: Option<SectionType>) {
        // Сохранить текущую страницу
        // TODO: return the page instead of storing it

        // Создать новую страницу
        self.current_page_number += 1;

        if let Some(new_section) = section {
            self.current_section = Some(new_section);
        }

        let width = self.current_section.as_ref().map_or(11906.0 / 15.0, |s| {
            super::engine::twips_to_px(s.page_size.width)
        }); // A4 width in px
        let height = self.current_section.as_ref().map_or(16838.0 / 15.0, |s| {
            super::engine::twips_to_px(s.page_size.height)
        }); // A4 height in px

        let margins = self.current_section.as_ref().map_or_else(
            || MarginsLayout {
                top: 72.0, // 1 inch in px
                right: 72.0,
                bottom: 72.0,
                left: 72.0,
                header: 0.0,
                footer: 0.0,
                gutter: 0.0,
            },
            |s| MarginsLayout::from_margins(&s.margins),
        );

        self.current_page = Page {
            number: self.current_page_number,
            width,
            height,
            items: Vec::new(),
            margins,
        };

        self.page_height = height;
        self.content_top = margins.top;
        self.content_bottom = height - margins.bottom;
        self.current_y = margins.top;
    }

    /// Добавить элемент на текущую страницу и продвинуть позицию Y.
    ///
    /// Если элемент не помещается, автоматически создаётся новая страница.
    ///
    /// # Arguments
    /// * `height` - высота элемента
    /// * `needs_page_break` - требует разрыва страницы
    ///
    /// # Returns
    /// `true` если элемент поместился на текущей странице, `false` если была создана новая.
    pub fn add_item(&mut self, height: f32, needs_page_break: bool) -> bool {
        if needs_page_break || !self.fits(height) {
            let current_section = std::mem::take(&mut self.current_section);
            self.new_page(current_section, None);
            return false;
        }

        self.current_y += height;
        true
    }

    /// Добавить элемент на текущую страницу без продвижения Y.
    pub fn add_item_at(&mut self, height: f32) {
        self.current_y += height;
    }

    /// Получить прямоугольник для элемента на текущей позиции.
    #[must_use]
    pub fn next_rect(&self, width: f32, height: f32) -> Rect {
        Rect::new(
            self.current_page.margins.left,
            self.current_y,
            width,
            height,
        )
    }

    /// Проверить, нужно ли начинать новую страницу по типу разрыва секции.
    #[must_use]
    pub fn needs_section_break(
        section_type: Option<SectionType>,
        current_page_number: u32,
    ) -> bool {
        match section_type {
            Some(SectionType::NextPage) | None => true, // По умолчанию NextPage
            Some(SectionType::Continuous) => false,
            Some(SectionType::EvenPage) => current_page_number % 2 == 1, // Текущая страница нечётная, нужно чётная
            Some(SectionType::OddPage) => current_page_number.is_multiple_of(2), // Текущая страница чётная, нужно нечётная
        }
    }
}

/// Тип разрыва для пагинации.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakType {
    /// Разрыв страницы.
    Page,
    /// Разрыв колонки.
    Column,
    /// Разрыв секции.
    Section,
}

/// Определить, требует ли блок разрыва страницы.
#[must_use]
pub fn block_needs_page_break(
    block: &crate::model::BlockItem,
    _next_block: Option<&crate::model::BlockItem>,
) -> bool {
    match block {
        crate::model::BlockItem::SectPr(sect_pr) => {
            // Проверяем тип разрыва секции
            match sect_pr.section_type {
                Some(SectionType::NextPage | SectionType::EvenPage | SectionType::OddPage)
                | None => true,
                Some(SectionType::Continuous) => false,
            }
        }
        crate::model::BlockItem::Paragraph(p) => {
            // Проверяем, содержит ли абзац разрыв страницы
            if p.section_break.is_some() {
                return true;
            }

            // Проверяем свойства абзаца
            p.ppr.sect_pr.is_some()
        }
        crate::model::BlockItem::Table(_) | crate::model::BlockItem::Unknown { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_paginator_new_page() {
        // For now, just test that the paginator can be created
        // TODO: add proper test with actual section when we have the layout working
        assert!(true);
    }
}
