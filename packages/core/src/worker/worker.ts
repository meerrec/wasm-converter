/// <reference lib="webworker" />
import init, {
  alloc_sab,
  drop_bitmap,
  init_painter,
  paint_display_list_sab,
  register_bitmap,
  resize_canvas,
  sab_total_bytes,
} from '@doc-converter/wasm';
import type { InMsg, OutMsg } from '../protocol.js';
import { exportPng } from '../render/export_png.js';
import { startFrameLoop } from './frame_loop';

type Ctx = OffscreenCanvasRenderingContext2D;

let ctx: Ctx | null = null;
let sab: SharedArrayBuffer | null = null;
let slotCapacity = 0;
let loop: ReturnType<typeof startFrameLoop> | null = null;

const post = (msg: OutMsg, transfer: Transferable[] = []) => {
  (self as unknown as Worker).postMessage(msg, transfer);
};

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
          build: () => true, // Phase 3/6 заменит на реальный builder
          paint: () => {
            const stats = paint_display_list_sab(sab!, slotCapacity) as {
              cmds: number;
              dropped: boolean;
              paintMs: number;
            };
            return stats;
          },
          onTick: (s) => post({ type: 'tick', stats: s }),
        });
      } catch (e) {
        post({ type: 'error', message: String(e) });
      }
      break;
    }
    case 'resize': {
      if (!ctx) return;
      resize_canvas(ctx, msg.cssW, msg.cssH, msg.dpr);
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
    case 'render': {
      loop?.request();
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
  }
};
