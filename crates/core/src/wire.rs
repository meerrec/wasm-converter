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
