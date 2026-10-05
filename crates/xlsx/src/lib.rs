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
    Threshold, ThresholdKind, Workbook, WorkbookImage, Worksheet, WorksheetBuilder, WorksheetMeta,
    THEME_COLOR_COUNT,
};
pub use paint::{build as paint_sheet, PaintOptions, Viewport};
pub use sheet_meta::{Hyperlink, HyperlinkTarget, Merges, Pane, PaneKind, PaneState, SheetView};
pub use strings::SharedStrings;
pub use workbook::WorkbookMeta;

use std::collections::HashMap;

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

/// Предел на одну media-часть: картинка крупнее в реестр не попадает, и её
/// [`SheetImage`] остаётся без `image_id` — рисовать такую нечем.
///
/// Число выбрано с запасом: обложки и фото в книгах редко переваливают за
/// десяток мегабайт, а вот медиа-часть на сотни мегабайт утопила бы wasm-память.
pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// Предел на все media-части книги: после него части не читаются вовсе.
///
/// Книга открывается целиком, и её архив уже держится в памяти на время
/// разбора; реестр добавляет к этому копию media. Предел ограничивает эту
/// копию, чтобы книга с десятками крупных картинок не съела память воркера.
pub const MAX_TOTAL_MEDIA_BYTES: usize = 128 * 1024 * 1024;

