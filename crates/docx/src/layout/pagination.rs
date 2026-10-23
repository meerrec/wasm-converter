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
    /// Секция каждой завершённой страницы — индекс в `Document::body.sections`.
    ///
    /// Идёт вровень с `pages`: колонтитулам нужна секция страницы, а сама
    /// страница несёт только общий номер, по которому `w:titlePg` (первая
    /// страница секции) и чётность не восстановить.
    page_sections: Vec<usize>,
    /// Индекс секции текущей страницы.
    section_index: usize,
    /// Страница, которую пагинатор заполняет сейчас.
    current_page: Page,
    /// Текущая позиция Y (от верхнего края страницы).
    current_y: f32,
    /// Секция текущей страницы; её геометрию унаследует следующая.
    current_section: Section,
    /// Ширины колонок полосы набора текущей секции; одна — одноколоночная.
    column_widths: Vec<f32>,
    /// Промежуток между колонками (`w:space`), в пикселях.
    column_gap: f32,
    /// Текущая колонка: 0 — первая.
    column_index: usize,
    /// Низ полос верхних колонтитулов по секциям (индекс — `Document::body.sections`).
    ///
    /// Пусто — колонтитулы телу не переданы: полоса набора остаётся на верхнем
    /// поле. Вровень с секциями, а не с страницами: колонтитул выбирается по
    /// секции, номеру страницы и `w:titlePg`, и на момент создания страницы
    /// известны только они (см. [`Paginator::apply_header_extent`]).
    header_extents: Vec<HeaderExtent>,
    footer_extents: Vec<FooterExtent>,
    /// `w:evenAndOddHeaders`: включает колонтитул чётных страниц.
    even_and_odd: bool,
    /// Текущая страница открывает свою секцию — ей достаётся колонтитул `first`.
    first_in_section: bool,
}

/// Низ полосы верхнего колонтитула в координатах страницы.
///
/// Значения — абсолютные координаты по вертикали; `0.0` — колонтитула нет, и
/// тело начинается от верхнего поля. Колонтитул, не поместившийся в верхнее
/// поле, отодвигает тело вниз — как это делают Word и `LibreOffice`. Без этого
/// первая таблица документа легла бы поверх строк колонтитула.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeaderExtent {
    /// `w:headerReference w:type="default"`.
    pub default: f32,
    /// `w:headerReference w:type="first"` — первая страница секции при `w:titlePg`.
    pub first: f32,
    /// `w:headerReference w:type="even"` — чётные страницы при `w:evenAndOddHeaders`.
    pub even: f32,
}

impl HeaderExtent {
    /// Низ полосы того колонтитула, который печатается на странице.
    ///
    /// Выбор повторяет `chrome_part`: `first` при `w:titlePg` на первой странице
    /// секции, `even` на чётной, иначе `default`. Отсутствующую часть уже
    /// подменил расчёт ([`HeaderExtent`] приходит из раскладки колонтитулов),
    /// поэтому здесь достаточно выбора по флагам.
    fn bottom_of(&self, section: &Section, first_in_section: bool, even_page: bool) -> f32 {
        if first_in_section && section.title_pg {
            self.first
        } else if even_page {
            self.even
        } else {
            self.default
        }
    }
}

/// Насколько нижний колонтитул поднимается в полосу набора.
///
/// `w:footer` задаёт расстояние от низа страницы до нижнего края колонтитула, а
/// не его высоту: высокий подвал в это поле не влезает и наезжает на тело —
/// таблица, поставленная до самого низа полосы, печаталась поверх подписи.
/// Значение — расстояние от низа страницы до *верха* полосы колонтитула; низ
/// полосы набора поднимается до него, если нижнее поле меньше.
#[derive(Debug, Clone, Copy, Default)]
pub struct FooterExtent {
    /// `w:footerReference w:type="default"`.
    pub default: f32,
    /// `w:footerReference w:type="first"` — первая страница секции при `w:titlePg`.
    pub first: f32,
    /// `w:footerReference w:type="even"` — чётные страницы при `w:evenAndOddHeaders`.
    pub even: f32,
}

