use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageConfig {
    pub size: PageSize,
    pub orientation: PageOrientation,
    pub margins: Margins,
    pub scale: f32,
    pub fit_to_width: Option<u32>,
    pub print_grid_lines: bool,
    pub repeat_header_rows: usize,
    pub repeat_first_columns: usize,
    pub center_horizontally: bool,
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
            print_grid_lines: false,
            repeat_header_rows: 0,
            repeat_first_columns: 0,
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
        }
    }
}
