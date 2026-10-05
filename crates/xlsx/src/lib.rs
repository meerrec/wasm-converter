//! XLSX: разбор `workbook.xml`, `sheet*.xml`, `sharedStrings.xml`, `styles.xml`
//! и `theme1.xml`.
//!
//! ```no_run
//! use doc_converter_xlsx::{open, CellRef};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("book.xlsx")?;
//! let book = open(bytes)?;
//!
//! let sheet = &book.sheets()[0];
//! println!("лист «{}», ячеек {}", sheet.meta.name, sheet.cells.cell_count());
//!
//! if let Some(cell) = sheet.cells.cell(CellRef::new(0, 0)) {
//!     // Значение ячейки — как оно записано в файле, текст — как его рисует Excel.
//!     println!("{}", cell.value.text(book.shared_strings()).unwrap_or("число"));
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Книга разбирается целиком при открытии: рендер и экспорт работают по модели,
//! а ленивый разбор потребовал бы держать открытый архив внутри книги.
//!
//! Точка входа — [`open`]; из чего состоит книга — [`Workbook`], [`Sheet`] и
//! [`Worksheet`].
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

pub mod cellref;
pub mod conditional;
pub mod dims;
pub mod drawing;
pub mod error;
pub mod layout;
pub mod model;
pub mod numfmt;
pub mod paint;
pub mod sheet_meta;
pub mod strings;
pub mod styles;
pub mod theme;
pub mod workbook;
pub mod worksheet;

/// Помощники чтения XML — деталь реализации, наружу не выходят.
mod xml;

pub use cellref::{CellRef, ParseError, Range};
pub use conditional::{EffectiveStyle, RuleIndex};
pub use dims::{ColWidth, ColWidths, RowHeight, RowHeights, SheetDims, SheetFormat};
pub use drawing::{EditAs, ImageAnchor, ImageExtent, ImageMarker, SheetImage};
pub use error::{Result, XlsxError};
pub use layout::SheetLayout;
pub use model::{
    Border, BorderSide, BorderStyle, Cell, CellError, CellFormat, CellIsOperator, CellValue, Color,
    ColorScale, ConditionalFormatting, ConditionalRule, DataBar, Dxf, DxfNumberFormat, Fill,
    FillPattern, Font, IconSet, RuleKind, Sheet, SheetContent, SheetState, StyleTable, Theme,
    Threshold, ThresholdKind, Workbook, Worksheet, WorksheetBuilder, WorksheetMeta,
    THEME_COLOR_COUNT,
};
pub use paint::{build as paint_sheet, PaintOptions, Viewport};
pub use sheet_meta::{Hyperlink, HyperlinkTarget, Merges, Pane, PaneKind, PaneState, SheetView};
pub use strings::SharedStrings;
pub use workbook::WorkbookMeta;

use doc_converter_core::rels::{rels_part, RelMap};
use doc_converter_core::Archive;

/// Часть с каталогом листов.
pub const WORKBOOK_PART: &str = "xl/workbook.xml";

/// Общая таблица строк. В книге без строк этой части нет вовсе, и это норма.
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// Таблица стилей.
pub const STYLES_PART: &str = "xl/styles.xml";

/// Отношение темы: `…/relationships/theme`. Имя части темы в пакете может
/// отличаться (`theme2.xml`), поэтому она ищется по типу связи.
const THEME_REL: &str = "/theme";

/// Отношение чертежа листа: `…/relationships/drawing`. Как и тема, ищется по
/// типу связи: имя части (`drawing2.xml`) зависит от порядка добавления.
const DRAWING_REL: &str = "/drawing";

