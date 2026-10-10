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
///
/// Страницы принадлежат пагинатору: завершённые он копит сам и отдаёт их
/// через [`Paginator::finish`]. Поэтому страница, начатая автоматически
/// из-за переполнения в [`Paginator::add_item`], не теряется.
pub struct Paginator {
    /// Завершённые страницы.
    pages: Vec<Page>,
    /// Страница, которую пагинатор заполняет сейчас.
    current_page: Page,
    /// Текущая позиция Y (от верхнего края страницы).
    current_y: f32,
    /// Секция текущей страницы; её геометрию унаследует следующая.
    current_section: Section,
}

/// Пустая страница с номером `number` по геометрии секции.
fn page_for(number: u32, section: &Section) -> Page {
    Page {
        number,
        width: super::engine::twips_to_px(section.page_size.width),
        height: super::engine::twips_to_px(section.page_size.height),
        items: Vec::new(),
        margins: MarginsLayout::from_margins(&section.margins),
    }
}

impl Paginator {
    /// Создать новый пагинатор для секции.
    #[must_use]
    pub fn new(section: &Section) -> Self {
        let current_page = page_for(1, section);
        let current_y = current_page.margins.top;

        Self {
            pages: Vec::new(),
            current_page,
            current_y,
            current_section: section.clone(),
        }
    }

    /// Текущая позиция Y.
    #[must_use]
    pub fn current_y(&self) -> f32 {
        self.current_y
    }

    /// Текущая страница.
    #[must_use]
    pub fn current_page(&self) -> &Page {
        &self.current_page
    }

    /// Текущая страница для заполнения.
    pub fn current_page_mut(&mut self) -> &mut Page {
        &mut self.current_page
    }

    /// Левая граница содержимого: от неё отсчитывается X элементов.
    #[must_use]
    pub fn content_left(&self) -> f32 {
        self.current_page.margins.left
    }

    /// Ширина полосы содержимого — ширина страницы без боковых полей.
    #[must_use]
    pub fn content_width(&self) -> f32 {
        self.current_page.width - self.current_page.margins.left - self.current_page.margins.right
    }

    /// Верхняя граница содержимого.
    #[must_use]
    pub fn content_top(&self) -> f32 {
        self.current_page.margins.top
    }

    /// Нижняя граница содержимого.
    #[must_use]
    pub fn content_bottom(&self) -> f32 {
        self.current_page.height - self.current_page.margins.bottom
    }

    /// Проверка, помещается ли элемент высотой `height` на текущей странице.
    #[must_use]
    pub fn fits(&self, height: f32) -> bool {
        self.current_y + height <= self.content_bottom()
    }

    /// Завершить текущую страницу и начать следующую с той же геометрией.
    ///
    /// Возвращает заполненную страницу. В пагинаторе остаётся её копия, поэтому
    /// [`Paginator::finish`] отдаёт документ целиком независимо от того,
    /// пользуется ли вызывающий возвратом.
    pub fn push_page(&mut self) -> Page {
        let next = page_for(self.current_page.number + 1, &self.current_section);
        self.break_page(next)
    }

    /// Завершить текущую страницу и продолжить раскладку в другой секции.
    ///
    /// Нужен для разрыва секции: следующая страница получает геометрию
    /// `section`, а её номер продолжает общий счёт документа.
    pub fn push_page_with_section(&mut self, section: &Section) -> Page {
        self.current_section = section.clone();
        let next = page_for(self.current_page.number + 1, &self.current_section);
        self.break_page(next)
    }

    /// Секция, в геометрии которой идёт раскладка.
    #[must_use]
    pub fn current_section(&self) -> &Section {
        &self.current_section
    }

    /// Перейти к геометрии другой секции, не завершая страницу.
    ///
    /// Нужен для `Continuous`-разрыва: содержимое продолжается на той же
    /// странице, поэтому уже разложенные элементы и позиция курсора
    /// сохраняются, а размеры листа и поля берутся у новой секции.
    pub fn set_section(&mut self, section: &Section) {
        self.current_section = section.clone();

        let items = std::mem::take(&mut self.current_page.items);
        let mut page = page_for(self.current_page.number, section);
        page.items = items;
        self.current_page = page;
    }

    /// Отдать все страницы документа, включая текущую.
    ///
    /// Пустая текущая страница в результат не попадает: документ, который
    /// закончился явным разрывом, не должен получить лишний пустой лист.
    /// Если же страниц нет вовсе, возвращается одна пустая — у документа
    /// всегда есть хотя бы одна страница.
    #[must_use]
    pub fn finish(mut self) -> Vec<Page> {
        if !self.current_page.items.is_empty() || self.pages.is_empty() {
            self.pages.push(self.current_page);
        }
        self.pages
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
            let next = page_for(self.current_page.number + 1, &self.current_section);
            self.break_page(next);
            return false;
        }

