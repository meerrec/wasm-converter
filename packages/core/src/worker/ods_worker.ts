/// <reference lib="webworker" />
// Отдельный воркер ODS: обслуживает одну книгу и не трогает состояние других
// форматов. Общая обвязка (painter, SAB, транспорт кадра, битмапы, потоки) — в
// `worker_core`; здесь ленивый ODS-модуль, сборка кадра и картинки.
import type { BuildStatsWire } from '../generated/wire.js';
import type {
  InMsg,
  OdsModuleUrls,
  OpenStreamMsg,
  RenderRequest,
} from '../protocol.js';
import { frameTarget } from './frame_args.js';
import {
  buildOdsInto,
  buildOdsSab,
  images,
  openOds,
  openOdsStream,
  type OdsWasm,
} from './wasm_typed.js';
import { post, postError, WorkerCore, type OpenStreamEntry } from './worker_core.js';

/** Откуда грузить ODS-модуль: заполняется на `init`, до него открытия нет. */
let odsUrls: OdsModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let odsWasm: OdsWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let odsWasmLoading: Promise<OdsWasm> | null = null;

/**
 * Загрузить ODS-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadOdsWasm(urls: OdsModuleUrls): Promise<OdsWasm> {
  if (odsWasm) return odsWasm;
  odsWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as OdsWasm;
    await mod.default(urls.binary);
    odsWasm = mod;
    return mod;
  })();
  try {
    return await odsWasmLoading;
  } catch (e) {
    odsWasmLoading = null;
    throw e;
  }
}

/** Начать приём потока: буфер выделяется после загрузки ленивого модуля. */
function beginStream(
  _msg: OpenStreamMsg,
  entry: OpenStreamEntry,
  alive: () => boolean,
): void {
  entry.moduleReady = loadOdsWasm(odsUrls!).then((mod) => {
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
  const target = frameTarget('ods', req);
  if (target.kind !== 'ods') return null;
  const mod = odsWasm;
  return mod ? buildOdsInto(mod, out, target.sheet, target.viewport, target.options) : null;
};

/** Сборка кадра книги прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('ods', req);
  if (target.kind !== 'ods') return null;
  const mod = odsWasm;
  return mod
    ? buildOdsSab(mod, sab, slotCapacity, target.sheet, target.viewport, target.options)
    : null;
};

const core = new WorkerCore({
  format: 'ods',
  configure: (msg) => {
    odsUrls = msg.ods ?? null;
  },
  openBlockedReason: () =>
    odsUrls ? null : 'ods module is not configured: init has no ods urls',
  open: async (msg) => {
    // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
    const mod = await loadOdsWasm(odsUrls!);
    const sheets = openOds(mod, new Uint8Array(msg.bytes));
    return { type: 'opened', id: msg.id, sheets };
  },
  beginStream,
  appendStream: (streamId, bytes) => {
    odsWasm!.stream_append(streamId, bytes);
  },
  freeStream: (streamId) => {
    odsWasm?.stream_free(streamId);
  },
  finishStream: (streamId, id) => ({
    type: 'opened',
    id,
    sheets: openOdsStream(odsWasm!, streamId),
  }),
  images: () => ({
    tag: 'ods',
    list: () => images(odsWasm!.ods_images()),
    bytes: (id) => odsWasm!.ods_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Книгу и реестр картинок держит ленивый модуль: отпускает их только
    // `ods_close`.
    odsWasm?.ods_close();
  },
  cancel: () => {
    odsWasm?.set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'export-pdf': {
      // PDF-экспорт ODS в этом срезе не реализован (отдельный pdf-wasm).
      postError('ods pdf export is not implemented', { id: msg.id, code: 'export' });
      break;
    }
  }
};
