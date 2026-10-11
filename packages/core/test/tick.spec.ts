import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  startFrameLoop,
  type BuildOutcome,
  type PaintStats,
} from '../src/worker/frame_loop.js';

/** Кадры выполняем вручную — иначе rAF в node недоступен. */
let frames: Array<() => void>;

const runFrame = () => {
  const queued = frames;
  frames = [];
  for (const f of queued) f();
};

/** Сборка идёт микрозадачей: даём ей выполниться. */
const runBuild = async () => {
  await Promise.resolve();
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
  it('coalesces repeated request() into one build and one frame', async () => {
    let built = 0;
    let painted = 0;
    const loop = startFrameLoop({
      build: () => {
        built++;
        return 'built';
      },
      paint: () => {
        painted++;
        return { cmds: 1, dropped: false, paintMs: 0.5 };
      },
      onTick: () => {},
    });

    loop.request();
    loop.request();
    loop.request();
    await runBuild();
    expect(built).toBe(1);
    expect(frames).toHaveLength(1);

    runFrame();
    expect(painted).toBe(1);
  });

  it('builds before the frame, so paint does not wait for the build', async () => {
    const order: string[] = [];
    const loop = startFrameLoop({
      build: () => {
        order.push('build');
        return 'built';
      },
      paint: () => {
        order.push('paint');
        return { cmds: 1, dropped: false, paintMs: 0.5 };
      },
      onTick: () => {},
    });

    loop.request();
    await runBuild();
    expect(order).toEqual(['build'], 'сборка прошла до кадра экрана');

    runFrame();
    expect(order).toEqual(['build', 'paint']);
  });

  it('reports stats with a growing frameId', async () => {
    const ticks: PaintStats[] = [];
    const loop = startFrameLoop({
      build: () => 'built',
      paint: () => ({ cmds: 7, dropped: false, paintMs: 1.25 }),
      onTick: (s) => ticks.push(s),
    });

    loop.request();
    await runBuild();
    runFrame();
    loop.request();
    await runBuild();
    runFrame();

    expect(ticks).toEqual([
      { cmds: 7, dropped: false, paintMs: 1.25, frameId: 1 },
      { cmds: 7, dropped: false, paintMs: 1.25, frameId: 2 },
    ]);
  });

  it('paints nothing when build() has nothing to draw', async () => {
    let painted = 0;
    let ticks = 0;
    const loop = startFrameLoop({
      build: () => 'idle',
      paint: () => {
        painted++;
        return { cmds: 0, dropped: false, paintMs: 0 };
      },
      onTick: () => ticks++,
    });

    loop.request();
    await runBuild();
    runFrame();

    expect(painted).toBe(0);
    expect(ticks).toBe(0);
  });

  it('retries a dropped build after paint frees the buffer', async () => {
    const outcomes: BuildOutcome[] = ['dropped', 'built'];
    let painted = 0;
    const loop = startFrameLoop({
      build: () => outcomes.shift() ?? 'built',
      paint: () => {
        painted++;
        return { cmds: 1, dropped: false, paintMs: 0.1 };
      },
      onTick: () => {},
    });

    loop.request();
    await runBuild();
    // Буфера не было: сборка отложена, но отрисовка (пустая) всё равно идёт —
    // она и освобождает слот.
    expect(frames).toHaveLength(1);
    runFrame();
    expect(painted).toBe(1);

    // Кадр освободил буфер: сборку повторяем и рисуем уже её.
    await runBuild();
    runFrame();
    expect(painted).toBe(2);
  });

  it('keeps building the newest state while the painter catches up', async () => {
    // Двойная буферизация: пока painter не рисовал, сборщик кладёт второй кадр.
    let built = 0;
    const loop = startFrameLoop({
      build: () => {
        built++;
        return 'built';
      },
      paint: () => ({ cmds: 1, dropped: false, paintMs: 0.1 }),
      onTick: () => {},
    });

    loop.request();
    await runBuild();
    loop.request();
    await runBuild();

    expect(built).toBe(2, 'второй кадр собран до первой отрисовки');
    // Откладывать уже нечего: кадр один, второй сборкой он не перекрывается.
    expect(frames).toHaveLength(1);
    runFrame();
  });

  it('survives a throwing paint and keeps the loop alive', async () => {
    let calls = 0;
    const loop = startFrameLoop({
      build: () => 'built',
      paint: () => {
        calls++;
        if (calls === 1) throw new Error('boom');
        return { cmds: 2, dropped: false, paintMs: 0.1 };
      },
      onTick: () => {},
    });

    loop.request();
    await runBuild();
    expect(() => runFrame()).not.toThrow();

    loop.request();
    await runBuild();
    runFrame();
    expect(calls).toBe(2);
  });

  it('survives a throwing build and keeps the loop alive', async () => {
    let calls = 0;
    const loop = startFrameLoop({
      build: () => {
        calls++;
        if (calls === 1) throw new Error('boom');
        return 'built';
      },
      paint: () => ({ cmds: 1, dropped: false, paintMs: 0.1 }),
      onTick: () => {},
    });

    loop.request();
    await expect(runBuild()).resolves.toBeUndefined();

    loop.request();
    await runBuild();
    runFrame();
    expect(calls).toBe(2);
  });

  it('stops scheduling after stop()', async () => {
    let painted = 0;
    const loop = startFrameLoop({
      build: () => 'built',
      paint: () => {
        painted++;
        return { cmds: 1, dropped: false, paintMs: 0.1 };
      },
      onTick: () => {},
    });

    loop.request();
    loop.stop();
    await runBuild();
    runFrame();

    loop.request();
    await runBuild();
    expect(frames).toHaveLength(0);
    expect(painted).toBe(0);
  });
});
