// Wire-протокол Main ↔ Worker. Источник истины — этот файл;
// Rust-сторона описывает только payload (JSON / SAB), не конверт.

export interface Viewport { x: number; y: number; w: number; h: number; scale: number }

export interface RenderConfig {
  showGrid: boolean;
  showHeaders: boolean;
  theme: 'light' | 'dark';
  dpr: number;
}

export type CellRef = { kind: 'cell'; sheet: number; row: number; col: number };
export type TextPos = { kind: 'text'; page: number; offset: number };
export type HitResult = CellRef | TextPos;

/**
 * Гиперссылка под точкой. Источник — Rust `HyperlinkInfo`.
 *
 * `target` — адрес перехода; `display` и `tooltip` — подпись и подсказка, как
 * их объявил файл, либо `null`, если их нет.
 */
export interface HyperlinkInfo {
  target: string;
  display: string | null;
  tooltip: string | null;
}

export interface PdfOptions {
  pageSize?: 'A4' | 'A3' | 'Letter' | 'Legal';
  orientation?: 'portrait' | 'landscape';
  margins?: { top: number; right: number; bottom: number; left: number };
  scale?: number;
  sheetIndex?: number;
  /**
   * Экспортировать всю книгу одним PDF: листы идут подряд, закладка — на
   * каждый. `sheetIndex` в этом режиме не участвует — он выбирает один лист.
   */
  allSheets?: boolean;
}

export type WorkerRequest =
  | { id: number; type: 'init';      payload: { canvas: OffscreenCanvas } }
  | { id: number; type: 'resize';    payload: { width: number; height: number; dpr: number } }
  | { id: number; type: 'open';      payload: { format: 'docx' | 'xlsx'; bytes: ArrayBuffer } }
  | { id: number; type: 'render';    payload: { sheet?: number; page?: number; viewport: Viewport; config: RenderConfig } }
  | { id: number; type: 'hitTest';   payload: { x: number; y: number } }
  | { id: number; type: 'exportPdf'; payload: PdfOptions }
  | { id: number; type: 'exportPng'; payload: { dpi: number } }
  | { id: number; type: 'cancel';    payload: { targetId: number } }
  | { id: number; type: 'dispose' };

export interface FrameTimings { buildMs: number; paintMs: number }

/** Метрики одного кадра painter'а. Источник — Rust `PaintStats`. */
export interface PaintStats {
  cmds: number;
  dropped: boolean;
  paintMs: number;
  frameId: number;
  /** Сколько заняла сборка DisplayList. Есть только у документов. */
  buildMs?: number;
}

/**
 * Лист открытой книги. Приходит из Rust: размеры — в пикселях раскладки,
 * то есть до зума.
 */
export interface SheetInfo {
  name: string;
  index: number;
  hidden: boolean;
  width: number;
  height: number;
  frozenCols: number;
  frozenRows: number;
  frozenWidth: number;
  frozenHeight: number;
}

/**
 * Картинка книги: id, MIME-тип и длина байтов. Байты в список не входят —
 * их забирает `xlsx_image_bytes(id)`, чтобы не копировать разом всю media
 * книги. Тем же id помечены команды `Image` в кадре.
 */
export interface ImageInfo {
  id: number;
  mime: string;
  byteLength: number;
}

// ── Фаза 2: прямой протокол Main ↔ Worker ───────────────────
// Отдельно от RPC-конверта выше: paint-путь намеренно без request/response,
// чтобы кадр не ждал ответа.

/**
 * Ленивый PDF-модуль: URL его JS-модуля и wasm-бинаря.
 *
 * Оба приходят с main-потока: воркер собран бандлом, и относительный путь из
 * него ничего не значит. Не задан — `export-pdf` вернёт внятную ошибку.
 */
export interface PdfModuleUrls {
  /** JS-модуль `@doc-converter/wasm-pdf`: воркер импортирует его по клику. */
  module: string;
  /** Бинарь для `init()`; без него wasm-bindgen ищет его рядом с модулем. */
  binary?: string | undefined;
}

