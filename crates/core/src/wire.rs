//! Payload'ы wire-протокола Main ↔ Worker — единственный источник истины.
//!
//! Конверт (`InMsg`/`OutMsg`) остаётся ручным в `packages/core/src/protocol.ts`:
//! в нём живут `OffscreenCanvas`, `ImageBitmap` и `ArrayBuffer`, которым в Rust
//! делать нечего. Всё, что пересекает границу wasm как данные, описано здесь,
//! а TS-типы для них генерируются примером [`export_wire`] (ADR-0028).
//!
//! Поля — camelCase: на Rust-стороне за это отвечает `serde(rename_all)`, а
//! ts-rs повторяет переименование в сгенерированный TS. `Option` на входе
//! помечен `#[ts(optional)]` — интерфейс вправе не слать поле вовсе; на выходе
//! `Option` остаётся `T | null`, потому что `serde_wasm_bindgen` отдаёт
//! отсутствующее поле как `undefined`, а `HyperlinkInfoWire` сериализуется с
//! `serialize_missing_as_null(true)`.
//!
//! [`export_wire`]: ../../examples/export_wire.rs

use serde::{Deserialize, Serialize};

/// Окно — то, что приходит из воркера. Смещение — пиксели раскладки (до
/// зума), `width`/`height` — физические пиксели canvas.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ViewportWire {
    #[serde(default)]
    pub scroll_x: f32,
    #[serde(default)]
    pub scroll_y: f32,
    pub width: f32,
    pub height: f32,
    /// Зум, умноженный на DPR. Не задан — 1.0: интерфейс может не слать поле.
    #[serde(default = "one")]
    pub scale: f32,
}

/// Настройки рисования XLSX. Поля необязательны: интерфейс может прислать
/// только то, что меняет, а остальное остаётся как в Excel.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RenderOptionsWire {
    #[serde(default = "yes")]
    pub show_grid: bool,
    #[serde(default = "yes")]
    pub show_headers: bool,
    /// Тёмное оформление; `false` — светлое, как в Excel.
    #[serde(default)]
    pub dark: bool,
}

/// Описание листа для интерфейса: имя, размеры и закрепления.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SheetInfoWire {
    pub name: String,
    pub index: usize,
    pub hidden: bool,
    /// Размер листа в пикселях раскладки — им распоряжается полоса прокрутки.
    pub width: f32,
    pub height: f32,
    pub frozen_cols: u32,
    pub frozen_rows: u32,
    /// Ширина и высота закреплённой части.
    pub frozen_width: f32,
    pub frozen_height: f32,
}

/// Описание страницы для интерфейса: размеры в пикселях раскладки — по ним
/// интерфейс строит прокрутку, как по размерам листа XLSX.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PageInfoWire {
    pub width: f32,
    pub height: f32,
}

/// Прямоугольник в пикселях раскладки (до зума и DPR).
///
/// Отдельный тип, а не `render::Rect`: wire не должен зависеть от крейта
/// рендера, а поля совпадают с формой, которую ждёт TS.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BBoxWire {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Направление строки по UAX #9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TextDirectionWire {
    Ltr,
    Rtl,
}

/// Физическое выравнивание строки внутри её полосы набора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TextAlignWire {
    Left,
    Center,
    Right,
}

/// Что за блок породил run — от этого зависит ARIA-роль.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum BlockKindWire {
    Paragraph,
    Heading,
    TableCell,
}

/// Стиль знака, которым нарисован run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TextStyleWire {
    /// Гарнитура, которая реально ушла в `ctx.font`; `None` — умолчание реестра.
    pub font_family: Option<String>,
    /// Кегль в пикселях раскладки.
    pub size_px: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Цвет `RRGGBB`; `None` — цвет по умолчанию.
    pub color: Option<u32>,
}

/// Табличный контекст run — для ARIA `table/row/cell/columnheader`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TableContextWire {
    /// `NodeId` таблицы: различает вложенные таблицы.
    pub table_id: u32,
    /// `NodeId` ячейки.
    pub cell_id: u32,
    /// Логический индекс строки в таблице (0-based).
    pub row: u32,
    /// Логический индекс колонки сетки (0-based); `w:bidiVisual` его не меняет.
    pub col: u32,
    /// Объединение по горизонтали (`w:gridSpan`).
    pub col_span: u32,
    /// Объединение по вертикали (`w:vMerge`).
    pub row_span: u32,
    /// Строка объявлена заголовком (`w:tblHeader`).
    pub is_header: bool,
    /// Строка повторена как заголовок на странице-продолжении.
    pub is_repeated_header: bool,
    /// Ячейка — продолжение вертикального объединения: своего текста у неё нет.
    pub is_vmerge_continuation: bool,
}

/// Позиционированный текстовый run — единица `TextLayer`.
///
/// `bbox` и `line_rect` приходят из layout и не пересчитываются в TS: один
/// источник истины для геометрии. `reading_order` — логический порядок (для
/// AT и копирования), а не визуальный: визуальный порядок RTL даёт браузер.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TextRunWire {
    /// Стабильный ключ вида `p12:l3:f0`.
    pub id: String,
    /// Прямоугольник самого фрагмента в пикселях раскладки.
    pub bbox: BBoxWire,
    /// Прямоугольник всей строки: по нему строится визуальный span.
    pub line_rect: BBoxWire,
    pub text: String,
    pub style: TextStyleWire,
    /// 0-based номер страницы.
    pub page: u32,
    pub line_index: u32,
    pub fragment_index: u32,
    /// Индекс абзаца в теле документа (0-based) — цель правки run'а;
    /// `None` — текст вне тела (ячейка таблицы, колонтитул) или фрагмент без
    /// модельного адреса (вычисленное поле).
    pub paragraph_index: Option<u32>,
    /// Индекс run'а в потоке абзаца (0-based) — цель правки run'а;
    /// `None` — у фрагмента нет правимого run'а.
    pub run_index: Option<u32>,
    /// Сквозной логический порядок по документу.
    pub reading_order: u32,
    pub direction: TextDirectionWire,
    /// Уровень UAX #9.
    pub bidi_level: u8,
    pub text_align: TextAlignWire,
    /// `NodeId` абзаца или ячейки — группировка для ARIA.
    pub node_id: u32,
    pub block_kind: BlockKindWire,
    /// Уровень заголовка 1–9; `None` — не заголовок.
    pub heading_level: Option<u8>,
    /// Табличный контекст; `None` — текст вне таблицы.
    pub table: Option<TableContextWire>,
}

