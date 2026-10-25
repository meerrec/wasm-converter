// Сгенерировано из Rust (`crates/core/src/wire.rs`) через ts-rs. Не править руками.
// Пересобрать: pnpm gen:wire

export type PageSizeWire = "A4" | "A3" | "Letter" | "Legal";

export type OrientationWire = "portrait" | "landscape";

export type MarginsWire = { top: number, right: number, bottom: number, left: number, };

export type PdfOptionsWire = { pageSize?: PageSizeWire, orientation?: OrientationWire, margins?: MarginsWire, scale?: number, 
/**
 * Уместить лист на одну страницу: `fitToWidth = fitToHeight = 1`.
 *
 * Явные [`Self::fit_to_width`]/[`Self::fit_to_height`] сильнее флага.
 */
fitToPage?: boolean, 
/**
 * Сколько страниц по ширине; `0` — без ограничения.
 */
fitToWidth?: number, 
/**
 * Сколько страниц по высоте; `0` — без ограничения.
 */
fitToHeight?: number, 
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

export type BBoxWire = { x: number, y: number, width: number, height: number, };

export type TextDirectionWire = "ltr" | "rtl";

export type TextAlignWire = "left" | "center" | "right";

export type BlockKindWire = "paragraph" | "heading" | "tableCell";

export type TextStyleWire = { 
/**
 * Гарнитура, которая реально ушла в `ctx.font`; `None` — умолчание реестра.
 */
fontFamily: string | null, 
/**
 * Кегль в пикселях раскладки.
 */
sizePx: number, bold: boolean, italic: boolean, underline: boolean, 
/**
 * Цвет `RRGGBB`; `None` — цвет по умолчанию.
 */
color: number | null, };

export type TableContextWire = { 
/**
 * `NodeId` таблицы: различает вложенные таблицы.
 */
tableId: number, 
/**
 * `NodeId` ячейки.
 */
cellId: number, 
/**
 * Логический индекс строки в таблице (0-based).
 */
row: number, 
/**
 * Логический индекс колонки сетки (0-based); `w:bidiVisual` его не меняет.
 */
col: number, 
/**
 * Объединение по горизонтали (`w:gridSpan`).
 */
colSpan: number, 
/**
 * Объединение по вертикали (`w:vMerge`).
 */
rowSpan: number, 
/**
 * Строка объявлена заголовком (`w:tblHeader`).
 */
isHeader: boolean, 
/**
 * Строка повторена как заголовок на странице-продолжении.
 */
isRepeatedHeader: boolean, 
/**
 * Ячейка — продолжение вертикального объединения: своего текста у неё нет.
 */
isVmergeContinuation: boolean, };

export type TextRunWire = { 
/**
 * Стабильный ключ вида `p12:l3:f0`.
 */
id: string, 
/**
 * Прямоугольник самого фрагмента в пикселях раскладки.
 */
bbox: BBoxWire, 
/**
 * Прямоугольник всей строки: по нему строится визуальный span.
 */
lineRect: BBoxWire, text: string, style: TextStyleWire, 
/**
 * 0-based номер страницы.
 */
page: number, lineIndex: number, fragmentIndex: number, 
/**
 * Индекс абзаца в теле документа (0-based) — цель правки run'а;
 * `None` — текст вне тела (ячейка таблицы, колонтитул) или фрагмент без
 * модельного адреса (вычисленное поле).
 */
paragraphIndex: number | null, 
/**
 * Индекс run'а в потоке абзаца (0-based) — цель правки run'а;
 * `None` — у фрагмента нет правимого run'а.
 */
runIndex: number | null, 
/**
 * Сквозной логический порядок по документу.
 */
readingOrder: number, direction: TextDirectionWire, 
/**
 * Уровень UAX #9.
 */
bidiLevel: number, textAlign: TextAlignWire, 
/**
 * `NodeId` абзаца или ячейки — группировка для ARIA.
 */
nodeId: number, blockKind: BlockKindWire, 
/**
 * Уровень заголовка 1–9; `None` — не заголовок.
 */
headingLevel: number | null, 
/**
 * Табличный контекст; `None` — текст вне таблицы.
 */
table: TableContextWire | null, };

export type PageRunsWire = { page: number, start: number, count: number, };

export type TextLayerWire = { runs: Array<TextRunWire>, pageMap: Array<PageRunsWire>, };

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

