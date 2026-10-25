/// <reference lib="webworker" />
// Отдельный воркер DOCX: обслуживает один документ и не трогает состояние
// XLSX. Общая обвязка (painter, SAB, транспорт кадра, битмапы, потоки) — в
// `worker_core`; здесь ленивый DOCX-модуль, сборка кадра и текстовый/PDF-экспорт.
import type { BuildStatsWire } from '../generated/wire.js';
import type { DocxModuleUrls, InMsg, OpenStreamMsg, RenderRequest } from '../protocol.js';
import { frameTarget } from './frame_args.js';
import {
  buildDocxInto,
  buildDocxSab,
  docxEdits,
  docxFullTextLayer,
  docxPages,
  docxSetRunText,
  docxTextLayer,
  exportDocxPdf,
  images,
  openDocx,
  openDocxStream,
  type DocxWasm,
} from './wasm_typed.js';
import { post, postError, WorkerCore, type OpenStreamEntry } from './worker_core.js';

/** Откуда грузить DOCX-модуль: заполняется на `init`, до него открытия нет. */
let docxUrls: DocxModuleUrls | null = null;
/** Загруженный модуль: `import()` и `init()` платятся один раз за воркер. */
let docxWasm: DocxWasm | null = null;
/** Незавершённая загрузка — чтобы два открытия подряд не тянули модуль дважды. */
let docxWasmLoading: Promise<DocxWasm> | null = null;

/**
 * Загрузить DOCX-модуль по требованию. Неудача не кэшируется: следующее
 * открытие попробует снова, иначе один сетевой сбой навсегда ломал бы просмотр.
 */
async function loadDocxWasm(urls: DocxModuleUrls): Promise<DocxWasm> {
  if (docxWasm) return docxWasm;
  docxWasmLoading ??= (async () => {
    // URL динамический: модуль лежит отдельным файлом, и статический импорт
    // вернул бы его в основной бандл — ровно то, от чего мы ушли.
    const mod = (await import(/* @vite-ignore */ urls.module)) as DocxWasm;
    await mod.default(urls.binary);
    docxWasm = mod;
    return mod;
  })();
  try {
    return await docxWasmLoading;
  } catch (e) {
    docxWasmLoading = null;
    throw e;
  }
}

/** Начать приём потока: буфер выделяется после загрузки ленивого модуля. */
function beginStream(
  _msg: OpenStreamMsg,
  entry: OpenStreamEntry,
  alive: () => boolean,
): void {
  entry.moduleReady = loadDocxWasm(docxUrls!).then((mod) => {
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
  const target = frameTarget('docx', req);
  if (target.kind !== 'docx') return null;
  const mod = docxWasm;
  return mod ? buildDocxInto(mod, out, target.viewport, target.gap) : null;
};

/** Сборка кадра стопки прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('docx', req);
  if (target.kind !== 'docx') return null;
  const mod = docxWasm;
  return mod ? buildDocxSab(mod, sab, slotCapacity, target.viewport, target.gap) : null;
};

const core = new WorkerCore({
  format: 'docx',
  configure: (msg) => {
    docxUrls = msg.docx ?? null;
  },
  openBlockedReason: () =>
    docxUrls ? null : 'docx module is not configured: init has no docx urls',
  open: async (msg) => {
    // Первое открытие платит за загрузку модуля; дальше он уже в памяти.
    const mod = await loadDocxWasm(docxUrls!);
    const pages = openDocx(mod, new Uint8Array(msg.bytes));
    return { type: 'docx-opened', id: msg.id, pages };
  },
  beginStream,
  appendStream: (streamId, bytes) => {
    docxWasm!.stream_append(streamId, bytes);
  },
  freeStream: (streamId) => {
    docxWasm?.stream_free(streamId);
  },
  finishStream: (streamId, id) => ({
    type: 'docx-opened',
    id,
    pages: openDocxStream(docxWasm!, streamId),
  }),
  images: () => ({
    tag: 'docx',
    list: () => images(docxWasm!.docx_images()),
    bytes: (id) => docxWasm!.docx_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Раскладку и реестр шрифтов держит ленивый модуль: отпускает их только
    // `docx_close`.
    docxWasm?.docx_close();
  },
  cancel: () => {
    docxWasm?.set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'edit-docx': {
      try {
        const mod = docxWasm;
        if (!mod) throw new Error('no docx document is open');
        // Правка ложится в документ и перераскладывает его; ответ несёт новые
        // размеры страниц — вьюер перестроит стопку и перезапросит слой.
        docxSetRunText(mod, msg.paragraph, msg.run, msg.text);
        post({ type: 'docx-edited', id: msg.id, pages: docxPages(mod) });
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'internal' });
      }
      break;
    }
    case 'export-docx-pdf': {
      const urls = core.pdfModuleUrls;
      // Снимок: за время загрузки модуля документ могли закрыть или сменить.
      const source = core.sourceBytes;
      if (!urls) {
        postError('pdf module is not configured: init has no pdf urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no docx document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await core.loadPdfWasm(urls);
        // Правки интерфейса: экспортёр разбирает байты заново и получает их
        // снимком из документа.
        const edits = docxWasm === null ? [] : docxEdits(docxWasm);
        const bytes = exportDocxPdf(mod, new Uint8Array(source), msg.options, edits);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: документ остаётся открытым.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'text-layer': {
      const mod = docxWasm;
      if (!mod) {
        postError('no docx document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Раскладка уже посчитана при открытии: здесь только выбор страницы.
        post({ type: 'text-layer', id: msg.id, layer: docxTextLayer(mod, msg.page) });
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'text-layer-all': {
      const mod = docxWasm;
      if (!mod) {
        postError('no docx document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        post({ type: 'text-layer', id: msg.id, layer: docxFullTextLayer(mod) });
      } catch (e) {
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
    case 'export-docx-text': {
      const urls = docxUrls;
      const source = core.sourceBytes;
      if (!urls) {
        postError('docx module is not configured: init has no docx urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no docx document is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Модуль загружен ещё открытием документа; здесь он уже в памяти.
        const mod = await loadDocxWasm(urls);
        const bytes = new Uint8Array(source);
        const text =
          msg.format === 'markdown'
            ? mod.docx_export_markdown(bytes)
            : msg.format === 'html'
              ? mod.docx_export_html(bytes)
              : mod.docx_export_text(bytes);
        post({ type: 'docx-text', id: msg.id, text });
      } catch (e) {
        // Разбор мог упасть и здесь: документ остаётся открытым, клиент получает ошибку.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
  }
};
