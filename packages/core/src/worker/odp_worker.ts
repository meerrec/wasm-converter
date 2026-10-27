/// <reference lib="webworker" />
// Отдельный воркер ODP: обслуживает одну презентацию и не трогает состояние
// других форматов. Общая обвязка (painter, SAB, транспорт кадра, битмапы,
// потоки) — в `worker_core`; здесь ленивый ODP-модуль, сборка кадра и текстовые
// выгрузки.
import type { BuildStatsWire } from '../generated/wire.js';
import type {
  InMsg,
  OdpModuleUrls,
  OpenStreamMsg,
  RenderRequest,
} from '../protocol.js';
import { frameTarget } from './frame_args.js';
import {
  buildOdpInto,
  buildOdpSab,
  exportOdpText,
  images,
  openOdp,
  openOdpStream,
  type OdpWasm,
} from './wasm_typed.js';
import { post, postError, WorkerCore, type OpenStreamEntry } from './worker_core.js';

/** Откуда грузить ODP-модуль: заполняется на `init`, до него открытия нет. */
let odpUrls: OdpModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let odpWasm: OdpWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let odpWasmLoading: Promise<OdpWasm> | null = null;

/**
 * Загрузить ODP-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadOdpWasm(urls: OdpModuleUrls): Promise<OdpWasm> {
  if (odpWasm) return odpWasm;
  odpWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as OdpWasm;
    await mod.default(urls.binary);
    odpWasm = mod;
    return mod;
  })();
  try {
    return await odpWasmLoading;
  } catch (e) {
    odpWasmLoading = null;
    throw e;
  }
}

/** Начать приём потока: буфер выделяется после загрузки ленивого модуля. */
function beginStream(
  _msg: OpenStreamMsg,
  entry: OpenStreamEntry,
  alive: () => boolean,
): void {
  entry.moduleReady = loadOdpWasm(odpUrls!).then((mod) => {
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
  const target = frameTarget('odp', req);
  if (target.kind !== 'odp') return null;
  const mod = odpWasm;
  return mod ? buildOdpInto(mod, out, target.slide, target.viewport) : null;
};

/** Сборка кадра слайда прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('odp', req);
  if (target.kind !== 'odp') return null;
  const mod = odpWasm;
  return mod ? buildOdpSab(mod, sab, slotCapacity, target.slide, target.viewport) : null;
};

const core = new WorkerCore({
  format: 'odp',
  configure: (msg) => {
    odpUrls = msg.odp ?? null;
  },
  openBlockedReason: () =>
    odpUrls ? null : 'odp module is not configured: init has no odp urls',
  open: async (msg) => {
    // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
    const mod = await loadOdpWasm(odpUrls!);
    const slides = openOdp(mod, new Uint8Array(msg.bytes));
    return { type: 'pptx-opened', id: msg.id, slides };
  },
  beginStream,
  appendStream: (streamId, bytes) => {
    odpWasm!.stream_append(streamId, bytes);
  },
  freeStream: (streamId) => {
    odpWasm?.stream_free(streamId);
  },
  finishStream: (streamId, id) => ({
    type: 'pptx-opened',
    id,
    slides: openOdpStream(odpWasm!, streamId),
  }),
  images: () => ({
    tag: 'odp',
    list: () => images(odpWasm!.odp_images()),
    bytes: (id) => odpWasm!.odp_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Презентацию и реестр картинок держит ленивый модуль: отпускает их только
    // `odp_close`.
    odpWasm?.odp_close();
  },
  cancel: () => {
    odpWasm?.set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'export-pptx-text': {
      const urls = odpUrls;
      const source = core.sourceBytes;
      if (!urls) {
        postError('odp module is not configured: init has no odp urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no odp presentation is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Модуль загружен ещё открытием презентации; здесь он уже в памяти.
        const mod = await loadOdpWasm(urls);
        const text = exportOdpText(mod, new Uint8Array(source), msg.format);
        post({ type: 'pptx-text', id: msg.id, text });
      } catch (e) {
        // Разбор мог упасть и здесь: презентация остаётся открытой, клиент получает ошибку.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-pptx-pdf': {
      // PDF-экспорт ODP в этом срезе не реализован.
      postError('odp pdf export is not implemented', { id: msg.id, code: 'export' });
      break;
    }
  }
};