/// Runs одной страницы: срез в общем `runs`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PageRunsWire {
    pub page: u32,
    pub start: u32,
    pub count: u32,
}

/// `TextLayer` документа: плоский список runs и карта страниц.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TextLayerWire {
    pub runs: Vec<TextRunWire>,
    pub page_map: Vec<PageRunsWire>,
}

/// Описание слайда для интерфейса: индекс, имя (`p:cSld name`) и размеры в
/// пикселях раскладки — по ним интерфейс показывает счётчик слайдов и строит
/// навигацию, как по размерам страниц DOCX.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SlideInfoWire {
    pub index: usize,
    pub name: String,
    pub width: f32,
    pub height: f32,
}

/// Картинка в том виде, в каком её видит интерфейс: id, MIME и размер.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImageInfoWire {
    /// id, которым помечены картинки листов и команды `Image` в кадре.
    pub id: u32,
    /// MIME-тип байтов — интерфейс собирает из них `Blob`.
    pub mime: String,
    /// Длина байтов в `*_image_bytes`.
    pub byte_length: u32,
}

/// Гиперссылка в том виде, в каком её видит интерфейс.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HyperlinkInfoWire {
    /// Адрес перехода: URL, почта, путь к файлу или ссылка внутри книги.
    pub target: String,
    /// Подпись, которую показывает Excel (`display`).
    pub display: Option<String>,
    /// Всплывающая подсказка (`tooltip`).
    pub tooltip: Option<String>,
}

/// Правка ячейки книги, как её ввёл интерфейс: сырой текст вместо значения.
///
/// Снимок правок едет между wasm-модулями: просмотрщик держит их в книге, а
/// экспортёр PDF разбирает байты заново и прикладывает снимок сам. К конверту
/// Main ↔ Worker отношения не имеет — этот тип не входит в `InMsg`/`OutMsg`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CellEditWire {
    /// Индекс листа в книге (`Workbook::sheets`).
    pub sheet: u32,
    /// Адрес ячейки, 0-based: `A1` — это `(0, 0)`.
    pub row: u32,
    pub col: u32,
    /// Сырой текст, как введён.
    pub text: String,
}

/// Правка run'а документа DOCX, как её ввёл интерфейс: новый текст вместо
/// содержимого `w:t`.
///
/// Снимок правок едет между wasm-модулями: просмотрщик держит их в документе,
/// а экспортёр PDF разбирает байты заново и прикладывает снимок сам. К конверту
/// Main ↔ Worker отношения не имеет — этот тип не входит в `InMsg`/`OutMsg`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocxRunEditWire {
    /// Индекс абзаца в теле документа (`Document::body.items`), 0-based.
    pub paragraph: u32,
    /// Индекс run'а в потоке абзаца, 0-based.
    pub run: u32,
    /// Новый текст run'а, как введён.
    pub text: String,
}

/// Метрики одного кадра painter'а. `frameId` присваивает воркер: Rust про
/// номера кадров ничего не знает.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PaintStatsWire {
    pub cmds: u32,
    pub dropped: bool,
    pub paint_ms: f64,
}

/// Что вышло из попытки собрать кадр. Общая форма для XLSX и DOCX.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BuildStatsWire {
    /// Кадр уложился в буфер. `false` — либо не было свободного слота ring'а,
    /// либо буфер вызывающего меньше кадра (`bytes` скажет, насколько).
    pub written: bool,
    pub cmds: u32,
    /// Размер кадра в байтах. По нему воркер понимает, что буфер мал, и растит его.
    pub bytes: u32,
    pub build_ms: f64,
}

/// Настройки экспорта из интерфейса: имена полей — как у TS `PdfOptions`.
/// Все поля необязательны; чего нет — берётся умолчание `PdfOptions`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PdfOptionsWire {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub page_size: Option<PageSizeWire>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub orientation: Option<OrientationWire>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub margins: Option<MarginsWire>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scale: Option<f32>,
    /// Уместить лист на одну страницу: `fitToWidth = fitToHeight = 1`.
    ///
    /// Явные [`Self::fit_to_width`]/[`Self::fit_to_height`] сильнее флага.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub fit_to_page: Option<bool>,
    /// Сколько страниц по ширине; `0` — без ограничения.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub fit_to_width: Option<u32>,
    /// Сколько страниц по высоте; `0` — без ограничения.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub fit_to_height: Option<u32>,
    /// Лист из настроек: интерфейс может экспортировать не тот, что на экране.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub sheet_index: Option<usize>,
}

/// Размер страницы: строки те же, что в TS-объединении `PdfOptions.pageSize`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PageSizeWire {
    #[serde(rename = "A4")]
    A4,
    #[serde(rename = "A3")]
    A3,
    Letter,
    Legal,
}

/// Ориентация страницы: в TS она записана строчными.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum OrientationWire {
    Portrait,
    Landscape,
}

/// Поля страницы в миллиметрах.
#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MarginsWire {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}
