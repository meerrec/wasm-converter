/// <reference lib="webworker" />
import init, {
  open_docx,
  open_xlsx,
  build_display_list,
  paint_display_list_to_offscreen,
  resize_canvas,
  hit_test,
  export_docx_to_pdf,
  export_xlsx_workbook_to_pdf,
} from '@doc-converter/wasm';

import type { WorkerRequest, WorkerResponse, FrameTimings } from '../protocol.js';

const ctxSelf = self as unknown as DedicatedWorkerGlobalScope;

let canvas: OffscreenCanvas | null = null;
let ctx: OffscreenCanvasRenderingContext2D | null = null;
let format: 'docx' | 'xlsx' | null = null;
let doc: unknown = null;
let lastRender: Extract<WorkerRequest, { type: 'render' }>['payload'] | null = null;
let rafScheduled = false;

ctxSelf.onmessage = async (ev: MessageEvent<WorkerRequest>) => {
  const msg = ev.data;
  try {
    switch (msg.type) {
      case 'init': {
        canvas = msg.payload.canvas;
        const c = canvas.getContext('2d', { alpha: false, desynchronized: true });
        if (!c) throw new Error('failed to acquire 2d context from OffscreenCanvas');
        ctx = c;
        await init();
        postOk(msg.id);
        break;
      }

      case 'resize': {
        if (!canvas) throw new Error('worker not initialized');
        const { width, height, dpr } = msg.payload;
        canvas.width  = Math.max(1, Math.round(width  * dpr));
        canvas.height = Math.max(1, Math.round(height * dpr));
        resize_canvas(dpr);
        scheduleRender();
        postOk(msg.id);
        break;
      }

      case 'open': {
        const { format: f, bytes } = msg.payload;
        format = f;
        doc = f === 'docx' ? open_docx(new Uint8Array(bytes)) : open_xlsx(new Uint8Array(bytes));
        const meta = f === 'docx'
          ? { pages: extractNumber(doc, 'pageCount') }
          : { sheets: extractStringArray(doc, 'sheets') };
        const reply: WorkerResponse = { id: msg.id, type: 'ready', meta };
        ctxSelf.postMessage(reply);
        break;
      }

      case 'render': {
        lastRender = msg.payload;
        scheduleRender();
        postOk(msg.id);
        break;
      }

      case 'hitTest': {
        const ref = hit_test(msg.payload.x, msg.payload.y);
        const reply: WorkerResponse = { id: msg.id, type: 'hit', ref: (ref ?? null) as never };
        ctxSelf.postMessage(reply);
        break;
      }

      case 'exportPdf': {
        const bytes = format === 'docx'
          ? export_docx_to_pdf(JSON.stringify(msg.payload))
          : export_xlsx_workbook_to_pdf(JSON.stringify(msg.payload));
        const u8 = toU8(bytes);
        const reply: WorkerResponse = { id: msg.id, type: 'pdf', bytes: u8 };
        ctxSelf.postMessage(reply, [u8.buffer as ArrayBuffer]);
        break;
      }

      case 'exportPng': {
        // TODO (Фаза 2): растровый экспорт через OffscreenCanvas.convertToBlob.
        throw new Error('exportPng: not yet implemented');
      }

      case 'dispose':
        ctxSelf.close();
        break;

      default:
        throw new Error(`unknown request type: ${(msg as { type: string }).type}`);
    }
  } catch (e) {
    const err = e as Error;
    const reply: WorkerResponse = { id: msg.id, type: 'error', message: String(err.message ?? err) };
    if (err.stack) reply.stack = err.stack;
    ctxSelf.postMessage(reply);
  }
};

function scheduleRender(): void {
  if (rafScheduled) return;
  rafScheduled = true;
  if (typeof ctxSelf.requestAnimationFrame === 'function') {
    ctxSelf.requestAnimationFrame(runFrame);
  } else {
    setTimeout(runFrame, 16);
  }
}

function runFrame(): void {
  rafScheduled = false;
  if (!ctx || !doc || !lastRender) return;
  const t0 = performance.now();
  const dlJson = build_display_list(JSON.stringify(lastRender));
  const t1 = performance.now();
  paint_display_list_to_offscreen(ctx, dlJson);
  const t2 = performance.now();
  const timings: FrameTimings = { buildMs: t1 - t0, paintMs: t2 - t1 };
  const reply: WorkerResponse = { id: 0, type: 'tick', frameId: performance.now(), timings };
  ctxSelf.postMessage(reply);
}

function postOk(id: number, result?: unknown): void {
  const reply: WorkerResponse = { id, type: 'ok', result };
  ctxSelf.postMessage(reply);
}

function toU8(v: unknown): Uint8Array {
  if (v instanceof Uint8Array) return v;
  if (v instanceof ArrayBuffer) return new Uint8Array(v);
  throw new Error('expected Uint8Array from WASM');
}

function extractNumber(o: unknown, key: string): number | undefined {
  if (o && typeof o === 'object' && key in o) {
    const v = (o as Record<string, unknown>)[key];
    if (typeof v === 'number') return v;
  }
  return undefined;
}

function extractStringArray(o: unknown, key: string): string[] | undefined {
  if (o && typeof o === 'object' && key in o) {
    const v = (o as Record<string, unknown>)[key];
    if (Array.isArray(v) && v.every((x) => typeof x === 'string')) return v as string[];
  }
  return undefined;
}
