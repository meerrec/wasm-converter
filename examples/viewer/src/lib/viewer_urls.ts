// URL-ы воркеров и ленивых wasm-модулей. Здесь они собраны в одном месте,
// потому что нужны страницам, а собираться должны один раз: чанки воркеров и
// wasm-модулей общие для DOCX и XLSX.

// Воркеры подключаются суффиксом `?worker&url`: только так Vite собирает их
// отдельными чанками. Голый `new URL('./docx_worker.ts', import.meta.url)` он
// считает обычным ассетом и инлайнит исходник в data-URL, где импорт
// `@doc-converter/core/worker` уже не разрешается, — воркер молча не стартует.
import convertWorkerFileUrl from '../workers/convert_worker.ts?worker&url';
import docxWorkerFileUrl from '../workers/docx_worker.ts?worker&url';
import xlsxWorkerFileUrl from '../workers/xlsx_worker.ts?worker&url';
import { getWasm, putWasm } from './doc_cache.js';

/** URL собранного воркера DOCX — уходит в `createDocxViewer({ workerUrl })`. */
export const docxWorkerUrl = docxWorkerFileUrl;

/** URL собранного воркера XLSX — уходит в `createXlsxViewer({ workerUrl })`. */
export const xlsxWorkerUrl = xlsxWorkerFileUrl;

/** URL собранного воркера конвертера — страница создаёт его `new Worker(...)`. */
export const convertWorkerUrl = convertWorkerFileUrl;

// Артефакты x2t (обвязка и wasm) лежат статикой: dev-сервер отдаёт `public/x2t/`
// как есть, сборка кладёт их туда скриптом `scripts/copy-x2t.mjs`. На Pages путь
// сдвигается базой Vite (`/<repo>/`), поэтому URL собирается от `BASE_URL`, —
// локально это корень, и оба режима остаются рабочими.
const X2T_DIR = `${import.meta.env.BASE_URL}x2t/`;

/** URL обвязки x2t — её воркер вставляет `<script>` при первом запросе. */
export const x2tScriptUrl = `${X2T_DIR}x2t.js`;

/** URL wasm-модуля x2t: 38 МБ, лежит рядом с обвязкой и грузится ею же. */
export const x2tWasmUrl = `${X2T_DIR}x2t.wasm`;

// Ленивые wasm-модули отдаются URL-ами, а не импортом: `new URL(...)` Vite
// разворачивает в ассет, тогда как импорт модуля втянул бы его в основной
// бандл, и страница платила бы за wasm ещё до открытия документа. JS при этом
// копируется как есть, поэтому сборщик не кладёт рядом его wasm-бинарь: второй
// URL передаётся явно.

/** URL DOCX-модуля: воркер грузит его при открытии документа. */
export const docxWasmModuleUrl = new URL(
  '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm.js',
  import.meta.url,
).href;

/** URL wasm-бинарника DOCX-модуля: рядом с JS он не оказывается. */
export const docxWasmBinaryUrl = new URL(
  '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm',
  import.meta.url,
).href;

/** URL PDF-модуля: воркер грузит его при первом экспорте в PDF. */
export const pdfWasmModuleUrl = new URL(
  '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
  import.meta.url,
).href;

/** URL wasm-бинарника PDF-модуля: рядом с JS он не оказывается. */
export const pdfWasmBinaryUrl = new URL(
  '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm',
  import.meta.url,
).href;

/** Пара URL-ов одного wasm-модуля: JS-обвязка и её бинарник. */
export interface WasmUrls {
  /** URL JS-обвязки: его воркер импортирует динамически. */
  module: string;
  /** URL бинарника: его воркер забирает `fetch`-ем при инициализации. */
  binary: string;
}

/** MIME JS-модуля: по нему браузер разбирает blob-URL как модуль, а не как текст. */
const JS_MIME = 'text/javascript';

/** MIME wasm-бинарника: без него стриминговая инициализация не сработает. */
const WASM_MIME = 'application/wasm';

/** Blob-URL-ы, выданные воркеру и ещё не отозванные: см. `releaseAllWasmUrls`. */
const liveBlobUrls = new Set<string>();

/** Blob-URL-ы, выданные последним резолвом: их и отзывают, когда модуль загружен. */
let docxResolved: WasmUrls | null = null;
let pdfResolved: WasmUrls | null = null;

/**
 * URL файла модуля для воркера: из кэша — blob-URL-ом, без кэша — сетевой адрес.
 *
 * Сеть здесь не трогается намеренно. Модуль грузит воркер, и только тогда, когда
 * он вправду понадобился: DOCX-модуль — при открытии документа, PDF-модуль —
 * при первом экспорте. Скачай резолвер байты сам, страница платила бы за оба
 * модуля сразу при заходе — ровно то, от чего ушли ленивые URL-ы.
 *
 * Байты из кэша отдаются blob-URL-ом: воркеру он неотличим от сетевого адреса —
 * JS он импортирует динамически, бинарник забирает `fetch`-ем. Отказ хранилища
 * возвращает исходный URL: кэш — ускорение, а не условие работы.
 */
