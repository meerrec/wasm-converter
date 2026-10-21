/// <reference lib="webworker" />
// Отдельный воркер XLSX: обслуживает одну книгу и не трогает состояние DOCX.
// Общая обвязка (painter, SAB, транспорт кадра, битмапы, потоки) — в
// `worker_core`; здесь только xlsx-разбор, сборка кадра и экспорт.
import {
  set_cancel,
  stream_append,
  stream_free,
  stream_new,
  xlsx_image_bytes,
  xlsx_images,
} from '@meerrec/wasm';
import type { BuildStatsWire } from '../generated/wire.js';
import type { InMsg, RenderRequest } from '../protocol.js';
import { frameTarget, toViewport } from './frame_args.js';
import {
  buildXlsxInto,
  buildXlsxSab,
  exportPdf,
  exportPdfBook,
  hitTest,
  hyperlinkAt,
  images,
  openXlsx,
  openXlsxStream,
} from './wasm_typed.js';
import { post, postError, WorkerCore } from './worker_core.js';

/** Сборка кадра книги: рамка запроса разбирается в аргументы wasm-былл-функции. */
const buildInto = (out: Uint8Array, req: RenderRequest): BuildStatsWire | null => {
  const target = frameTarget('xlsx', req);
  if (target.kind !== 'xlsx') return null;
  return buildXlsxInto(out, target.sheet, target.viewport, target.options);
};

/** Сборка кадра книги прямо в слот ring'а. */
const buildSab = (
  sab: SharedArrayBuffer,
  slotCapacity: number,
  req: RenderRequest,
): BuildStatsWire | null => {
  const target = frameTarget('xlsx', req);
  if (target.kind !== 'xlsx') return null;
  return buildXlsxSab(sab, slotCapacity, target.sheet, target.viewport, target.options);
};

const core = new WorkerCore({
  format: 'xlsx',
  openBlockedReason: () => null,
  open: async (msg) => {
    const sheets = openXlsx(new Uint8Array(msg.bytes));
    return { type: 'opened', id: msg.id, sheets };
  },
  beginStream: (_msg, entry) => {
    // Основной wasm-модуль уже в памяти: буфер потока выделяется сразу.
    entry.streamId = stream_new();
  },
  appendStream: (streamId, bytes) => stream_append(streamId, bytes),
  freeStream: (streamId) => stream_free(streamId),
  finishStream: (streamId, id) => ({
    type: 'opened',
    id,
    sheets: openXlsxStream(streamId),
  }),
  images: () => ({
    tag: 'xlsx',
    list: () => images(xlsx_images()),
    bytes: (id) => xlsx_image_bytes(id),
  }),
  buildInto,
  buildSab,
  release: () => {
    // Книга XLSX живёт в основном модуле до следующего `xlsx_open`: отдельного
    // закрытия у неё нет.
  },
  cancel: () => {
    set_cancel(true);
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  if (await core.handle(msg)) return;
  switch (msg.type) {
    case 'hit-test': {
      try {
        const viewport = toViewport(msg.viewport);
        const cell = hitTest(msg.sheet, viewport, msg.x, msg.y);
        post({ type: 'hit', id: msg.id, cell });
      } catch {
        // Точка вне книги — не ошибка, а отсутствие ответа.
        post({ type: 'hit', id: msg.id, cell: null });
      }
      break;
    }
    case 'hyperlink-at': {
      try {
        const viewport = toViewport(msg.viewport);
        const link = hyperlinkAt(msg.sheet, viewport, msg.x, msg.y);
        post({ type: 'hyperlink', id: msg.id, link });
      } catch {
        // Книги нет или точка описана неверно — ответ пустой, а не ошибка.
        post({ type: 'hyperlink', id: msg.id, link: null });
      }
      break;
    }
    case 'export-pdf': {
      const urls = core.pdfModuleUrls;
      // Снимок: за время загрузки модуля книгу могли закрыть или сменить.
      const source = core.sourceBytes;
      if (!urls) {
        postError('pdf module is not configured: init has no pdf urls', {
          id: msg.id,
          code: 'export',
        });
        break;
      }
      if (!source) {
        postError('no workbook is open', { id: msg.id, code: 'export' });
        break;
      }
      try {
        // Первый клик платит за загрузку модуля; дальше он уже в памяти.
        const mod = await core.loadPdfWasm(urls);
        const bytes = msg.options.allSheets
          ? exportPdfBook(mod, new Uint8Array(source), msg.options)
          : exportPdf(mod, new Uint8Array(source), msg.sheet, msg.options);
        post({ type: 'pdf', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        // Ошибка экспорта — ответ, а не падение воркера: книга остаётся открытой.
        postError(String(e), { id: msg.id, code: 'export' });
      }
      break;
    }
  }
};
