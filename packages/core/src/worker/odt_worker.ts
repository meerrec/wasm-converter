/// <reference lib="webworker" />
// Отдельный воркер ODT: обслуживает один документ и не трогает состояние других
// форматов. Общая обвязка (painter, SAB, транспорт кадра, битмапы, потоки) — в
// `worker_core`; здесь ленивый ODT-модуль, сборка кадра, текстовый слой и
// текстовые выгрузки.
import type { BuildStatsWire } from '../generated/wire.js';
import type {
  InMsg,
  OdtModuleUrls,
  OpenStreamMsg,
  RenderRequest,
} from '../protocol.js';
import { frameTarget } from './frame_args.js';
import {
  buildOdtInto,
  buildOdtSab,
  exportOdtText,
  images,
  odtFullTextLayer,
  odtTextLayer,
  openOdt,
  openOdtStream,
  type OdtWasm,
} from './wasm_typed.js';
import { post, postError, WorkerCore, type OpenStreamEntry } from './worker_core.js';

/** Откуда грузить ODT-модуль: заполняется на `init`, до него открытия нет. */
let odtUrls: OdtModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let odtWasm: OdtWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let odtWasmLoading: Promise<OdtWasm> | null = null;

/**
 * Загрузить ODT-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadOdtWasm(urls: OdtModuleUrls): Promise<OdtWasm> {
  if (odtWasm) return odtWasm;
  odtWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as OdtWasm;
    await mod.default(urls.binary);
    odtWasm = mod;
    return mod;
  })();
  try {
    return await odtWasmLoading;
  } catch (e) {
    odtWasmLoading = null;
    throw e;
  }
}

/** Начать приём потока: буфер выделяется после загрузки ленивого модуля. */
function beginStream(
  _msg: OpenStreamMsg,
  entry: OpenStreamEntry,
  alive: () => boolean,
): void {
  entry.moduleReady = loadOdtWasm(odtUrls!).then((mod) => {
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

/** Сборка кадра стопки страниц: рамка запроса разбирается в wasm-аргументы. */
const buildInto = (out: Uint8Array, req: RenderRequest): BuildStatsWire | null => {
  const target = frameTarget('odt', req);
  if (target.kind !== 'odt') return null;
  const mod = odtWasm;
  return mod ? buildOdtInto(mod, out, target.viewport, target.gap) : null;
};

/** Сборка кадра стопки прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('odt', req);
  if (target.kind !== 'odt') return null;
  const mod = odtWasm;
  return mod ? buildOdtSab(mod, sab, slotCapacity, target.viewport, target.gap) : null;
};

const core = new WorkerCore({
  format: 'odt',
  configure: (msg) => {
    odtUrls = msg.odt ?? null;
  },
  openBlockedReason: () =>
    odtUrls ? null : 'odt module is not configured: init has no odt urls',
  open: async (msg) => {
    // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
    const mod = await loadOdtWasm(odtUrls!);
    const pages = openOdt(mod, new Uint8Array(msg.bytes));
    return { type: 'docx-opened', id: msg.id, pages };
  },
  beginStream,
  appendStream: (streamId, bytes) => {
    odtWasm!.stream_append(streamId, bytes);
  },
  freeStream: (streamId) => {
    odtWasm?.stream_free(streamId);
  },
  finishStream: (streamId, id) => ({
    type: 'docx-opened',
    id,
    pages: openOdtStream(odtWasm!, streamId),
  }),
  images: () => ({
    tag: 'odt',
    list: () => images(odtWasm!.odt_images()),
    bytes: (id) => odtWasm!.odt_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Раскладку и реестр картинок держит ленивый модуль: отпускает их только
    // `odt_close`.
    odtWasm?.odt_close();
  },
  cancel: () => {
    odtWasm?.set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'text-layer': {
      const mod = odtWasm;
      if (!mod) {
        postError('no odt document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Раскладка уже посчитана при открытии: здесь только выбор страницы.
        post({ type: 'text-layer', id: msg.id, layer: odtTextLayer(mod, msg.page) });
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'text-layer-all': {
      const mod = odtWasm;
      if (!mod) {
        postError('no odt document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        post({ type: 'text-layer', id: msg.id, layer: odtFullTextLayer(mod) });
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-docx-text': {
      const urls = odtUrls;
      const source = core.sourceBytes;
      if (!urls) {
        postError('odt module is not configured: init has no odt urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no odt document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Модуль загружен ещё открытием документа; здесь он уже в памяти.
        const mod = await loadOdtWasm(urls);
        const text = exportOdtText(mod, new Uint8Array(source), msg.format);
        post({ type: 'docx-text', id: msg.id, text });
      } catch (e) {
        // Разбор мог упасть и здесь: документ остаётся открытым, клиент получает ошибку.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-docx-pdf': {
      // PDF-экспорт ODT в этом срезе не реализован.
      postError('odt pdf export is not implemented', { id: msg.id, code: 'export' });
      break;
    }
  }
};
