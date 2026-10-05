/// <reference lib="webworker" />
import init, {
  alloc_sab,
  drop_bitmap,
  init_painter,
  paint_display_list_sab,
  register_bitmap,
  resize_canvas,
  sab_total_bytes,
  xlsx_build_display_list_sab,
  xlsx_hit_test,
  xlsx_open,
} from '@doc-converter/wasm';
import type { InMsg, OutMsg, PaintStats, RenderRequest, SheetInfo } from '../protocol.js';
import { exportPng } from '../render/export_png.js';
import { startFrameLoop } from './frame_loop';

type Ctx = OffscreenCanvasRenderingContext2D;

let ctx: Ctx | null = null;
let sab: SharedArrayBuffer | null = null;
let slotCapacity = 0;
let loop: ReturnType<typeof startFrameLoop> | null = null;
/** Последний запрос на кадр: перерисовка идёт по нему, а не по сообщению. */
let pending: RenderRequest | null = null;
/** Сколько заняла сборка последнего кадра. */
let buildMs = 0;

const post = (msg: OutMsg, transfer: Transferable[] = []) => {
  (self as unknown as Worker).postMessage(msg, transfer);
};

/** Окно и настройки в том виде, в каком их ждёт Rust. */
const renderArgs = (req: RenderRequest) => ({
  viewport: {
    scrollX: req.viewport.x,
    scrollY: req.viewport.y,
    width: req.viewport.w,
    height: req.viewport.h,
    scale: req.viewport.scale,
  },
  options: {
    showGrid: req.config.showGrid,
    showHeaders: req.config.showHeaders,
  },
});

self.onmessage = async (ev: MessageEvent<InMsg>) => {
  const msg = ev.data;
  switch (msg.type) {
    case 'init': {
      if (ctx) {
        post({ type: 'error', message: 'worker already initialised' });
        return;
      }
      try {
        await init(msg.wasmUrl);
        ctx = msg.canvas.getContext('2d', {
          alpha: true,
          desynchronized: true,
        }) as Ctx;
        if (!ctx) throw new Error('getContext("2d") returned null');

        slotCapacity = msg.slotCapacity;
        // SAB — не transferable, шарится через structured clone.
        sab = alloc_sab(slotCapacity) as unknown as SharedArrayBuffer;
        post({
          type: 'ready',
          sab,
          slotCapacity,
          totalBytes: sab_total_bytes(slotCapacity),
        });

        init_painter(ctx);
        loop = startFrameLoop({
          build: () => {
            if (!pending || !sab) return false;
            const { viewport, options } = renderArgs(pending);
            const stats = xlsx_build_display_list_sab(
              sab,
              slotCapacity,
              pending.sheet ?? 0,
              viewport,
              options,
            ) as { written: boolean; cmds: number; buildMs: number };
            buildMs = stats.buildMs;
            // Свободного слота не было — кадр пропущен намеренно.
            return stats.written;
          },
          paint: () => {
            const stats = paint_display_list_sab(sab!, slotCapacity) as Omit<
              PaintStats,
              'frameId'
            >;
            return stats;
          },
          onTick: (s) => post({ type: 'tick', stats: { ...s, buildMs } }),
        });
      } catch (e) {
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'resize': {
      if (!ctx) return;
      resize_canvas(ctx, msg.cssW, msg.cssH, msg.dpr);
      if (pending) loop?.request();
      break;
    }
    case 'bitmap': {
      register_bitmap(msg.id, msg.bitmap);
      break;
    }
    case 'drop-bitmap': {
      drop_bitmap(msg.id);
      break;
    }
    case 'open': {
      try {
        // wasm-bindgen отдаёт JsValue: разбирает его serde, тип знает только Rust.
        const sheets = xlsx_open(new Uint8Array(msg.bytes)) as SheetInfo[];
        pending = null;
        post({ type: 'opened', sheets });
      } catch (e) {
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'render': {
      if (msg.req) pending = msg.req;
      if (pending) loop?.request();
      break;
    }
    case 'hit-test': {
      try {
        const { viewport, options } = renderArgs({
          sheet: msg.sheet,
          viewport: msg.viewport,
          config: msg.config,
        });
        const cell = xlsx_hit_test(msg.sheet, viewport, msg.x, msg.y) as [number, number];
        post({ type: 'hit', id: msg.id, cell: cell ?? null });
      } catch {
        // Точка вне книги — не ошибка, а отсутствие ответа.
        post({ type: 'hit', id: msg.id, cell: null });
      }
      break;
    }
    case 'export-png': {
      if (!ctx) {
        post({ type: 'error', message: 'export-png before init' });
        break;
      }
      try {
        const bytes = await exportPng(ctx.canvas);
        post({ type: 'png', id: msg.id, bytes }, [bytes.buffer]);
      } catch (e) {
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'close': {
      pending = null;
      break;
    }
  }
};
