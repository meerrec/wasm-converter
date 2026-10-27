/// <reference lib="webworker" />
// Отдельный воркер XLSB: обслуживает одну книгу и не трогает состояние других
// форматов. Общая обвязка (painter, SAB, транспорт кадра, битмапы, потоки) — в
// `worker_core`; здесь ленивый XLSB-модуль, сборка кадра и картинки.
import type { BuildStatsWire } from '../generated/wire.js';
import type {
  InMsg,
  OpenStreamMsg,
  RenderRequest,
  XlsbModuleUrls,
} from '../protocol.js';
import { frameTarget } from './frame_args.js';
import {
  buildXlsbInto,
  buildXlsbSab,
  images,
  openXlsb,
  openXlsbStream,
  type XlsbWasm,
} from './wasm_typed.js';
import { post, postError, WorkerCore, type OpenStreamEntry } from './worker_core.js';

/** Откуда грузить XLSB-модуль: заполняется на `init`, до него открытия нет. */
let xlsbUrls: XlsbModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let xlsbWasm: XlsbWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let xlsbWasmLoading: Promise<XlsbWasm> | null = null;

/**
 * Загрузить XLSB-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadXlsbWasm(urls: XlsbModuleUrls): Promise<XlsbWasm> {
  if (xlsbWasm) return xlsbWasm;
  xlsbWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as XlsbWasm;
    await mod.default(urls.binary);
    xlsbWasm = mod;
    return mod;
  })();
  try {
    return await xlsbWasmLoading;
  } catch (e) {
    xlsbWasmLoading = null;
    throw e;
  }
}

/** Начать приём потока: буфер выделяется после загрузки ленивого модуля. */
function beginStream(
  _msg: OpenStreamMsg,
  entry: OpenStreamEntry,
  alive: () => boolean,
): void {
  entry.moduleReady = loadXlsbWasm(xlsbUrls!).then((mod) => {
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

/** Сборка кадра книги: рамка запроса разбирается в аргументы wasm-функции. */
const buildInto = (out: Uint8Array, req: RenderRequest): BuildStatsWire | null => {
  const target = frameTarget('xlsb', req);
  if (target.kind !== 'xlsb') return null;
  const mod = xlsbWasm;
  return mod ? buildXlsbInto(mod, out, target.sheet, target.viewport, target.options) : null;
};

/** Сборка кадра книги прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('xlsb', req);
  if (target.kind !== 'xlsb') return null;
  const mod = xlsbWasm;
  return mod
    ? buildXlsbSab(mod, sab, slotCapacity, target.sheet, target.viewport, target.options)
    : null;
};

const core = new WorkerCore({
  format: 'xlsb',
  configure: (msg) => {
    xlsbUrls = msg.xlsb ?? null;
  },
  openBlockedReason: () =>
    xlsbUrls ? null : 'xlsb module is not configured: init has no xlsb urls',
  open: async (msg) => {
    // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
    const mod = await loadXlsbWasm(xlsbUrls!);
    const sheets = openXlsb(mod, new Uint8Array(msg.bytes));
    return { type: 'opened', id: msg.id, sheets };
  },
  beginStream,
  appendStream: (streamId, bytes) => {
    xlsbWasm!.stream_append(streamId, bytes);
  },
  freeStream: (streamId) => {
    xlsbWasm?.stream_free(streamId);
  },
  finishStream: (streamId, id) => ({
    type: 'opened',
    id,
    sheets: openXlsbStream(xlsbWasm!, streamId),
  }),
  images: () => ({
    tag: 'xlsb',
    list: () => images(xlsbWasm!.xlsb_images()),
    bytes: (id) => xlsbWasm!.xlsb_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Книгу и реестр картинок держит ленивый модуль: отпускает их только
    // `xlsb_close`.
    xlsbWasm?.xlsb_close();
  },
  cancel: () => {
    xlsbWasm?.set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'export-pdf': {
      // PDF-экспорт XLSB в этом срезе не реализован (отдельный pdf-wasm).
      postError('xlsb pdf export is not implemented', { id: msg.id, code: 'export' });
      break;
    }
  }
};