/// MIME-тип по расширению media-части: он уезжает в JS, где из байтов
/// собирается `Blob`.
fn mime_of(part: &str) -> &'static str {
    match part.rsplit_once('.').map(|(_, ext)| ext) {
        Some(ext) if ext.eq_ignore_ascii_case("png") => "image/png",
        Some(ext) if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") => {
            "image/jpeg"
        }
        Some(ext) if ext.eq_ignore_ascii_case("gif") => "image/gif",
        Some(ext) if ext.eq_ignore_ascii_case("bmp") => "image/bmp",
        Some(ext) if ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff") => {
            "image/tiff"
        }
        Some(ext) if ext.eq_ignore_ascii_case("webp") => "image/webp",
        Some(ext) if ext.eq_ignore_ascii_case("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Реестр media-частей книги.
///
/// Байты читаются при `open()`: архив живёт только внутри него, а рисующему
/// нужны байты уже после. Одна часть — одна запись, сколько бы картинок на неё
/// ни ссылалось; id выдаются по порядку первого упоминания, поэтому общий для
/// Rust и JS и стабильный для книги.
#[derive(Debug)]
struct MediaRegistry {
    images: Vec<WorkbookImage>,
    /// Часть → id; `None` — часть прочитана, но не сохранена (не влезла
    /// в предел). Отрицательный ответ кэшируется, чтобы не читать её снова.
    by_part: HashMap<String, Option<u32>>,
    bytes_used: usize,
    max_image_bytes: usize,
    max_total_bytes: usize,
}

impl MediaRegistry {
    fn new(max_image_bytes: usize, max_total_bytes: usize) -> Self {
        Self {
            images: Vec::new(),
            by_part: HashMap::new(),
            bytes_used: 0,
            max_image_bytes,
            max_total_bytes,
        }
    }

    /// Прочитать media-часть и вернуть её id; `None` — части нет или она
    /// не уложилась в пределы.
    ///
    /// # Errors
    /// Если часть есть в архиве, но не читается из него.
    fn register(&mut self, archive: &mut Archive, media: Option<&str>) -> Result<Option<u32>> {
        let Some(media) = media else {
            return Ok(None);
        };
        if let Some(id) = self.by_part.get(media) {
            return Ok(*id);
        }

        let bytes = archive.read(media)?;
        let fits = bytes.len() <= self.max_image_bytes
            && self.bytes_used + bytes.len() <= self.max_total_bytes;
        let id = if fits {
            u32::try_from(self.images.len()).ok()
        } else {
            None
        };
        if let Some(id) = id {
            self.bytes_used += bytes.len();
            self.images.push(WorkbookImage {
                id,
                media: media.to_owned(),
                mime: mime_of(media).to_owned(),
                bytes,
            });
        }
        self.by_part.insert(media.to_owned(), id);
        Ok(id)
    }
}

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

    let mut media = MediaRegistry::new(MAX_IMAGE_BYTES, MAX_TOTAL_MEDIA_BYTES);
    let mut sheets = Vec::with_capacity(catalog.sheets.len());
    for meta in catalog.sheets {
        // Связи листа нужны гиперссылкам и изображениям: их цели живут
        // в отдельных частях.
        let rels = read_rels(&mut archive, &meta.part)?;
        let mut content = worksheet::parse(&archive.read(&meta.part)?, &meta.part, rels.as_ref())?;
        // Чертёж — отдельная часть: сам лист на него только ссылается.
        content.images = read_images(&mut archive, &meta.part, rels.as_ref())?;
        // Байты media читаются здесь же: дальше архив закрывается.
        for image in &mut content.images {
            let id = media.register(&mut archive, image.media.as_deref())?;
            image.image_id = id;
        }
        sheets.push(Sheet::new(meta, content));
    }

    Ok(
        Workbook::new(sheets, shared_strings, style_table, theme, catalog.date1904)
            .with_images(media.images),
    )
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

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    /// Архив из пар «имя — содержимое»: пакет собирается в памяти, как Excel.
    fn archive(entries: &[(&str, &[u8])]) -> Archive {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, body) in entries {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(body).unwrap();
            }
            zip.finish().unwrap();
        }
        Archive::new(buf).unwrap()
    }

    /// Повторная ссылка на ту же часть получает тот же id: картинок две,
    /// а запись в реестре одна.
    #[test]
    fn registry_dedups_repeated_parts() {
        let mut archive = archive(&[
            ("xl/media/image1.png", b"png"),
            ("xl/media/image2.jpeg", b"jpeg"),
        ]);
        let mut registry = MediaRegistry::new(1024, 1024);

        let first = registry
            .register(&mut archive, Some("xl/media/image1.png"))
            .unwrap();
        let second = registry
            .register(&mut archive, Some("xl/media/image2.jpeg"))
            .unwrap();
        assert_eq!((first, second), (Some(0), Some(1)));
        assert_eq!(
            registry
                .register(&mut archive, Some("xl/media/image1.png"))
                .unwrap(),
            Some(0)
        );

        assert_eq!(registry.images.len(), 2);
        assert_eq!(registry.images[0].bytes, b"png");
        assert_eq!(registry.images[0].mime, "image/png");
        assert_eq!(registry.images[1].mime, "image/jpeg");
    }

    /// Часть крупнее предела не сохраняется, но и места в бюджете не занимает:
    /// следующая, мелкая, всё равно попадает в реестр.
    #[test]
    fn registry_skips_parts_over_the_image_limit() {
        let mut archive = archive(&[
            ("xl/media/large.png", b"large"),
            ("xl/media/small.png", b"ok"),
        ]);
        let mut registry = MediaRegistry::new(4, 1024);

        assert_eq!(
            registry
                .register(&mut archive, Some("xl/media/large.png"))
                .unwrap(),
            None
        );
        assert_eq!(
            registry
                .register(&mut archive, Some("xl/media/small.png"))
                .unwrap(),
            Some(0)
        );
        // Отказ кэшируется: повторная ссылка снова читать не будет.
        assert_eq!(
            registry
                .register(&mut archive, Some("xl/media/large.png"))
                .unwrap(),
            None
        );
        assert_eq!(registry.images.len(), 1);
    }

    /// Исчерпанный бюджет книги закрывает реестр: дальнейшие части не читаются.
    #[test]
    fn registry_stops_when_the_book_budget_is_gone() {
        let mut archive = archive(&[("xl/media/a.png", b"aaa"), ("xl/media/b.png", b"bb")]);
        let mut registry = MediaRegistry::new(1024, 3);

        assert_eq!(
            registry
                .register(&mut archive, Some("xl/media/a.png"))
                .unwrap(),
            Some(0)
        );
        assert_eq!(
            registry
                .register(&mut archive, Some("xl/media/b.png"))
                .unwrap(),
            None
        );
        assert_eq!(registry.images.len(), 1);
    }

    /// Битая или отсутствующая ссылка — не запись реестра.
    #[test]
    fn registry_without_media_stores_nothing() {
        let mut archive = archive(&[("xl/media/image1.png", b"png")]);
        let mut registry = MediaRegistry::new(1024, 1024);

        assert_eq!(registry.register(&mut archive, None).unwrap(), None);
        assert!(registry.images.is_empty());
    }

    #[test]
    fn mime_of_maps_known_extensions() {
        assert_eq!(mime_of("xl/media/image1.png"), "image/png");
        assert_eq!(mime_of("xl/media/photo.JPEG"), "image/jpeg");
        assert_eq!(mime_of("xl/media/anim.gif"), "image/gif");
        assert_eq!(mime_of("xl/media/no-extension"), "application/octet-stream");
        assert_eq!(mime_of("xl/media/scan.tiff"), "image/tiff");
    }
}
