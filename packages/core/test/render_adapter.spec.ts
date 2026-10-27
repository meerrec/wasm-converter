import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { FrameMsg, PaintStats } from '../src/protocol.js';
import { createMainRenderAdapter } from '../src/render/render_adapter.js';

/**
 * Main-сторонний адаптер рендера: два режима, одна поверхность.
 *
 * Проверяется проводка, а не картинка. В воркер-режиме адаптер только
 * пересылает `resize`, а кадр, снимок и битмапы остаются воркеру. В
 * main-режиме он поднимает painter, ставит физические размеры холста,
 * возвращает битмапы в кэш после `resize`, рисует кадр цепочкой
 * (`main_frame_chain`) и возвращает буфер в `frame-ack` с transfer-списком, а
 * `destroy` снимает вытесненный кадр и освобождает singleton painter'а.
 *
 * Настоящий `main_painter` тянет `@meerrec/wasm` динамическим импортом; в
 * тестах без собранного пакета он подменён — настоящий модуль сюда не
 * попадает.
 */

const painter = vi.hoisted(() => ({
  initMainPainter: vi.fn(async (_ctx: unknown, _wasmUrl?: string) => {}),
  paintDisplayList: vi.fn((_bytes: Uint8Array) => ({
    cmds: 12,
    dropped: false,
    paintMs: 0.25,
  })),
  resizeMainCanvas: vi.fn((_cssW: number, _cssH: number, _dpr: number) => {}),
  registerMainBitmap: vi.fn((_id: number, _bitmap: ImageBitmap) => {}),
  dropMainBitmap: vi.fn((_id: number) => true),
  disposeMainPainter: vi.fn(() => {}),
}));

vi.mock('../src/render/main_painter.js', () => painter);

interface Sent {
  msg: Record<string, unknown>;
  transfer: unknown[];
}

/** Двойник воркера: копит отправленное вместе с transfer-списками. */
class FakeWorker {
  readonly sent: Sent[] = [];

  postMessage(msg: object, transfer: unknown[] = []): void {
    this.sent.push({ msg: msg as Record<string, unknown>, transfer });
  }

  last(type: string): Record<string, unknown> | undefined {
    return [...this.sent].reverse().find(({ msg }) => msg.type === type)?.msg;
  }

  transferOf(type: string): unknown[] {
    return [...this.sent].reverse().find(({ msg }) => msg.type === type)?.transfer ?? [];
  }
}

/** Двойник `<canvas>` с 2D-контекстом и `toBlob`, как у настоящего холста. */
class FakeCanvas {
  width = 0;
  height = 0;
  readonly style: Record<string, string> = {};
  /** Контекст один на холст: painter должен получить ровно его. */
  readonly context = { canvas: this as unknown as HTMLCanvasElement };
  readonly getContext = vi.fn((kind: string): unknown =>
    kind === '2d' ? this.context : null,
  );

  toBlob(callback: (blob: Blob | null) => void, type = 'image/png'): void {
    callback(new Blob([new Uint8Array([9, 8, 7])], { type }));
  }
}

/** Собрать адаптер и то, чем двигает тест: холст, воркер и `tick`-и. */
function create(opts: { mode: 'worker' | 'main'; wasmUrl?: string }): {
  adapter: ReturnType<typeof createMainRenderAdapter>;
  canvas: FakeCanvas;
  worker: FakeWorker;
  ticks: PaintStats[];
} {
  const canvas = new FakeCanvas();
  const worker = new FakeWorker();
  const ticks: PaintStats[] = [];
  const adapter = createMainRenderAdapter({
    mode: opts.mode,
    canvas: canvas as unknown as HTMLCanvasElement,
    worker: worker as unknown as Worker,
    // `exactOptionalPropertyTypes`: необязательное поле не принимает `undefined`.
    ...(opts.wasmUrl === undefined ? {} : { wasmUrl: opts.wasmUrl }),
    onTick: (stats) => ticks.push(stats),
  });
  return { adapter, canvas, worker, ticks };
}