async function urlFromCache(url: string, type: string): Promise<string> {
  try {
    const bytes = await getWasm(url);
    if (bytes === null) return url;
    const blobUrl = URL.createObjectURL(new Blob([bytes], { type }));
    liveBlobUrls.add(blobUrl);
    return blobUrl;
  } catch {
    return url;
  }
}

/**
 * URL-ы DOCX-модуля для воркера: прогретый кэш — blob-URL-ы, холодный — сеть.
 *
 * Зовётся при создании вьюера: чтение хранилища дёшево, а сетевых запросов за
 * собой не влечёт — модуль остаётся ленивым.
 */
export async function resolveDocxWasm(): Promise<WasmUrls> {
  const [module, binary] = await Promise.all([
    urlFromCache(docxWasmModuleUrl, JS_MIME),
    urlFromCache(docxWasmBinaryUrl, WASM_MIME),
  ]);
  docxResolved = { module, binary };
  return docxResolved;
}

/**
 * URL-ы PDF-модуля для воркера: прогретый кэш — blob-URL-ы, холодный — сеть.
 *
 * Зовётся при создании вьюера вместе с `resolveDocxWasm`: адрес модуля уходит
 * воркеру сразу, а грузит его воркер сам — при первом экспорте.
 */
export async function resolvePdfWasm(): Promise<WasmUrls> {
  const [module, binary] = await Promise.all([
    urlFromCache(pdfWasmModuleUrl, JS_MIME),
    urlFromCache(pdfWasmBinaryUrl, WASM_MIME),
  ]);
  pdfResolved = { module, binary };
  return pdfResolved;
}

/**
 * Отозвать blob-URL-ы пары. Сетевые адреса в наборе не значатся, поэтому
 * холодный кэш отзыв просто пропускает.
 */
function revokeResolved(urls: WasmUrls | null): void {
  if (urls === null) return;
  for (const url of [urls.module, urls.binary]) {
    if (liveBlobUrls.delete(url)) URL.revokeObjectURL(url);
  }
}

/**
 * Отозвать blob-URL-ы DOCX-модуля. Воркер забирает модуль один раз — при первом
 * успешном открытии документа, — и до этого момента адрес обязан жить: `import`
 * отозванного blob-URL падает. Раньше отзывать нельзя, позже незачем.
 */
export function releaseDocxWasmUrls(): void {
  revokeResolved(docxResolved);
  docxResolved = null;
}

/** Отозвать blob-URL-ы PDF-модуля: тот же расчёт, но загрузка — первый экспорт. */
export function releasePdfWasmUrls(): void {
  revokeResolved(pdfResolved);
  pdfResolved = null;
}

/**
 * Отозвать всё, что выдали резолверы. Нужно на уходе со страницы: воркер вместе
 * с ней уничтожается, и blob-URL-ы пережили бы его, оставшись в памяти вкладки
 * до её закрытия.
 */
export function releaseAllWasmUrls(): void {
  for (const url of liveBlobUrls) URL.revokeObjectURL(url);
  liveBlobUrls.clear();
  docxResolved = null;
  pdfResolved = null;
}

/**
 * Забрать байты одного файла модуля в кэш — если их там ещё нет.
 *
 * Зовётся после того, как модуль уже загружен: к этому моменту те же URL-ы
 * заведомо прошли через HTTP-кэш браузера, и повторный `fetch` почти бесплатен,
 * а в хранилище ложится копия для следующего визита. Уже лежащие байты не
 * перезаписываются: загрузка модуля их не меняет, а лишняя запись — это лишний
 * расход квоты и времени.
 */
async function warmOne(url: string): Promise<void> {
  try {
    if ((await getWasm(url)) !== null) return;
    const response = await fetch(url);
    if (!response.ok) return;
    await putWasm(url, await response.arrayBuffer());
  } catch {
    // Сеть или хранилище отказали — кэш просто останется холодным.
  }
}

/**
 * Положить DOCX-модуль в кэш: вызывать после первого успешного открытия
 * документа, когда воркер уже загрузил модуль.
 */
export async function warmDocxWasmCache(): Promise<void> {
  await Promise.all([warmOne(docxWasmModuleUrl), warmOne(docxWasmBinaryUrl)]);
}

/**
 * Положить PDF-модуль в кэш: вызывать после первого успешного экспорта в PDF.
 */
export async function warmPdfWasmCache(): Promise<void> {
  await Promise.all([warmOne(pdfWasmModuleUrl), warmOne(pdfWasmBinaryUrl)]);
}
