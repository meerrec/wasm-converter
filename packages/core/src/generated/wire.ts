// Сгенерировано из Rust (`crates/core/src/wire.rs`) через ts-rs. Не править руками.
// Пересобрать: pnpm gen:wire

export type PageSizeWire = "A4" | "A3" | "Letter" | "Legal";

export type OrientationWire = "portrait" | "landscape";

export type MarginsWire = { top: number, right: number, bottom: number, left: number, };

export type PdfOptionsWire = { pageSize?: PageSizeWire, orientation?: OrientationWire, margins?: MarginsWire, scale?: number, 
/**
 * Лист из настроек: интерфейс может экспортировать не тот, что на экране.
 */
sheetIndex?: number, };

export type ViewportWire = { scrollX: number, scrollY: number, width: number, height: number, 
/**
 * Зум, умноженный на DPR. Не задан — 1.0: интерфейс может не слать поле.
 */
scale: number, };

export type RenderOptionsWire = { showGrid: boolean, showHeaders: boolean, 
/**
 * Тёмное оформление; `false` — светлое, как в Excel.
 */
dark: boolean, };

export type SheetInfoWire = { name: string, index: number, hidden: boolean, 
/**
 * Размер листа в пикселях раскладки — им распоряжается полоса прокрутки.
 */
width: number, height: number, frozenCols: number, frozenRows: number, 
/**
 * Ширина и высота закреплённой части.
 */
frozenWidth: number, frozenHeight: number, };

export type PageInfoWire = { width: number, height: number, };

export type SlideInfoWire = { index: number, name: string, width: number, height: number, };

export type ImageInfoWire = { 
/**
 * id, которым помечены картинки листов и команды `Image` в кадре.
 */
id: number, 
/**
 * MIME-тип байтов — интерфейс собирает из них `Blob`.
 */
mime: string, 
/**
 * Длина байтов в `*_image_bytes`.
 */
byteLength: number, };

export type HyperlinkInfoWire = { 
/**
 * Адрес перехода: URL, почта, путь к файлу или ссылка внутри книги.
 */
target: string, 
/**
 * Подпись, которую показывает Excel (`display`).
 */
display: string | null, 
/**
 * Всплывающая подсказка (`tooltip`).
 */
tooltip: string | null, };

export type PaintStatsWire = { cmds: number, dropped: boolean, paintMs: number, };

export type BuildStatsWire = { 
/**
 * Кадр уложился в буфер. `false` — либо не было свободного слота ring'а,
 * либо буфер вызывающего меньше кадра (`bytes` скажет, насколько).
 */
written: boolean, cmds: number, 
/**
 * Размер кадра в байтах. По нему воркер понимает, что буфер мал, и растит его.
 */
bytes: number, buildMs: number, };

