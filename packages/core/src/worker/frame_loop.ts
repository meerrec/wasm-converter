import type { PaintStats } from '../protocol.js';

export type { PaintStats };

export interface FrameLoopCallbacks {
  /** Строит DisplayList и кладёт в SAB. true — есть что рисовать. */
  build(): boolean;
  /** Рисует текущий слот. */
  paint(): { cmds: number; dropped: boolean; paintMs: number };
  onTick(stats: PaintStats): void;
}

export function startFrameLoop(cb: FrameLoopCallbacks): {
  request(): void;
  stop(): void;
} {
  let scheduled = false;
  let stopped = false;
  let frameId = 0;

  const tick = () => {
    scheduled = false;
    if (stopped) return;
    try {
      if (cb.build()) {
        const r = cb.paint();
        cb.onTick({ ...r, frameId: ++frameId });
      }
    } catch (e) {
      // Ошибку логируем, loop не роняем.
      console.error('[frame_loop]', e);
    }
  };

  return {
    request() {
      if (scheduled || stopped) return;
      scheduled = true;
      requestAnimationFrame(tick);
    },
    stop() {
      stopped = true;
    },
  };
}
