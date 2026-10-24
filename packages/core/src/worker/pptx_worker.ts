/// <reference lib="webworker" />
// Отдельный воркер PPTX: обслуживает одну презентацию и не трогает состояние
// XLSX/DOCX. Общая обвязка (painter, SAB, транспорт кадра, битмапы, потоки) — в
// `worker_core`; здесь ленивый PPTX-модуль, сборка кадра и текстовый/PDF-экспорт.
// Просмотрщик и экспортёр разнесены по модулям, как у DOCX: экспорт в PDF
// держит отдельный ленивый модуль `@meerrec/wasm-pptx-pdf`.
import type { BuildStatsWire } from '../generated/wire.js';
import type {
  InMsg,
  OpenStreamMsg,
  PdfModuleUrls,
  PptxModuleUrls,
  RenderRequest,
} from '../protocol.js';
import { frameTarget } from './frame_args.js';
import {
  buildPptxInto,
  buildPptxSab,
  exportPptxPdf,
  exportPptxText,
  images,
  openPptx,
  openPptxStream,
  type PptxPdfWasm,
  type PptxWasm,
} from './wasm_typed.js';
import { post, postError, WorkerCore, type OpenStreamEntry } from './worker_core.js';

/** Откуда грузить PPTX-модуль: заполняется на `init`, до него открытия нет. */
let pptxUrls: PptxModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let pptxWasm: PptxWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let pptxWasmLoading: Promise<PptxWasm> | null = null;

/** Откуда грузить PPTX-PDF-модуль: заполняется на `init`, до него экспорта нет. */
let pptxPdfUrls: PdfModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let pptxPdfWasm: PptxPdfWasm | null = null;
/** Незавершённая загрузка — чтобы два клика подряд не тянули модуль дважды. */
let pptxPdfWasmLoading: Promise<PptxPdfWasm> | null = null;

/**
 * Загрузить PPTX-PDF-модуль по требованию. Неудача не кэшируется: следующий
 * клик попробует снова, иначе один сетевой сбой навсегда ломал бы экспорт.
 */
async function loadPptxPdfWasm(urls: PdfModuleUrls): Promise<PptxPdfWasm> {
  if (pptxPdfWasm) return pptxPdfWasm;
  pptxPdfWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as PptxPdfWasm;
    await mod.default(urls.binary);
    pptxPdfWasm = mod;
    return mod;
  })();
  try {
    return await pptxPdfWasmLoading;
  } catch (e) {
    pptxPdfWasmLoading = null;
    throw e;
  }
}

/**
 * Загрузить PPTX-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadPptxWasm(urls: PptxModuleUrls): Promise<PptxWasm> {
  if (pptxWasm) return pptxWasm;
  pptxWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as PptxWasm;
    await mod.default(urls.binary);
    pptxWasm = mod;
    return mod;
  })();
  try {
    return await pptxWasmLoading;
  } catch (e) {
    pptxWasmLoading = null;
    throw e;
  }
}

/** Начать приём потока: буфер выделяется после загрузки ленивого модуля. */
function beginStream(
  _msg: OpenStreamMsg,
  entry: OpenStreamEntry,
  alive: () => boolean,
): void {
  entry.moduleReady = loadPptxWasm(pptxUrls!).then((mod) => {
    // id буфера выдаётся самой записи, а не через поиск по карте:
    // end-stream к этому моменту запись уже мог удалить.
    entry.streamId = mod.stream_new();
    // Поток могли закрыть, пока грузился модуль: разбирать буфер некому.
    if (!alive()) {
      mod.stream_free(entry.streamId);
      entry.streamId = 0;
    }
    return mod;
  });
}

/** Сборка кадра слайда: рамка запроса разбирается в wasm-аргументы. */
const buildInto = (out: Uint8Array, req: RenderRequest): BuildStatsWire | null => {
  const target = frameTarget('pptx', req);
  if (target.kind !== 'pptx') return null;
  const mod = pptxWasm;
  return mod ? buildPptxInto(mod, out, target.slide, target.viewport) : null;
};

/** Сборка кадра слайда прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('pptx', req);
  if (target.kind !== 'pptx') return null;
  const mod = pptxWasm;
  return mod ? buildPptxSab(mod, sab, slotCapacity, target.slide, target.viewport) : null;
};

const core = new WorkerCore({
  format: 'pptx',
  configure: (msg) => {
    pptxUrls = msg.pptx ?? null;
    pptxPdfUrls = msg.pptxPdf ?? null;
  },
  openBlockedReason: () =>
    pptxUrls ? null : 'pptx module is not configured: init has no pptx urls',
  open: async (msg) => {
    // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
    const mod = await loadPptxWasm(pptxUrls!);
    const slides = openPptx(mod, new Uint8Array(msg.bytes));
    return { type: 'pptx-opened', id: msg.id, slides };
  },
  beginStream,
  appendStream: (streamId, bytes) => {
    pptxWasm!.stream_append(streamId, bytes);
  },
  freeStream: (streamId) => {
    pptxWasm?.stream_free(streamId);
  },
  finishStream: (streamId, id) => ({
    type: 'pptx-opened',
    id,
    slides: openPptxStream(pptxWasm!, streamId),
  }),
  images: () => ({
    tag: 'pptx',
    list: () => images(pptxWasm!.pptx_images()),
    bytes: (id) => pptxWasm!.pptx_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Раскладку и реестр шрифтов держит ленивый модуль: отпускает их только
    // `pptx_close`.
    pptxWasm?.pptx_close();
  },
  cancel: () => {
    pptxWasm?.set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'export-pptx-pdf': {
      const urls = pptxPdfUrls;
      // Снимок: за время загрузки модуля презентацию могли закрыть или сменить.
      const source = core.sourceBytes;
      if (!urls) {
        postError('pdf module is not configured: init has no pdf urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no pptx presentation is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await loadPptxPdfWasm(urls);
        const bytes = exportPptxPdf(mod, new Uint8Array(source), msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: презентация остаётся открытой.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-pptx-text': {
      const urls = pptxUrls;
      const source = core.sourceBytes;
      if (!urls) {
        postError('pptx module is not configured: init has no pptx urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no pptx presentation is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Модуль загружен ещё открытием презентации; здесь он уже в памяти.
        const mod = await loadPptxWasm(urls);
        const text = exportPptxText(mod, new Uint8Array(source), msg.format);
        post({ type: 'pptx-text', id: msg.id, text });
      } catch (e) {
        // Разбор мог упасть и здесь: презентация остаётся открытой, клиент получает ошибку.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
  }
};
