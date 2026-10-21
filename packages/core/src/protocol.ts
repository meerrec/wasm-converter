// Wire-протокол Main ↔ Worker. Источник истины — этот файл;
// Rust-сторона описывает только payload (JSON / SAB), не конверт.

/** Формат документа, который умеет открыть воркер. */
export type DocFormat = 'xlsx' | 'docx';

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

/**
 * Чем кадр едет от сборщика к painter'у. Для хоста пути неразличимы: тот же
 * API, та же картинка, отличается только транспорт и, вслед за ним, скорость.
 *
 * - `sab` — ring из двух слотов в `SharedArrayBuffer`; доступен странице,
 *   отданной с COOP/COEP (`crossOriginIsolated`);
 * - `bytes` — два обычных буфера `ArrayBuffer`, которые воркер заполняет по
 *   очереди; работают всегда, в том числе в iframe без заголовков.
 */
export type FrameTransport = 'sab' | 'bytes';

/** Метрики одного кадра painter'а. Источник — Rust `PaintStats`. */
export interface PaintStats {
  cmds: number;
  /**
   * Кадр, который просили нарисовать, не нарисован. Что именно случилось,
   * зависит от транспорта: на `sab` ring был полон и промежуточный кадр
   * пропущен, на `bytes` готового кадра не оказалось вовсе. Общее одно: этот
   * кадр хосту не показывали, следующий покажут.
   */
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
 * Страница открытого документа. Приходит из Rust: размеры — в пикселях
 * раскладки, то есть до зума; по ним интерфейс строит прокрутку страницы, как
 * по размерам листа.
 */
export interface PageInfo {
  width: number;
  height: number;
}

/**
 * Картинка открытого документа: id, MIME-тип и длина байтов. Байты в список
 * не входят — их забирает `xlsx_image_bytes(id)` или `docx_image_bytes(id)`,
 * чтобы не копировать разом всю media. Тем же id помечены команды `Image`
 * в кадре.
 */
export interface ImageInfo {
  id: number;
  mime: string;
  byteLength: number;
}

// ── Фаза 2: прямой протокол Main ↔ Worker ───────────────────
// Paint-путь намеренно без request/response, чтобы кадр не ждал ответа.

/**
 * Ленивый PDF-модуль: URL его JS-модуля и wasm-бинаря.
 *
 * Оба приходят с main-потока: воркер собран бандлом, и относительный путь из
 * него ничего не значит. Не задан — `export-pdf` вернёт внятную ошибку.
 */
export interface PdfModuleUrls {
  /** JS-модуль `@meerrec/wasm-pdf`: воркер импортирует его по клику. */
  module: string;
  /** Бинарь для `init()`; без него wasm-bindgen ищет его рядом с модулем. */
  binary?: string | undefined;
}

/**
 * Ленивый DOCX-модуль: URL его JS-модуля и wasm-бинаря.
 *
 * Оба приходят с main-потока: воркер собран бандлом, и относительный путь из
 * него ничего не значит. Не задан — открытие `.docx` вернёт внятную ошибку.
 */
export interface DocxModuleUrls {
  /** JS-модуль `@meerrec/wasm-docx`: воркер импортирует его при открытии `.docx`. */
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
  /** Где лежит DOCX-модуль. Не задан — открыть документ не получится. */
  docx?: DocxModuleUrls | undefined;
  slotCapacity: number;
  /**
   * Транспорт кадра. `auto` (по умолчанию) — `sab`, если `SharedArrayBuffer`
   * доступен, иначе `bytes`; `sab` требует cross-origin isolation и падает
   * внятной ошибкой, если её нет. Задавать явно стоит только ради отладки и
   * тестов: оба пути first-class.
   */
  transport?: 'auto' | FrameTransport | undefined;
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
 *
 * От чего отсчитывается `viewport.y`, зависит от документа: у книги — от верха
 * листа, у DOCX — от ВЕРХА СТОПКИ страниц: кадр документа несёт сразу все
 * страницы, пересекающие окно, а не одну текущую.
 */
export interface RenderRequest {
  /** Лист книги — поле кадра XLSX. */
  sheet?: number;
  /**
   * Зазор между страницами стопки в пикселях раскладки — поле кадра DOCX; у
   * XLSX не читается. Зазор тот же, что в разметке просмотрщика: по нему Rust
   * ставит страницы друг под другом, и разойдись эти два зазора — кадр поехал
   * бы относительно прокрутки.
   */
  gap?: number;
  viewport: Viewport;
  config: RenderConfig;
}

/**
 * Открыть книгу или документ. Байты передаются переводом владения.
 *
 * `id` обязателен, хотя ответ на открытие и так один на запрос: ошибка
 * приходит общим тегом `error`, и без `id` main не отличил бы, какому из
 * параллельных открытий она адресована. Ответ `opened`/`docx-opened` несёт
 * тот же `id` — по нему просмотрщик находит свой ожидающий промис.
 */
export interface OpenMsg { type: 'open'; id: number; format: DocFormat; bytes: ArrayBuffer }
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

/** Формат текстовой выгрузки DOCX: какую из трёх функций модуля звать. */
export type DocxTextFormat = 'markdown' | 'html' | 'text';

/**
 * Экспорт открытого документа DOCX в PDF. Считается в ленивом PDF-модуле: он
 * разбирает документ сам, а байты получает те же, что пришли при открытии.
 *
 * Страницы DOCX — это страницы раскладки, поэтому `pageSize`, `orientation`,
 * `margins`, `scale` и `sheetIndex` не читаются: размер, поля и ориентация
 * берутся из `w:sectPr` документа.
 */
export interface ExportDocxPdfMsg {
  type: 'export-docx-pdf';
  id: number;
  options: PdfOptions;
}

/**
 * Экспорт открытого документа DOCX в текст: markdown, HTML или обычный текст.
 * Считается в ленивом DOCX-модуле, который воркер загрузил ещё при открытии
 * документа.
 */
export interface ExportDocxTextMsg {
  type: 'export-docx-text';
  id: number;
  format: DocxTextFormat;
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
  | ExportPdfMsg
  | ExportDocxPdfMsg
  | ExportDocxTextMsg;

/**
 * Воркер инициализирован. `sab` в сообщении нет намеренно: ring живёт в
 * воркере, main им не пользуется, а готовность воркера и наличие
 * `SharedArrayBuffer` — независимые факты (без COOP/COEP второго просто нет).
 */
export interface ReadyMsg {
  type: 'ready';
  /** Каким путём поедут кадры. */
  transport: FrameTransport;
  /** Размер одного слота: у `sab` — слота ring'а, у `bytes` — буфера. */
  slotCapacity: number;
  /** Сколько байт отдано под транспорт кадров — у обоих путей поровну. */
  totalBytes: number;
  /** Cross-origin isolation страницы: без неё `SharedArrayBuffer` недоступен. */
  crossOriginIsolated: boolean;
}

/** `ReadyMsg` без тега: то, что просмотрщик знает о воркере после `init`. */
export type ReadyInfo = Omit<ReadyMsg, 'type'>;
export interface TickMsg { type: 'tick'; stats: PaintStats }
/**
 * Книга разобрана: листы и их размеры. `id` — id запроса `open`: по нему
 * просмотрщик разрешает именно свой промис, а не все ожидающие открытия.
 */
export interface OpenedMsg { type: 'opened'; id: number; sheets: SheetInfo[] }
/**
 * Документ разложен: размеры страниц.
 *
 * Тег свой, а не `opened` с другим полем: сообщения с общим `type` в union не
 * сужаются, и ветка `opened` на main-стороне потеряла бы `sheets`.
 */
export interface DocxOpenedMsg { type: 'docx-opened'; id: number; pages: PageInfo[] }
/** Ответ на `hit-test`: строка и столбец либо ничего, если точка мимо. */
export interface HitMsg { type: 'hit'; id: number; cell: [number, number] | null }
/** Ответ на `hyperlink-at`: ссылка под точкой либо ничего. */
export interface HyperlinkMsg { type: 'hyperlink'; id: number; link: HyperlinkInfo | null }
export interface PngMsg { type: 'png'; id: number; bytes: Uint8Array }
/**
 * Готовый PDF. Байты передаются переводом владения — копии нет.
 *
 * Тег общий у XLSX- и DOCX-путей: тело совпадает, а воркеры у просмотрщиков
 * свои, так что сужать нечего.
 */
export interface PdfMsg { type: 'pdf'; id: number; bytes: Uint8Array }
/** Текстовая выгрузка документа в том формате, что запросил клиент. */
export interface DocxTextMsg { type: 'docx-text'; id: number; text: string }
/**
 * Стадия, на которой воркер потерпел неудачу. Код отличает сбой инициализации,
 * после которого воркер бесполезен, от ошибки одного запроса, после которой
 * остальные запросы продолжают работать.
 */
export type WorkerErrorCode = 'init' | 'open' | 'export' | 'internal';

/**
 * Ошибка воркера.
 *
 * `id` — запрос, который её вызвал: по нему просмотрщик отклоняет только свой
 * промис, не трогая параллельные. Ошибки без `id` — сбой воркера целиком
 * (инициализация, повторный `init`): ответа не будет ни на один запрос, и
 * просмотрщик отклоняет все ожидающие.
 */
export interface WorkerErrorMsg {
  type: 'error';
  message: string;
  /** id запроса, если ошибка адресная. */
  id?: number;
  /**
   * Стадия сбоя: `init` — воркер не поднялся, `open`/`export` — один запрос,
   * `internal` — нарушение протокола (например, повторный `init`).
   */
  code?: WorkerErrorCode;
}

export type OutMsg =
  | ReadyMsg
  | TickMsg
  | PngMsg
  | PdfMsg
  | OpenedMsg
  | DocxOpenedMsg
  | HitMsg
  | HyperlinkMsg
  | DocxTextMsg
  | WorkerErrorMsg;