/** Кадр из воркера: значения по умолчанию — как у первого кадра документа. */
function frame(overrides: Partial<FrameMsg> = {}): FrameMsg {
  return {
    type: 'frame',
    frameId: 7,
    slot: 1,
    bytes: new ArrayBuffer(8),
    len: 4,
    cmds: 12,
    buildMs: 3,
    dropped: false,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  // Цикл кадра рисует по rAF; в тесте кадр должен быть нарисован сразу.
  vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void) => {
    cb(0);
    return 1;
  });
  vi.stubGlobal('cancelAnimationFrame', () => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('адаптер рендера: воркер-режим', () => {
  it('resize уходит воркеру, кадр и снимок остаются воркеру', async () => {
    const { adapter, canvas, worker } = create({ mode: 'worker' });
    await adapter.init();

    expect(adapter.mode).toBe('worker');
    // Painter и шрифты в этом режиме поднимает воркер, не страница.
    expect(painter.initMainPainter).not.toHaveBeenCalled();

    adapter.resize(800, 600, 2);
    expect(worker.last('resize')).toEqual({
      type: 'resize',
      cssW: 800,
      cssH: 600,
      dpr: 2,
    });
    // Холст offscreen: размеры ставит воркер, локальный painter не при чём.
    expect(canvas.width).toBe(0);
    expect(painter.resizeMainCanvas).not.toHaveBeenCalled();

    // Снимок в этом режиме делает воркер по `export-png`.
    expect(adapter.capturePng()).toBeNull();

    // Кадры приходят только в main-режиме: подтверждать нечего.
    adapter.handleFrame(frame());
    expect(painter.paintDisplayList).not.toHaveBeenCalled();
    expect(worker.last('frame-ack')).toBeUndefined();

    adapter.destroy();
    // Painter'а на странице нет — освобождать нечего.
    expect(painter.disposeMainPainter).not.toHaveBeenCalled();
  });
});

describe('адаптер рендера: main-режим', () => {
  it('init поднимает painter на контексте холста, без wasmUrl — без него', async () => {
    const { adapter, canvas } = create({ mode: 'main' });

    await adapter.init();

    expect(adapter.mode).toBe('main');
    expect(canvas.getContext).toHaveBeenCalledWith('2d');
    expect(painter.initMainPainter).toHaveBeenCalledTimes(1);
    expect(painter.initMainPainter.mock.calls[0]).toEqual([canvas.context]);
  });

  it('init передаёт wasmUrl из опций явно', async () => {
    const { adapter, canvas } = create({ mode: 'main', wasmUrl: '/wasm/main.wasm' });

    await adapter.init();

    expect(painter.initMainPainter).toHaveBeenCalledWith(
      canvas.context,
      '/wasm/main.wasm',
    );
  });

  it('init без 2D-контекста — ошибка инициализации, painter не трогается', async () => {
    const { adapter, canvas } = create({ mode: 'main' });
    canvas.getContext.mockReturnValueOnce(null);

    await expect(adapter.init()).rejects.toThrow(/getContext/);
    expect(painter.initMainPainter).not.toHaveBeenCalled();
  });

  it('resize ставит физические размеры и перерегистрирует битмапы', async () => {
    const { adapter, canvas } = create({ mode: 'main' });
    await adapter.init();
    const bitmap = { close: vi.fn() } as unknown as ImageBitmap;
    adapter.registerBitmap(3, bitmap);

    adapter.resize(800, 600, 2);

    // Размеры холста — физические пиксели, не CSS.
    expect(canvas.width).toBe(1600);
    expect(canvas.height).toBe(1200);
    expect(painter.resizeMainCanvas).toHaveBeenCalledWith(800, 600, 2);
    // Смена размеров сбросила кэш painter'а — картинка вернулась в него.
    expect(painter.registerMainBitmap).toHaveBeenLastCalledWith(3, bitmap);

    // Нулевой размер недопустим: холст в один пиксель, а не нулевой.
    adapter.resize(0, 0, 1);
    expect(canvas.width).toBe(1);
    expect(canvas.height).toBe(1);
  });

  it('registerBitmap/dropBitmap/clearBitmaps ведут реестр и painter', async () => {
    const { adapter } = create({ mode: 'main' });
    await adapter.init();
    const first = { close: vi.fn() } as unknown as ImageBitmap;
    const second = { close: vi.fn() } as unknown as ImageBitmap;

    adapter.registerBitmap(3, first);
    adapter.registerBitmap(4, second);
    expect(painter.registerMainBitmap.mock.calls).toEqual([
      [3, first],
      [4, second],
    ]);

    adapter.dropBitmap(3);
    expect(painter.dropMainBitmap.mock.calls).toEqual([[3]]);

    // Снятый id в кэш после resize не возвращается: в реестре его уже нет.
    adapter.resize(800, 600, 1);
    expect(painter.registerMainBitmap).toHaveBeenLastCalledWith(4, second);

    // `clear-bitmaps` снимает весь набор: id нового документа могут совпасть.
    adapter.clearBitmaps();
    expect(painter.dropMainBitmap.mock.calls).toEqual([[3], [4]]);

    // Реестр пуст: возвращать в кэш после resize нечего.
    const before = painter.registerMainBitmap.mock.calls.length;
    adapter.resize(800, 600, 1);
    expect(painter.registerMainBitmap.mock.calls.length).toBe(before);
  });

  it('handleFrame рисует кадр цепочкой и возвращает буфер в frame-ack', async () => {
    const { adapter, worker, ticks } = create({ mode: 'main' });
    await adapter.init();
    const bytes = new ArrayBuffer(8);

    adapter.handleFrame(frame({ bytes, len: 4 }));

    expect(painter.paintDisplayList).toHaveBeenCalledTimes(1);
    const painted = painter.paintDisplayList.mock.calls[0]?.[0];
    expect(painted).toBeInstanceOf(Uint8Array);
    expect(painted).toHaveLength(4);

    // `tick` несёт статистику painter'а и сборки из кадра: без `buildMs`
    // хост не отличил бы медленную сборку от медленной отрисовки.
    expect(ticks).toEqual([
      { cmds: 12, dropped: false, paintMs: 0.25, frameId: 7, buildMs: 3 },
    ]);

    const ack = worker.last('frame-ack');
    expect(ack).toMatchObject({ type: 'frame-ack', slot: 1, buffer: bytes });
    // Владение буфером возвращается переводом: копии кадра нет.
    expect(worker.transferOf('frame-ack')).toEqual([bytes]);
  });

  it('capturePng снимает локальный холст через toBlob', async () => {
    const { adapter } = create({ mode: 'main' });
    await adapter.init();

    const png = await adapter.capturePng();

    expect(png).not.toBeNull();
    expect([...(png as Uint8Array)]).toEqual([9, 8, 7]);
  });

  it('destroy возвращает неотрисованный кадр, снимает painter и битмапы', async () => {
    // rAF здесь отложенный: нужно состояние «кадр пришёл, но ещё не нарисован».
    const queue: Array<(time: number) => void> = [];
    vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void) => {
      queue.push(cb);
      return queue.length;
    });
    const { adapter, worker } = create({ mode: 'main' });
    await adapter.init();
    const bitmap = { close: vi.fn() } as unknown as ImageBitmap;
    adapter.registerBitmap(3, bitmap);

    const bytes = new ArrayBuffer(8);
    adapter.handleFrame(frame({ slot: 2, bytes }));
    expect(painter.paintDisplayList).not.toHaveBeenCalled();

    adapter.destroy();

    // Вытесненный кадр вернулся воркеру, иначе тот ждал бы буфер вечно.
    expect(painter.paintDisplayList).not.toHaveBeenCalled();
    const ack = worker.last('frame-ack');
    expect(ack).toMatchObject({ type: 'frame-ack', slot: 2, buffer: bytes });
    expect(worker.transferOf('frame-ack')).toEqual([bytes]);

    expect(painter.dropMainBitmap).toHaveBeenCalledWith(3);
    expect(painter.disposeMainPainter).toHaveBeenCalledTimes(1);

    // После destroy адаптер инертен: кадр не рисуется и не подтверждается.
    adapter.handleFrame(frame({ slot: 0 }));
    expect(painter.paintDisplayList).not.toHaveBeenCalled();
    expect(worker.sent.filter(({ msg }) => msg.type === 'frame-ack')).toHaveLength(1);
  });
});