        self.current_y += height;
        true
    }

    /// Получить прямоугольник для элемента на текущей позиции.
    #[must_use]
    pub fn next_rect(&self, width: f32, height: f32) -> Rect {
        Rect::new(self.content_left(), self.current_y, width, height)
    }

    /// Продвинуть курсор на `height`, не проверяя переполнение.
    ///
    /// Нужен блоку, который уже разложен по текущей позиции: [`Paginator::add_item`]
    /// на переполнении начал бы новую страницу и оставил бы координаты элементов
    /// от прежней. Блок выше полосы набора так и остаётся на этой странице —
    /// содержимое просто выходит за нижнее поле.
    pub fn advance(&mut self, height: f32) {
        self.current_y += height;
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

    /// Начать страницу `next`, завершив текущую.
    ///
    /// Завершённая страница уходит в список для [`Paginator::finish`], но
    /// возвращается и вызывающему — отсюда копия.
    fn break_page(&mut self, next: Page) -> Page {
        let finished = std::mem::replace(&mut self.current_page, next);
        self.current_y = self.current_page.margins.top;
        self.pages.push(finished.clone());
        finished
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
///
/// Блочный `w:sectPr` — это свойства последней секции тела, а не её конец:
/// разрыв за ним дал бы документу лишнюю пустую страницу. Абзац со `w:sectPr`
/// внутри `w:pPr` секцию закрывает, но нужен ли за ним разрыв, решает тип
/// следующей секции ([`Paginator::needs_section_break`]), а не сам абзац.
#[must_use]
pub fn block_needs_page_break(
    block: &crate::model::BlockItem,
    _next_block: Option<&crate::model::BlockItem>,
) -> bool {
    match block {
        crate::model::BlockItem::Paragraph(p) => {
            p.section_break.is_some() || p.ppr.sect_pr.is_some()
        }
        crate::model::BlockItem::SectPr(_)
        | crate::model::BlockItem::Table(_)
        | crate::model::BlockItem::Unknown { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::{twips_to_px, LayoutItem};
    use crate::model::{Columns, Margins, Orientation, PageSize, SectionProperties};
    use crate::Twips;
    use doc_converter_core::NodeId;

    /// Размеры и поля точны в f32: 15360 twips = 1024 px, 1440 twips = 96 px.
    const PAGE_HEIGHT_PX: f32 = 1024.0;
    const MARGIN_PX: f32 = 96.0;

    /// Секция с явной геометрией: размеры и все поля в twips.
    fn section(width: i32, height: i32, margin: i32) -> Section {
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
                width: Twips::new(width),
                height: Twips::new(height),
            },
            orientation: Orientation::Portrait,
            margins: Margins {
                top: Twips::new(margin),
                right: Twips::new(margin),
                bottom: Twips::new(margin),
                left: Twips::new(margin),
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

    /// Секция A4 с полями в один дюйм.
    fn a4() -> Section {
        section(11_906, 16_838, 1_440)
    }

    #[test]
    fn test_page_geometry_follows_section() {
        let paginator = Paginator::new(&a4());

        assert_eq!(paginator.current_page().number, 1);
        assert!((paginator.current_page().width - twips_to_px(Twips::new(11_906))).abs() < 0.01);
        assert!((paginator.current_page().height - twips_to_px(Twips::new(16_838))).abs() < 0.01);
        assert!((paginator.content_left() - MARGIN_PX).abs() < 0.01);
        assert!((paginator.content_top() - MARGIN_PX).abs() < 0.01);
        assert!(
            (paginator.content_width() - (twips_to_px(Twips::new(11_906)) - 2.0 * MARGIN_PX)).abs()
                < 0.01
        );
        assert!(
            (paginator.content_bottom() - (twips_to_px(Twips::new(16_838)) - MARGIN_PX)).abs()
                < 0.01
        );
        // Курсор стартует у верхней границы содержимого.
        assert!((paginator.current_y() - paginator.content_top()).abs() < 0.01);
    }

    #[test]
    fn test_push_page_hands_out_finished_pages() {
        let mut paginator = Paginator::new(&a4());
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::PageBreak);

        let first = paginator.push_page();
        assert_eq!(first.number, 1);
        assert_eq!(first.items.len(), 1, "элементы уходят вместе со страницей");
        assert_eq!(paginator.current_page().number, 2);
        assert!((paginator.current_y() - paginator.content_top()).abs() < 0.01);

        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::PageBreak);
        let second = paginator.push_page();
        assert_eq!(second.number, 2);

        let pages = paginator.finish();
        assert_eq!(pages.len(), 2, "пустая текущая страница не попадает в итог");
        assert_eq!(pages[0].number, 1);
        assert_eq!(pages[1].number, 2);

        // Геометрия наследуется от секции на каждой странице.
        for page in &pages {
            assert!((page.width - twips_to_px(Twips::new(11_906))).abs() < 0.01);
            assert!((page.height - twips_to_px(Twips::new(16_838))).abs() < 0.01);
            assert!((page.margins.left - MARGIN_PX).abs() < 0.01);
            assert!((page.margins.top - MARGIN_PX).abs() < 0.01);
            assert!((page.margins.bottom - MARGIN_PX).abs() < 0.01);
        }
    }

    #[test]
    fn test_finish_of_empty_document_is_single_page() {
        let pages = Paginator::new(&a4()).finish();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].number, 1);
        assert!(pages[0].items.is_empty());
    }

    #[test]
    fn test_section_change_switches_page_geometry() {
        let mut paginator = Paginator::new(&a4());
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::PageBreak);

        // Та же бумага, повёрнутая; поля вдвое уже (720 twips = 48 px).
        let mut landscape = section(16_838, 11_906, 720);
        landscape.orientation = Orientation::Landscape;

        let finished = paginator.push_page_with_section(&landscape);
        assert_eq!(finished.number, 1, "завершается страница прежней секции");
        assert!((finished.width - twips_to_px(Twips::new(11_906))).abs() < 0.01);

        assert_eq!(paginator.current_page().number, 2);
        assert!(
            (paginator.current_page().width - twips_to_px(Twips::new(16_838))).abs() < 0.01,
            "ширина берётся из новой секции"
        );
        assert!((paginator.current_page().height - twips_to_px(Twips::new(11_906))).abs() < 0.01);
        assert!((paginator.content_left() - 48.0).abs() < 0.01);
        assert!((paginator.content_top() - 48.0).abs() < 0.01);
        assert!(
            (paginator.content_width() - (twips_to_px(Twips::new(16_838)) - 96.0)).abs() < 0.01
        );
        assert!(
            (paginator.content_bottom() - (twips_to_px(Twips::new(11_906)) - 48.0)).abs() < 0.01
        );
        assert!((paginator.current_y() - paginator.content_top()).abs() < 0.01);

        // Следующая страница наследует уже ландшафтную секцию.
        let next = paginator.push_page();
        assert_eq!(next.number, 2);
        assert!((paginator.current_page().width - twips_to_px(Twips::new(16_838))).abs() < 0.01);
    }

    #[test]
    fn test_fits_at_content_bottom_boundary() {
        let paginator = Paginator::new(&section(11_906, 15_360, 1_440));

        assert!((paginator.current_page().height - PAGE_HEIGHT_PX).abs() < 0.01);
        assert!((paginator.content_bottom() - (PAGE_HEIGHT_PX - MARGIN_PX)).abs() < 0.01);

        let room = paginator.content_bottom() - paginator.current_y();
        assert!(
            paginator.fits(room),
            "элемент ровно по нижнюю границу влезает"
        );
        assert!(
            !paginator.fits(room + 1.0),
            "на пиксель больше — не влезает"
        );
    }

    #[test]
    fn test_add_item_starts_new_page_on_overflow() {
        let mut paginator = Paginator::new(&section(11_906, 15_360, 1_440));
        let room = paginator.content_bottom() - paginator.current_y();

        assert!(
            paginator.add_item(room, false),
            "элемент влез на первую страницу"
        );
        assert!((paginator.current_y() - paginator.content_bottom()).abs() < 0.01);

        assert!(
            !paginator.add_item(1.0, false),
            "переполнение начинает новую страницу"
        );
        assert_eq!(paginator.current_page().number, 2);
        assert!((paginator.current_y() - paginator.content_top()).abs() < 0.01);

        assert!(
            !paginator.add_item(1.0, true),
            "явный разрыв переносит, даже когда элемент помещается"
        );
        assert_eq!(paginator.current_page().number, 3);

        // Страницы, начатые автоматически, не теряются.
        let pages = paginator.finish();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].number, 1);
        assert_eq!(pages[1].number, 2);

        // Маленький элемент помещается и продвигает курсор.
        let mut paginator = Paginator::new(&a4());
        assert!(paginator.add_item(10.0, false));
        assert!((paginator.current_y() - (paginator.content_top() + 10.0)).abs() < 0.01);
    }

    #[test]
    fn test_set_section_changes_geometry_without_a_break() {
        let mut paginator = Paginator::new(&a4());
        paginator.advance(10.0);
        paginator
            .current_page_mut()
            .items
            .push(LayoutItem::PageBreak);
        let y_before = paginator.current_y();

        // Та же бумага, повёрнутая; поля вдвое уже (720 twips = 48 px).
        let mut landscape = section(16_838, 11_906, 720);
        landscape.orientation = Orientation::Landscape;
        paginator.set_section(&landscape);

        assert_eq!(paginator.current_page().number, 1, "страница не завершена");
        assert_eq!(
            paginator.current_page().items.len(),
            1,
            "уже разложенные элементы остаются"
        );
        assert!(
            (paginator.current_y() - y_before).abs() < 0.01,
            "курсор не сдвигается: содержимое продолжается"
        );
        assert!(
            (paginator.current_page().width - twips_to_px(Twips::new(16_838))).abs() < 0.01,
            "ширина берётся из новой секции"
        );
        assert!((paginator.current_page().height - twips_to_px(Twips::new(11_906))).abs() < 0.01);
        assert!((paginator.content_left() - 48.0).abs() < 0.01);
        assert_eq!(
            paginator.current_section().page_size.width.value(),
            16_838,
            "дальше раскладка идёт в новой секции"
        );

        // Следующая страница наследует уже ландшафтную секцию.
        let finished = paginator.push_page();
        assert_eq!(finished.number, 1);
        assert!((paginator.current_page().width - twips_to_px(Twips::new(16_838))).abs() < 0.01);
    }

    #[test]
    fn test_body_sect_pr_is_not_a_page_break() {
        // Блочный `w:sectPr` описывает последнюю секцию: разрыва за ним нет.
        let properties = SectionProperties {
            section_type: Some(SectionType::NextPage),
            ..SectionProperties::default()
        };
        assert!(!block_needs_page_break(
            &crate::model::BlockItem::SectPr(properties),
            None
        ));

        // Абзац со своим `w:sectPr` секцию закрывает.
        let mut paragraph = crate::model::Paragraph::default();
        assert!(!block_needs_page_break(
            &crate::model::BlockItem::Paragraph(paragraph.clone()),
            None
        ));
        paragraph.section_break = Some(Box::new(SectionProperties::default()));
        assert!(block_needs_page_break(
            &crate::model::BlockItem::Paragraph(paragraph),
            None
        ));
    }

    #[test]
    fn test_advance_keeps_the_block_on_the_page() {
        let mut paginator = Paginator::new(&a4());
        // На единицу больше, чем осталось до нижней границы: add_item начал бы
        // новую страницу, advance — нет.
        let past_the_bottom = paginator.content_bottom() - paginator.current_y() + 1.0;

        paginator.advance(past_the_bottom);

        assert_eq!(
            paginator.current_page().number,
            1,
            "новая страница не начата"
        );
        assert!(
            paginator.current_y() > paginator.content_bottom(),
            "курсор ушёл за нижнюю границу полосы набора"
        );
        assert!(!paginator.fits(1.0));
    }

    #[test]
    fn test_needs_section_break_truth_table() {
        // Без типа разрыва секция ведёт себя как NextPage.
        assert!(Paginator::needs_section_break(None, 1));
        assert!(Paginator::needs_section_break(None, 2));
        assert!(Paginator::needs_section_break(
            Some(SectionType::NextPage),
            1
        ));
        assert!(Paginator::needs_section_break(
            Some(SectionType::NextPage),
            2
        ));

        // Continuous продолжает ту же страницу.
        assert!(!Paginator::needs_section_break(
            Some(SectionType::Continuous),
            1
        ));
        assert!(!Paginator::needs_section_break(
            Some(SectionType::Continuous),
            2
        ));

        // EvenPage: после нечётной нужен разрыв, после чётной — уже нет.
        assert!(Paginator::needs_section_break(
            Some(SectionType::EvenPage),
            1
        ));
        assert!(!Paginator::needs_section_break(
            Some(SectionType::EvenPage),
            2
        ));

        // OddPage: наоборот.
        assert!(!Paginator::needs_section_break(
            Some(SectionType::OddPage),
            1
        ));
        assert!(Paginator::needs_section_break(
            Some(SectionType::OddPage),
            2
        ));
    }
}