export interface InitMsg {
  type: 'init';
  canvas: OffscreenCanvas;
  /** Без значения wasm-bindgen резолвит .wasm относительно своего модуля. */
  wasmUrl?: string | undefined;
  /** Где лежит PDF-модуль. Не задан — экспорт ответит ошибкой. */
  pdf?: PdfModuleUrls | undefined;
  slotCapacity: number;
}

export interface ResizeMsg { type: 'resize'; cssW: number; cssH: number; dpr: number }
export interface BitmapMsg { type: 'bitmap'; id: number; bitmap: ImageBitmap }
export interface DropBitmapMsg { type: 'drop-bitmap'; id: number }

/**
 * Полезная нагрузка `render`.
 *
 * Единицы намеренно разные, и это важно: `viewport.x/y` — в пикселях раскладки
 * (то есть до зума), а `w/h` — в физических пикселях canvas. `scale` связывает
 * их: он равен зуму, умноженному на плотность пикселей экрана.
 */
export interface RenderRequest {
  sheet?: number;
  page?: number;
  viewport: Viewport;
  config: RenderConfig;
}

/** Открыть книгу. Байты передаются переводом владения. */
export interface OpenMsg { type: 'open'; format: 'xlsx'; bytes: ArrayBuffer }
export interface CloseMsg { type: 'close' }
export interface RenderMsg { type: 'render'; req?: RenderRequest }

/** Ячейка под точкой: координаты — физические пиксели холста. */
export interface HitTestMsg {
  type: 'hit-test';
  id: number;
  sheet: number;
  x: number;
  y: number;
  viewport: Viewport;
  config: RenderConfig;
}
export interface ExportPngMsg { type: 'export-png'; id: number }

/**
 * Экспорт листа в PDF. Считается в воркере: там же лежит разобранная книга.
 * `sheet` — из тех же единиц, что у `render`, то есть индекс в книге.
 */
export interface ExportPdfMsg {
  type: 'export-pdf';
  id: number;
  sheet: number;
  options: PdfOptions;
}

/** Гиперссылка под точкой: те же единицы, что у `hit-test`. */
export interface HyperlinkAtMsg {
  type: 'hyperlink-at';
  id: number;
  sheet: number;
  x: number;
  y: number;
  viewport: Viewport;
  config: RenderConfig;
}

export type InMsg =
  | InitMsg
  | ResizeMsg
  | BitmapMsg
  | DropBitmapMsg
  | OpenMsg
  | CloseMsg
  | RenderMsg
  | HitTestMsg
  | HyperlinkAtMsg
  | ExportPngMsg
  | ExportPdfMsg;

export interface ReadyMsg {
  type: 'ready';
  sab: SharedArrayBuffer;
  slotCapacity: number;
  totalBytes: number;
}
export interface TickMsg { type: 'tick'; stats: PaintStats }
/** Книга разобрана: листы и их размеры. */
export interface OpenedMsg { type: 'opened'; sheets: SheetInfo[] }
/** Ответ на `hit-test`: строка и столбец либо ничего, если точка мимо. */
export interface HitMsg { type: 'hit'; id: number; cell: [number, number] | null }
/** Ответ на `hyperlink-at`: ссылка под точкой либо ничего. */
export interface HyperlinkMsg { type: 'hyperlink'; id: number; link: HyperlinkInfo | null }
export interface PngMsg { type: 'png'; id: number; bytes: Uint8Array }
/** Готовый PDF. Байты передаются переводом владения — копии нет. */
export interface PdfMsg { type: 'pdf'; id: number; bytes: Uint8Array }
export interface WorkerErrorMsg { type: 'error'; message: string }

export type OutMsg =
  | ReadyMsg
  | TickMsg
  | PngMsg
  | PdfMsg
  | OpenedMsg
  | HitMsg
  | HyperlinkMsg
  | WorkerErrorMsg;

export type WorkerResponse =
  | { id: number; type: 'ok';    result?: unknown }
  | { id: number; type: 'error'; message: string; stack?: string }
  | { id: number; type: 'ready'; meta: { pages?: number; sheets?: string[] } }
  | { id: number; type: 'pdf';   bytes: Uint8Array }
  | { id: number; type: 'png';   bytes: Uint8Array }
  | { id: number; type: 'hit';   ref: HitResult | null }
  | { id: number; type: 'tick';  frameId: number; timings: FrameTimings };