/// Открыть XLSX из сырых байт.
///
/// Читает каталог листов, общую таблицу строк, стили, тему, содержимое всех
/// листов и изображения.
///
/// Листы разбираются сразу: рендер и экспорт работают по модели, а ленивый
/// разбор потребовал бы держать открытый архив внутри книги.
///
/// # Errors
/// Если байты не OOXML-пакет, в нём нет `xl/workbook.xml` или какая-то из
/// частей не разбирается.
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes)?;
    archive.validate_ooxml()?;

    let rels = RelMap::parse(&archive.read(&rels_part(WORKBOOK_PART))?)?;
    let catalog = WorkbookMeta::parse(&archive.read(WORKBOOK_PART)?, &rels, WORKBOOK_PART)?;

    let shared_strings = if archive.contains(SHARED_STRINGS_PART) {
        SharedStrings::parse(&archive.read(SHARED_STRINGS_PART)?, SHARED_STRINGS_PART)?
    } else {
        SharedStrings::default()
    };

    let style_table = if archive.contains(STYLES_PART) {
        styles::parse(&archive.read(STYLES_PART)?, STYLES_PART)?
    } else {
        StyleTable::default()
    };

    let theme = read_theme(&mut archive, &rels)?;

    let mut sheets = Vec::with_capacity(catalog.sheets.len());
    for meta in catalog.sheets {
        // Связи листа нужны гиперссылкам и изображениям: их цели живут
        // в отдельных частях.
        let rels = read_rels(&mut archive, &meta.part)?;
        let mut content = worksheet::parse(&archive.read(&meta.part)?, &meta.part, rels.as_ref())?;
        // Чертёж — отдельная часть: сам лист на него только ссылается.
        content.images = read_images(&mut archive, &meta.part, rels.as_ref())?;
        sheets.push(Sheet::new(meta, content));
    }

    Ok(Workbook::new(
        sheets,
        shared_strings,
        style_table,
        theme,
        catalog.date1904,
    ))
}

/// Тема книги: часть ищется по связи книги, а не по жёсткому пути — имя темы
/// в пакете может отличаться от `xl/theme/theme1.xml`.
///
/// Отсутствие темы или её части — не ошибка: цвета `theme="n"` тогда просто
/// не разрешаются, а текст берёт цвет по умолчанию.
fn read_theme(archive: &mut Archive, rels: &RelMap) -> Result<Theme> {
    let Some(rel) = rels
        .items
        .values()
        .find(|rel| rel.rel_type.ends_with(THEME_REL))
    else {
        return Ok(Theme::default());
    };
    let Some(part) = rel.part(WORKBOOK_PART) else {
        return Ok(Theme::default());
    };
    if !archive.contains(&part) {
        return Ok(Theme::default());
    }
    theme::parse(&archive.read(&part)?, part)
}

/// Связи части пакета, если они есть: у листа без гиперссылок и картинок
/// части `_rels` может не быть вовсе, и это норма.
fn read_rels(archive: &mut Archive, source_part: &str) -> Result<Option<RelMap>> {
    let part = rels_part(source_part);
    if !archive.contains(&part) {
        return Ok(None);
    }
    Ok(Some(RelMap::parse(&archive.read(&part)?)?))
}

/// Изображения листа: чертёж ищется по связи листа, как тема — по связи книги.
///
/// Отсутствие чертежа, битая связь на media или ссылка на часть, которой нет
/// в пакете, — не ошибка: лист тогда просто остаётся без картинок.
fn read_images(
    archive: &mut Archive,
    sheet_part: &str,
    rels: Option<&RelMap>,
) -> Result<Vec<SheetImage>> {
    let Some(rel) = rels.and_then(|rels| {
        rels.items
            .values()
            .find(|rel| rel.rel_type.ends_with(DRAWING_REL))
    }) else {
        return Ok(Vec::new());
    };
    let Some(part) = rel.part(sheet_part) else {
        return Ok(Vec::new());
    };
    if !archive.contains(&part) {
        return Ok(Vec::new());
    }

    let drawing_rels = read_rels(archive, &part)?;
    let mut images = drawing::parse(&archive.read(&part)?, &part, drawing_rels.as_ref())?;
    // Ссылка на media без самой части — битая: рисовать по ней нечего.
    for image in &mut images {
        if image
            .media
            .as_deref()
            .is_some_and(|media| !archive.contains(media))
        {
            image.media = None;
        }
    }
    Ok(images)
}
