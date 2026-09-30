import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { startFrameLoop, type PaintStats } from '../src/worker/frame_loop.js';

/** Кадры выполняем вручную — иначе rAF в node недоступен. */
let frames: Array<() => void>;

const runFrame = () => {
  const queued = frames;
  frames = [];
  for (const f of queued) f();
};

beforeEach(() => {
  frames = [];
  vi.stubGlobal('requestAnimationFrame', (cb: () => void) => {
    frames.push(cb);
    return frames.length;
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('startFrameLoop', () => {
  it('coalesces repeated request() into one frame', () => {
    let painted = 0;
    const loop = startFrameLoop({
      build: () => true,
      paint: () => {
        painted++;
        return { cmds: 1, dropped: false, paintMs: 0.5 };
      },
      onTick: () => {},
    });

    loop.request();
    loop.request();
    loop.request();
    expect(frames).toHaveLength(1);

    runFrame();
    expect(painted).toBe(1);
  });

  it('reports stats with a growing frameId', () => {
    const ticks: PaintStats[] = [];
    const loop = startFrameLoop({
      build: () => true,
      paint: () => ({ cmds: 7, dropped: false, paintMs: 1.25 }),
      onTick: (s) => ticks.push(s),
    });

    loop.request();
    runFrame();
    loop.request();
    runFrame();

    expect(ticks).toEqual([
      { cmds: 7, dropped: false, paintMs: 1.25, frameId: 1 },
      { cmds: 7, dropped: false, paintMs: 1.25, frameId: 2 },
    ]);
  });

  it('skips paint when build() reports nothing to draw', () => {
    let painted = 0;
    let ticks = 0;
    const loop = startFrameLoop({
      build: () => false,
      paint: () => {
        painted++;
        return { cmds: 0, dropped: false, paintMs: 0 };
      },
      onTick: () => ticks++,
    });

    loop.request();
    runFrame();

    expect(painted).toBe(0);
    expect(ticks).toBe(0);
  });

  it('survives a throwing paint and keeps the loop alive', () => {
    let calls = 0;
    const loop = startFrameLoop({
      build: () => true,
      paint: () => {
        calls++;
        if (calls === 1) throw new Error('boom');
        return { cmds: 2, dropped: false, paintMs: 0.1 };
      },
      onTick: () => {},
    });

    loop.request();
    expect(() => runFrame()).not.toThrow();

    loop.request();
    runFrame();
    expect(calls).toBe(2);
  });

  it('stops scheduling after stop()', () => {
    let painted = 0;
    const loop = startFrameLoop({
      build: () => true,
      paint: () => {
        painted++;
        return { cmds: 1, dropped: false, paintMs: 0.1 };
      },
      onTick: () => {},
    });

    loop.request();
    loop.stop();
    runFrame();

    loop.request();
    expect(frames).toHaveLength(0);
    expect(painted).toBe(0);
  });
});
