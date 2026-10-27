//! ODS: электронная таблица ODF 1.2 в модель `doc-converter-xlsx`.
//!
//! Пакет ODF — это ZIP с `mimetype`, каталогом `META-INF/manifest.xml` и
//! частями `content.xml`/`styles.xml`. Листы лежат в
//! `office:body/office:spreadsheet/table:table`; каждый `<table:table>` — лист
//! книги, `<table:table-row>` — строка, `<table:table-cell>` — ячейка.
//!
//! Первый срез доводит книгу до модели [`Workbook`],
//! чтобы просмотр, `xlsx::paint` и PDF-экспорт работали без отдельной ветки на
//! формат — как это сделано для XLSB. Поддержаны значения всех типов ODF,
//! повторы строк и столбцов, объединения, ширины/высоты и частично числовые
//! форматы. Стили ячеек (шрифт, заливка, рамки), печать, картинки, диаграммы и
//! примечания — следующие срезы; см. TODO в [`styles`] и [`content`].
//!
//! # Ограничения первого среза
//!
//! - Формулы сохраняются текстом в синтаксисе ODF (`of:=SUM([.B2:.B3])`) без
//!   перевода в синтаксис Excel и без вычисления: значение рядом с формулой
//!   уже посчитано файлом и берётся как есть.
//! - `date`/`time` становятся [`CellValue::Number`](doc_converter_xlsx::CellValue)
//!   с серийным номером Excel; видимый вид задаёт числовой формат из
//!   `styles.xml`. Если формат не разобрался, ячейка покажет серийный номер —
//!   это осознанная потеря вида, а не значения (TODO: полный разбор форматов).
//! - Вложенные таблицы (`table:table` внутри ячейки) пропускаются: модель
//!   XLSX их не выражает.
//! - Скрытые листы пока считаются видимыми: видимость задаётся свойством
//!   `table:display` стиля таблицы, а слой стилей — следующий срез.

pub mod content;
pub mod styles;

use doc_converter_core::{Archive, ZipLimits};
use doc_converter_xlsx::{SharedStrings, Theme, Workbook};

use crate::error::{OdfError, Result};
use crate::manifest::{self, PackageKind};

/// Часть с содержимым документа: телом и автоматическими стилями.
pub const CONTENT_PART: &str = "content.xml";

/// Часть с общими стилями: именованные стили и числовые форматы.
pub const STYLES_PART: &str = "styles.xml";

/// Открыть ODS из сырых байтов.
///
/// `mimetype` обязан объявлять электронную таблицу; `.odt`/`.odp` дают
/// [`OdfError::Unsupported`], а не пустую книгу. Архив переходит во владение
/// разбора: модель книги собирается целиком, и открытый архив после `open` не
/// нужен.
///
/// # Errors
/// [`OdfError::Core`] — байты не ZIP, в пакете нет `content.xml` или нарушены
/// лимиты архива ([`ZipLimits`] по умолчанию);
/// [`OdfError::Malformed`] — `mimetype`/каталог/XML/ячейка нарушают ODF 1.2;
/// [`OdfError::Unsupported`] — пакет не ODS;
/// [`OdfError::Limit`] — повторы строк или ячеек вышли за лимиты Excel или
/// предел модели ([`content::MAX_CELLS_PER_SHEET`]).
pub fn open(bytes: Vec<u8>) -> Result<Workbook> {
    let mut archive = Archive::new(bytes, ZipLimits::default())?;
    let manifest = manifest::read(&mut archive)?;
    if manifest.kind != PackageKind::Ods {
        return Err(OdfError::Unsupported(format!(
            "`{}` packages are not supported yet: this slice reads ODS only",
            manifest.kind.extension()
        )));
    }

    let content = archive.part(CONTENT_PART)?;

    // Именованные стили и форматы лежат в styles.xml, автоматические — в
    // content.xml. Собираем их до тела: ячейка сразу получает индекс формата,
    // а не разрешает имя стиля задним числом.
    let mut book = styles::StyleBook::default();
    if archive.contains(STYLES_PART) {
        let bytes = archive.part(STYLES_PART)?;
        styles::collect(&bytes, STYLES_PART, &mut book)?;
    }
    styles::collect(&content, CONTENT_PART, &mut book)?;
    let cell_styles = book.freeze();

    let sheets = content::parse(&content, CONTENT_PART, &cell_styles)?;

    // Архив уходит из области видимости здесь: книга уже владеет всем, что ей
    // нужно. Строки встраиваются в ячейки, поэтому общая таблица пуста.
    Ok(Workbook::new(
        sheets,
        SharedStrings::default(),
        cell_styles.table().clone(),
        Theme::default(),
        false,
    ))
}
