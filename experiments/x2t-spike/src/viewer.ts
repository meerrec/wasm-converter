/**
 * Просмотр документа: наш собственный вьюер DOCX из `@meerrec/core` на странице харнесса.
 *
 * Ядро подключено относительным путём — харнесс лежит вне pnpm-workspace, и имя пакета в нём не
 * резолвится (та же причина, что у `packages/x2t` в src/worker.ts). Воркер вьюера собирает Vite
 * из src/core_worker.ts: ядро само создаёт его из переданного URL, и `workerUrl` — единственная
 * точка, где страница на это влияет.
 *
 * Документ один: вьюер создаётся при первом показе и живёт до перезагрузки страницы. Новый файл
 * открывается в нём же (`open` заменяет документ в том же воркере), поэтому холст не плодится —
 * холстов в разметке ровно столько, сколько вьюеров. Отказ открытия уничтожает вьюер: состояние
 * воркера после падения разбора неизвестно, и следующий документ получит свежий.
 */
import {
  createDocxViewer,
  type DocxViewerHandle,
  type PageInfo,
} from '../../../packages/core/dist/index.js';
import coreWorkerUrl from './core_worker.ts?worker&url';

/**
 * Ленивый DOCX-модуль (`@meerrec/wasm-docx`): воркер забирает обвязку динамическим
 * `import()` при первом открытии документа, а бинарник — `fetch`-ем. Бинарь передаётся отдельным
 * URL, потому что рядом с обвязкой его не оказывается: ассетом файл копируется как есть, а
 * wasm-bindgen искал бы его относительно модуля.
 */
const DOCX_MODULE_URL = new URL(
  '../../../packages/wasm-docx/pkg/doc_converter_docx_wasm.js',
  import.meta.url,
).href;
const DOCX_BINARY_URL = new URL(
  '../../../packages/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm',
  import.meta.url,
).href;

/** Как вьюер себя чувствует — для строки состояния на странице и для приёмки. */
export interface ViewerState {
  readonly status: 'closed' | 'opening' | 'ready' | 'error';
  /** Чем открыт документ: имя файла или пометка «результат конвертации». */
  readonly source: string | null;
  readonly pageCount: number;
  /** Размер первой страницы в пикселях раскладки — им проверяют, что документ разобран. */
  readonly pageSize: PageInfo | null;
  readonly pageIndex: number;
  readonly zoom: number;
  /** Каким путём идут кадры: `sab` при cross-origin isolation, иначе `bytes`. */
  readonly transport: string | null;
  /** Сколько кадров painter отрисовал — по нему видно, что холст не пустой. */
  readonly ticks: number;
  readonly paintMs: number | null;
  readonly error: string | null;
}

let handle: DocxViewerHandle | null = null;
let creating: Promise<DocxViewerHandle> | null = null;

let status: ViewerState['status'] = 'closed';
let source: string | null = null;
let openedPages: PageInfo[] = [];
let transport: string | null = null;
let ticks = 0;
let paintMs: number | null = null;
let lastError: string | null = null;

function describe(error: unknown): string {
  return error instanceof Error ? `${error.name}: ${error.message}` : String(error);
}

/** Состояние вьюера: страница показывает его строкой, приёмка — через `window.x2tHarness`. */
export function viewerState(): ViewerState {
  return {
    status,
    source,
    pageCount: handle?.pageCount ?? 0,
    pageSize: openedPages[0] ?? null,
    pageIndex: handle?.pageIndex ?? 0,
    zoom: handle?.zoom ?? 1,
    transport,
    ticks,
    paintMs,
    error: lastError,
  };
}

/** Вьюер на это `host`, созданный один раз; повторные вызовы ждут тот же самый. */
async function ensureViewer(host: HTMLElement): Promise<DocxViewerHandle> {
  if (handle !== null) return handle;
  creating ??= createDocxViewer(host, {
    workerUrl: coreWorkerUrl,
    docxWasm: { module: DOCX_MODULE_URL, binary: DOCX_BINARY_URL },
  }).then((created) => {
    handle = created;
    transport = created.transport;
    created.onTick((stats) => {
      ticks += 1;
      paintMs = stats.paintMs;
    });
    return created;
  });
  try {
    return await creating;
  } finally {
    creating = null;
  }
}

/**
 * Очередь открытий. Документ в вьюере один, и ответ воркера — один на запрос: два открытия
 * вперемешку (быстрый выбор файла и следом конвертация) разрешились бы по первому же ответу, и
 * на экране оказался бы не тот документ. Ошибка очередь не рвёт — следующий документ откроется.
 */
let queue: Promise<unknown> = Promise.resolve();

/**
 * Показать документ: `bytes` — байты `.docx` (свои или полученные конвертацией).
 *
 * Буфер передаётся воркеру переводом владения, поэтому вызывающий отдаёт копию: оригинал
 * остаётся ему для скачивания и повторных конвертаций.
 */
export function openDocument(host: HTMLElement, bytes: ArrayBuffer, from: string): Promise<PageInfo[]> {
  const opening = queue.then(
    () => openOne(host, bytes, from),
    () => openOne(host, bytes, from),
  );
  queue = opening.then(
    () => undefined,
    () => undefined,
  );
  return opening;
}

async function openOne(host: HTMLElement, bytes: ArrayBuffer, from: string): Promise<PageInfo[]> {
  status = 'opening';
  source = from;
  lastError = null;
  const viewer = await ensureViewer(host);
  try {
    const pages = await viewer.open(bytes);
    openedPages = pages;
    status = 'ready';
    return pages;
  } catch (error) {
    lastError = describe(error);
    destroyViewer();
    status = 'error';
    source = from;
    throw error;
  }
}

/**
 * Остановить воркер и убрать разметку вьюера. Зовётся на уходе со страницы: воркер держит
 * разобранный документ и wasm, а вместе со страницей ему жить незачем.
 */
export function destroyViewer(): void {
  handle?.destroy();
  handle = null;
  openedPages = [];
  transport = null;
}

/** Текущий кадр холста картинкой PNG в base64 — им проверяют, что нарисовано, а не пусто. */
export async function viewerPngBase64(): Promise<string | null> {
  if (handle === null) return null;
  return toBase64(await handle.exportPng());
}

/** base64 по кускам: `String.fromCharCode(...bytes)` на большом кадре упирается в лимит аргументов. */
function toBase64(bytes: Uint8Array): string {
  const chunk = 0x8000;
  let binary = '';
  for (let offset = 0; offset < bytes.byteLength; offset += chunk) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
  }
  return btoa(binary);
}
