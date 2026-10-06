use serde::{Deserialize, Serialize};

use crate::overlay::OverlayConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PageSize {
    A4,
    A3,
    Letter,
    Legal,
    Custom { w_mm: f32, h_mm: f32 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PageOrientation {
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Margins {
    pub top_mm: f32,
    pub right_mm: f32,
    pub bottom_mm: f32,
    pub left_mm: f32,
}

impl Default for Margins {
    fn default() -> Self {
        Self {
            top_mm: 20.0,
            right_mm: 15.0,
            bottom_mm: 20.0,
            left_mm: 15.0,
        }
    }
}

/// `serde(default)`: настройки приходят из интерфейса, и новых полей в них
/// может не быть. Недостающее берётся из умолчаний, а не из нулей: у
/// [`PageConfig::avoid_row_break`] умолчание — «включено».
// Булевы поля — независимые настройки печати, как `pageSetup` в OOXML: сетка,
// запрет разрыва строки и два центрирования включаются порознь.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PageConfig {
    pub size: PageSize,
    pub orientation: PageOrientation,
    pub margins: Margins,
    pub scale: f32,
    pub fit_to_width: Option<u32>,
    /// Уместить лист на N страниц по высоте; `None` — по естественной высоте.
    ///
    /// Как и [`PageConfig::fit_to_width`], перебивает ручной масштаб и не
    /// увеличивает лист; подбор не опускается ниже 10% — предела Excel.
    pub fit_to_height: Option<u32>,
    pub print_grid_lines: bool,
    pub repeat_header_rows: usize,
    pub repeat_first_columns: usize,
    /// Не разрывать строку между страницами (как Excel).
    ///
    /// `true` — строка, не поместившаяся целиком, начинает следующую страницу.
    /// `false` — разрыв проходит по нижней границе области содержимого, а
    /// остаток строки продолжается на следующей странице: painter не режет
    /// ячейку, поэтому строка рисуется на обеих страницах, а неполная полоса
    /// уезжает в поле. И то и другое лучше потери данных, но лишний раз
    /// строку в PDF показывает только этот режим.
    pub avoid_row_break: bool,
    /// Сколько строк блока обязано остаться на нижнем краю страницы.
    ///
    /// Блок — отрезок подряд идущих непустых строк листа (раздел отчёта):
    /// пустая строка его разрывает. Разрыв, оставивший внизу меньше
    /// `orphan_rows` строк блока, сдвигается к его началу — блок уходит на
    /// следующую страницу целиком. `0` — правило выключено.
    pub orphan_rows: usize,
    /// Сколько строк блока обязано начинать следующую страницу.
    ///
    /// Если разрыв оставляет на следующей странице меньше `widow_rows` строк
    /// блока, разрыв сдвигается вверх: строки подтягиваются с текущей
    /// страницы (но не все — страница не остаётся пустой). `0` — правило
    /// выключено.
    pub widow_rows: usize,
    /// Центрировать полосу набора по горизонтали между полями.
    ///
    /// Сдвиг получает каждая страница: её полоса столбцов вместе с повторяемыми
    /// ставится посередине области содержимого, пустое место делится пополам.
    /// Полоса шире области (столбец шире страницы) не сдвигается.
    pub center_horizontally: bool,
    /// Центрировать полосу набора по вертикали между полями.
    ///
    /// Аналогично [`PageConfig::center_horizontally`], но по высоте: полоса
    /// строк вместе с шапкой ставится посередине области содержимого.
    pub center_vertically: bool,
}

impl Default for PageConfig {
    fn default() -> Self {
        Self {
            size: PageSize::A4,
            orientation: PageOrientation::Portrait,
            margins: Margins::default(),
            scale: 1.0,
            fit_to_width: None,
            fit_to_height: None,
            print_grid_lines: false,
            repeat_header_rows: 0,
            repeat_first_columns: 0,
            // Excel строку не рвёт: не поместившаяся целиком строка начинает
            // следующую страницу.
            avoid_row_break: true,
            orphan_rows: 0,
            widow_rows: 0,
            center_horizontally: false,
            center_vertically: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfOptions {
    pub page: PageConfig,
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: Vec<String>,
    pub compress: bool,
    pub bookmarks: bool,
    pub sheet_index: Option<usize>,
    /// Колонтитулы и водяной знак документа.
    ///
    /// Пустая настройка — поведение без изменений: колонтитулы берутся из
    /// книги (`oddHeader`/`oddFooter` листа), водяного знака нет. Заданные
    /// здесь строки перебивают книжные; коды `&L`/`&C`/`&R`, `&P` и `&N`
    /// разворачиваются при записи, когда число страниц уже известно.
    #[serde(default)]
    pub overlay: OverlayConfig,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            page: PageConfig::default(),
            title: String::new(),
            author: String::new(),
            subject: String::new(),
            keywords: Vec::new(),
            compress: true,
            bookmarks: true,
            sheet_index: None,
            overlay: OverlayConfig::default(),
        }
    }
}
