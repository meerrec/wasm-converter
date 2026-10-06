//! Отдельный wasm-модуль экспорта XLSX → PDF.
//!
//! Тянет `doc-converter-pdf` (→ printpdf), поэтому в основной модуль не
//! входит: просмотрщику он нужен только по клику «Экспорт в PDF», и воркер
//! подгружает его по требованию. Модуль самодостаточен — никакого
//! `thread_local`: книгу он получает байтами и разбирает сам.
//!
//! Как и `crates/wasm`, крейт собирается только под `wasm32`: на нативном
//! таргете он пуст, и `cargo clippy --workspace --all-targets` проходит без
//! выбора цели.

#![cfg(target_arch = "wasm32")]

use doc_converter_pdf::{Margins, PageOrientation, PageSize, PdfExporter, PdfOptions};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

/// Настройки экспорта из интерфейса: имена полей — как у TS `PdfOptions`.
/// Все поля необязательны; чего нет — берётся умолчание [`PdfOptions`].
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfOptionsJs {
    #[serde(default)]
    page_size: Option<PageSizeJs>,
    #[serde(default)]
    orientation: Option<OrientationJs>,
    #[serde(default)]
    margins: Option<MarginsJs>,
    #[serde(default)]
    scale: Option<f32>,
    /// Лист из настроек: интерфейс может экспортировать не тот, что на экране.
    #[serde(default)]
    sheet_index: Option<usize>,
}

/// Размер страницы: строки те же, что в TS-объединении `PdfOptions.pageSize`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub enum PageSizeJs {
    #[serde(rename = "A4")]
    A4,
    #[serde(rename = "A3")]
    A3,
    #[serde(rename = "Letter")]
    Letter,
    #[serde(rename = "Legal")]
    Legal,
}

/// Ориентация страницы: в TS она записана строчными.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrientationJs {
    Portrait,
    Landscape,
}

/// Поля страницы в миллиметрах.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct MarginsJs {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl PdfOptionsJs {
    /// Дополнить умолчания PDF-крейта тем, что выбрал интерфейс.
    fn into_options(self) -> PdfOptions {
        let mut options = PdfOptions::default();
        if let Some(size) = self.page_size {
            options.page.size = match size {
                PageSizeJs::A4 => PageSize::A4,
                PageSizeJs::A3 => PageSize::A3,
                PageSizeJs::Letter => PageSize::Letter,
                PageSizeJs::Legal => PageSize::Legal,
            };
        }
        if let Some(orientation) = self.orientation {
            options.page.orientation = match orientation {
                OrientationJs::Portrait => PageOrientation::Portrait,
                OrientationJs::Landscape => PageOrientation::Landscape,
            };
        }
        if let Some(margins) = self.margins {
            options.page.margins = Margins {
                top_mm: margins.top,
                right_mm: margins.right,
                bottom_mm: margins.bottom,
                left_mm: margins.left,
            };
        }
        if let Some(scale) = self.scale {
            options.page.scale = scale;
        }
        options.sheet_index = self.sheet_index;
        options
    }
}

/// Ошибка разбора — сообщением для человека: показывать её будет интерфейс.
fn to_js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Настройки из JS или умолчания, если их не передали.
fn parse_options(options: JsValue) -> Result<PdfOptions, JsValue> {
    if options.is_undefined() || options.is_null() {
        return Ok(PdfOptions::default());
    }
    serde_wasm_bindgen::from_value::<PdfOptionsJs>(options)
        .map(PdfOptionsJs::into_options)
        .map_err(to_js)
}

/// Экспортировать лист книги в PDF. Книга приходит байтами XLSX и
/// разбирается здесь же: у модуля нет общего с просмотрщиком состояния.
///
/// `sheet` — индекс листа в книге; `options.sheetIndex` перекрывает его,
/// если задан.
///
/// # Errors
/// Если байты не OOXML-пакет, листа с таким индексом нет или printpdf не
/// смог собрать документ.
#[wasm_bindgen]
pub fn export_pdf(bytes: &[u8], sheet: usize, options: JsValue) -> Result<Vec<u8>, JsValue> {
    let options = parse_options(options)?;
    let sheet = options.sheet_index.unwrap_or(sheet);
    let book = doc_converter_xlsx::open(bytes.to_vec()).map_err(to_js)?;
    let mut exporter = PdfExporter::new(options);
    exporter.export_xlsx_sheet(&book, sheet).map_err(to_js)
}