impl FooterExtent {
    /// Верх полосы того колонтитула, который печатается на странице, —
    /// расстояние от низа страницы.
    ///
    /// Выбор варианта повторяет [`HeaderExtent::bottom_of`].
    fn top_of(&self, section: &Section, first_in_section: bool, even_page: bool) -> f32 {
        if first_in_section && section.title_pg {
            self.first
        } else if even_page {
            self.even
        } else {
            self.default
        }
    }
}

/// Пустая страница с номером `number` по геометрии секции.
///
/// `Section::orientation` здесь не спрашивается намеренно: в OOXML альбомную
/// страницу производитель уже развернул — `w:w`/`w:h` при `w:orient="landscape"`
/// переставлены, — и `page_size` несёт готовые ширину и высоту. `LibreOffice`
/// (эталон `scripts/oracle-docx-pages.sh`) считает так же: у файла с
/// `w:orient="landscape"` и книжными `w:w`/`w:h` страница выходит книжной.
/// Разворот по `w:orient` разошёлся бы с эталоном на таком файле.
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

        let mut paginator = Self {
            pages: Vec::new(),
            page_sections: Vec::new(),
            section_index: 0,
            current_page,
            current_y,
            current_section: section.clone(),
            column_widths: Vec::new(),
            column_gap: 0.0,
            column_index: 0,
            header_extents: Vec::new(),
            footer_extents: Vec::new(),
            even_and_odd: false,
            first_in_section: true,
        };
        paginator.reset_columns();
        paginator
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

    /// Передать пагинатору низы верхних колонтитулов по секциям.
    ///
    /// Вызывается один раз, сразу после [`Paginator::new`]: от низов зависит
    /// верхняя граница полосы набора каждой страницы, а её пагинатор отдаёт
    /// блокам через [`Paginator::content_top`]. Если текущая страница ещё пуста,
    /// колонтитул сдвигает и её — иначе тело первой страницы осталось бы поверх
    /// колонтитула, пока следующие страницы уже отодвинуты.
    pub fn set_header_extents(&mut self, extents: Vec<HeaderExtent>, even_and_odd: bool) {
        self.header_extents = extents;
        self.even_and_odd = even_and_odd;
        let empty = self.current_page.items.is_empty();
        self.apply_header_extent();
        self.apply_footer_extent();
        if empty {
            self.current_y = self.current_page.margins.top;
        }
    }

    /// Передать пагинатору верхи нижних колонтитулов по секциям.
    ///
    /// Вызывается рядом с [`Paginator::set_header_extents`]: высокий подвал
    /// поднимает низ полосы набора, иначе тело наезжает на подпись в подвале.
    pub fn set_footer_extents(&mut self, extents: Vec<FooterExtent>) {
        self.footer_extents = extents;
        let empty = self.current_page.items.is_empty();
        self.apply_footer_extent();
        if empty {
            self.current_y = self.current_page.margins.top;
        }
    }

    /// Текущая страница для заполнения.
    pub fn current_page_mut(&mut self) -> &mut Page {
        &mut self.current_page
    }

    /// Текущая колонка: 0 — первая.
    #[must_use]
    pub fn column_index(&self) -> usize {
        self.column_index
    }

    /// Левая граница содержимого: от неё отсчитывается X элементов.
    ///
    /// В многоколоночной секции — левый край текущей колонки.
    // Колонок в секции единицы: точность f32 для их счёта заведомо достаточна.
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn content_left(&self) -> f32 {
        let before: f32 = self.column_widths.iter().take(self.column_index).sum();
        self.current_page.margins.left + before + self.column_gap * self.column_index as f32
    }

    /// Ширина полосы содержимого — ширина текущей колонки.
    #[must_use]
    pub fn content_width(&self) -> f32 {
        self.column_widths
            .get(self.column_index)
            .copied()
            .unwrap_or(
                self.current_page.width
                    - self.current_page.margins.left
                    - self.current_page.margins.right,
            )
    }

    /// Перейти к следующей колонке той же страницы.
    ///
    /// `false` — колонка последняя: следующей нет, начинать нужно новую
    /// страницу. Курсор возвращается к верхней границе полосы набора: колонка
    /// заполняется сверху вниз, где бы ни остановилась предыдущая.
    pub fn next_column(&mut self) -> bool {
        if self.column_index + 1 >= self.column_widths.len() {
            return false;
        }
        self.column_index += 1;
        self.current_y = self.current_page.margins.top;
        true
    }

    /// Пересчитать колонки под текущую страницу и секцию.
    ///
    /// Новая страница и смена секции всегда начинаются с первой колонки,
    /// поэтому здесь же сбрасывается её номер.
    fn reset_columns(&mut self) {
        let band = self.current_page.width
            - self.current_page.margins.left
            - self.current_page.margins.right;
        let (widths, gap) = super::engine::column_geometry(&self.current_section, band);
        self.column_widths = widths;
        self.column_gap = gap;
        self.column_index = 0;
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
        let section_index = self.section_index;
        let next = page_for(self.current_page.number + 1, &self.current_section);
        self.break_page(next, section_index, false)
    }

    /// Завершить текущую страницу и продолжить раскладку в другой секции.
    ///
    /// Нужен для разрыва секции: следующая страница получает геометрию
    /// `section`, а её номер продолжает общий счёт документа. `index` — номер
    /// секции в `Document::body.sections`; завершаемая страница ещё принадлежит
    /// прежней секции, поэтому её номер запоминается до смены.
    pub fn push_page_with_section(&mut self, section: &Section, index: usize) -> Page {
        let finished_section = self.section_index;
        self.current_section = section.clone();
        self.section_index = index;
        let next = page_for(self.current_page.number + 1, &self.current_section);
        // Страница открывает секцию: её колонтитул выбирается по `w:titlePg`.
        self.break_page(next, finished_section, true)
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
    /// сохраняются, а размеры листа и поля берутся у новой секции. Страница
    /// целиком достаётся новой секции: колонтитул — свойство страницы, а не
    /// куска текста, и разрыва посреди неё у колонтитула нет.
    pub fn set_section(&mut self, section: &Section, index: usize) {
        self.current_section = section.clone();
        self.section_index = index;
        // По номеру страница первая в новой секции (в `layout_chrome` она же
        // признаётся первой по секции предыдущей страницы). Полосу набора ей
        // колонтитул не двигает: содержимое уже разложено и курсор стоит в нём.
        self.first_in_section = true;

        let items = std::mem::take(&mut self.current_page.items);
        let mut page = page_for(self.current_page.number, section);
        page.items = items;
        self.current_page = page;
        self.reset_columns();
    }

    /// Отдать все страницы документа, включая текущую.
    #[must_use]
    pub fn finish(self) -> Vec<Page> {
        self.finish_with_sections().0
    }

    /// Отдать все страницы документа вместе с номером секции каждой.
    ///
    /// Номера секций идут вторым списком, вровень со страницами: [`Page`]
    /// их не несёт, а колонтитулам они нужны — первая страница секции
    /// (`w:titlePg`) и чётность по общему номеру не считаются.
    ///
    /// Пустая текущая страница в результат не попадает: документ, который
    /// закончился явным разрывом, не должен получить лишний пустой лист.
    /// Если же страниц нет вовсе, возвращается одна пустая — у документа
    /// всегда есть хотя бы одна страница.
    #[must_use]
    pub fn finish_with_sections(mut self) -> (Vec<Page>, Vec<usize>) {
        if !self.current_page.items.is_empty() || self.pages.is_empty() {
            self.pages.push(self.current_page);
            self.page_sections.push(self.section_index);
        }
        (self.pages, self.page_sections)
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
            let section_index = self.section_index;
            let next = page_for(self.current_page.number + 1, &self.current_section);
            self.break_page(next, section_index, false);
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
    /// Завершённая страница уходит в список для [`Paginator::finish`] вместе
    /// с номером своей секции (`finished_section`), но возвращается и
    /// вызывающему — отсюда копия.
    ///
    /// `starts_section` — новая страница открывает секцию `finished_section`:
    /// от этого зависит её колонтитул, а через него — верхняя граница полосы
    /// набора ([`Paginator::apply_header_extent`]).
    fn break_page(&mut self, next: Page, finished_section: usize, starts_section: bool) -> Page {
        let finished = std::mem::replace(&mut self.current_page, next);
        self.first_in_section = starts_section;
        self.apply_header_extent();
        // Подвал поднимает низ полосы набора: без этого последняя строка
        // страницы ложится на подпись в подвале.
        self.apply_footer_extent();
        self.current_y = self.current_page.margins.top;
        self.reset_columns();
        self.pages.push(finished.clone());
        self.page_sections.push(finished_section);
        finished
    }

    /// Опустить верхнюю границу полосы набора под колонтитул текущей страницы.
    ///
    /// Низ колонтитула приходит из раскладки колонтитулов (`layout_chrome`) как
    /// координата страницы, но самой раскладке тела страницы ещё нет: полоса
    /// набора одной страницы не зависит от содержимого других, поэтому низ
    /// считается заранее по секции, номеру страницы и `w:titlePg`.
    fn apply_header_extent(&mut self) {
        let Some(extent) = self.header_extents.get(self.section_index) else {
            return;
        };
        // Чётность — как в `layout_chrome`: она включается только флагом
        // `w:evenAndOddHeaders`, иначе чётная страница несёт колонтитул `default`.
        let even_page = self.even_and_odd && self.current_page.number.is_multiple_of(2);
        let bottom = extent.bottom_of(&self.current_section, self.first_in_section, even_page);
        if bottom > self.current_page.margins.top {
            self.current_page.margins.top = bottom;
        }
    }

    /// Поднять низ полосы набора над подвалом, если нижнего поля не хватает.
    fn apply_footer_extent(&mut self) {
        let Some(extent) = self.footer_extents.get(self.section_index) else {
            return;
        };
        let even_page = self.even_and_odd && self.current_page.number.is_multiple_of(2);
        let top = extent.top_of(&self.current_section, self.first_in_section, even_page);
        if top > self.current_page.margins.bottom {
            self.current_page.margins.bottom = top;
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

        let finished = paginator.push_page_with_section(&landscape, 1);
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

    /// Высокий подвал поднимает низ полосы набора: тело не наезжает на подпись.
    ///
    /// `w:footer` задаёт расстояние до нижнего края колонтитула, а не его
    /// высоту, поэтому подвал выше нижнего поля раньше ложился поверх тела —
    /// на реальной книге таблица печаталась прямо на строке «стр. 15 из 22».
    #[test]
    fn tall_footer_raises_the_content_bottom() {
        let mut paginator = Paginator::new(&section(11_906, 15_360, 1_440));
        let before = paginator.content_bottom();
        let normal_bottom = PAGE_HEIGHT_PX - MARGIN_PX;
        assert!((before - normal_bottom).abs() < 0.01);

        paginator.set_footer_extents(vec![FooterExtent {
            default: 120.0,
            first: 120.0,
            even: 120.0,
        }]);

        assert!(
            (paginator.current_page().margins.bottom - 120.0).abs() < 0.01,
            "нижнее поле выросло до верха подвала"
        );
        assert!(
            (paginator.content_bottom() - (PAGE_HEIGHT_PX - 120.0)).abs() < 0.01,
            "низ полосы набора поднялся над подвалом"
        );
    }

    /// Подвал ниже нижнего поля ничего не меняет: поле уже его накрывает.
    #[test]
    fn footer_inside_the_margin_keeps_the_content_bottom() {
        let mut paginator = Paginator::new(&section(11_906, 15_360, 1_440));
        paginator.set_footer_extents(vec![FooterExtent {
            default: 48.0,
            first: 48.0,
            even: 48.0,
        }]);
        assert!((paginator.current_page().margins.bottom - MARGIN_PX).abs() < 0.01);
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
        paginator.set_section(&landscape, 1);

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
