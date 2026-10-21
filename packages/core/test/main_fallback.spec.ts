import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PaintStats } from '../src/protocol.js';
import {
  createDocxViewer,
  type DocxViewerHandle,
} from '../src/render/docx_viewer.js';
import {
  createXlsxViewer,
  type XlsxViewerHandle,
} from '../src/render/xlsx_viewer.js';

/**
 * Резервный путь без OffscreenCanvas: painter поднимается на main-потоке.
 *
 * Проверяется решение о режиме и проводка кадра: `auto` молча уходит на main,
 * явный `worker` без `transferControlToOffscreen` — ошибка, `main` вместе с
 * `sab` — ошибка конфигурации. Дальше — что кадр рисуется локальным painter'ом,
 * `tick` несёт `buildMs`, `frame-ack` возвращает буфер и transfer-список,
 * `exportPng` снимает обычный `<canvas>` через `toBlob`, а `destroy`
 * освобождает singleton painter'а.
 *
 * Настоящий `main_painter` тянет `@meerrec/wasm` динамическим импортом;
 * в тестах без собранного пакета он подменён.
 */

const painter = vi.hoisted(() => ({
  initMainPainter: vi.fn(async () => {}),
  paintDisplayList: vi.fn(() => ({ cmds: 12, dropped: false, paintMs: 0.25 })),
  resizeMainCanvas: vi.fn(),
  registerMainBitmap: vi.fn(),
  dropMainBitmap: vi.fn(() => true),
  disposeMainPainter: vi.fn(),
}));

vi.mock('../src/render/main_painter.js', () => painter);

type MessageHandler = (ev: { data: unknown }) => void;

interface Sent {
  msg: Record<string, unknown>;
  transfer: unknown[];
}

/** Двойник воркера: копит отправленное вместе с transfer-списками. */
class FakeWorker {
  static last: FakeWorker | null = null;
  readonly sent: Sent[] = [];
  private readonly handlers = new Set<MessageHandler>();

  constructor() {
    FakeWorker.last = this;
  }

  addEventListener(type: string, fn: MessageHandler): void {
    if (type === 'message') this.handlers.add(fn);
  }

  removeEventListener(type: string, fn: MessageHandler): void {
    this.handlers.delete(fn);
  }

  postMessage(msg: Record<string, unknown>, transfer: unknown[] = []): void {
    this.sent.push({ msg, transfer });
    if (msg.type === 'init') {
      this.reply({
        type: 'ready',
        backend: 'main',
        transport: 'bytes',
        slotCapacity: 1024,
        totalBytes: 2048,
        crossOriginIsolated: false,
      });
    }
  }

  reply(data: unknown): void {
    for (const fn of [...this.handlers]) fn({ data });
  }

  last(type: string): Record<string, unknown> | undefined {
    return [...this.sent].reverse().find(({ msg }) => msg.type === type)?.msg;
  }

  transferOf(type: string): unknown[] {
    return [...this.sent].reverse().find(({ msg }) => msg.type === type)?.transfer ?? [];
  }

  terminate(): void {}
}

/**
 * Двойник элемента без `transferControlToOffscreen`: именно так выглядит
 * браузер, которому main-режим и нужен. У холста есть `getContext` и `toBlob`.
 */
class FakeElement {
  readonly style: Record<string, string> = {};
  readonly children: FakeElement[] = [];
  private readonly listeners = new Map<string, Set<(ev: unknown) => void>>();
  clientWidth = 800;
  clientHeight = 600;
  scrollLeft = 0;
  scrollTop = 0;
  width = 0;
  height = 0;

  constructor(readonly tag: string) {}

  append(...nodes: FakeElement[]): void {
    this.children.push(...nodes);
  }

  remove(): void {}

  addEventListener(type: string, fn: (ev: unknown) => void): void {
    const set = this.listeners.get(type) ?? new Set();
    this.listeners.set(type, set);
    set.add(fn);
  }

  removeEventListener(type: string, fn: (ev: unknown) => void): void {
    this.listeners.get(type)?.delete(fn);
  }

  getBoundingClientRect(): DOMRect {
    return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight } as DOMRect;
  }

  getContext(kind: string): unknown {
    return kind === '2d' ? { canvas: this } : null;
  }

  toBlob(callback: (blob: Blob | null) => void, type = 'image/png'): void {
    callback(new Blob([new Uint8Array([9, 8, 7])], { type }));
  }
}

class FakeResizeObserver {
  observe(): void {}
  disconnect(): void {}
}

const viewers: Array<{ destroy(): void }> = [];

/** Собрать XLSX-просмотрщик на main-режиме; наружу — то, чем двигает тест. */
async function startXlsx(
  options: Parameters<typeof createXlsxViewer>[1] = {},
): Promise<{ viewer: XlsxViewerHandle; worker: FakeWorker }> {
  const viewer = await createXlsxViewer(new FakeElement('div') as unknown as HTMLElement, options);
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

/** Собрать DOCX-просмотрщик на main-режиме; см. `startXlsx`. */
async function startDocx(): Promise<{ viewer: DocxViewerHandle; worker: FakeWorker }> {
  const viewer = await createDocxViewer(new FakeElement('div') as unknown as HTMLElement);
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

beforeEach(() => {
  FakeWorker.last = null;
  vi.clearAllMocks();
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  vi.stubGlobal('devicePixelRatio', 1);
  vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void) => {
    cb(0);
    return 0;
  });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.stubGlobal('document', {
    createElement: (tag: string) => new FakeElement(tag),
  });
});

afterEach(() => {
  for (const viewer of viewers.splice(0)) viewer.destroy();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('выбор режима', () => {
  it('auto без transferControlToOffscreen уходит на main и не отдаёт холст воркеру', async () => {
    const { viewer, worker } = await startXlsx();

    expect(viewer.renderMode).toBe('main');
    expect(viewer.transport).toBe('bytes');

    const init = worker.last('init');
    expect(init?.backend).toBe('main');
    expect(init !== undefined && 'canvas' in init).toBe(false);

    // Painter поднят на main, шрифты и resize — тоже там: воркеру о размерах
    // холста знать нечего.
    expect(painter.initMainPainter).toHaveBeenCalledTimes(1);
    expect(painter.resizeMainCanvas).toHaveBeenCalledWith(800, 600, 1);
    expect(worker.last('resize')).toBeUndefined();
  });

  it('явный worker без OffscreenCanvas — ошибка инициализации', async () => {
    await expect(
      createXlsxViewer(new FakeElement('div') as unknown as HTMLElement, {
        renderMode: 'worker',
      }),
    ).rejects.toThrow(/OffscreenCanvas.*worker/);
    expect(FakeWorker.last).toBeNull();
  });

  it('main вместе с transport sab — ошибка конфигурации', async () => {
    await expect(
      createXlsxViewer(new FakeElement('div') as unknown as HTMLElement, {
        renderMode: 'main',
        transport: 'sab',
      }),
    ).rejects.toThrow(/sab/);
    expect(FakeWorker.last).toBeNull();
  });
});

describe('кадр на main-потоке', () => {
  it('рисует кадр, отдаёт tick с buildMs и возвращает буфер в frame-ack', async () => {
    const { viewer, worker } = await startXlsx();
    const ticks: PaintStats[] = [];
    viewer.onTick((stats) => ticks.push(stats));

    const bytes = new ArrayBuffer(4);
    worker.reply({
      type: 'frame',
      frameId: 7,
      slot: 1,
      bytes,
      len: 4,
      cmds: 12,
      buildMs: 3,
      dropped: false,
    });

    expect(painter.paintDisplayList).toHaveBeenCalledTimes(1);
    const painted = painter.paintDisplayList.mock.calls[0]?.[0] as Uint8Array;
    expect(painted).toBeInstanceOf(Uint8Array);
    expect(painted).toHaveLength(4);

    expect(ticks).toHaveLength(1);
    expect(ticks[0]).toMatchObject({
      cmds: 12,
      dropped: false,
      paintMs: 0.25,
      frameId: 7,
      buildMs: 3,
    });

    const ack = worker.last('frame-ack');
    expect(ack).toMatchObject({ type: 'frame-ack', slot: 1, buffer: bytes });
    expect(worker.transferOf('frame-ack')).toEqual([bytes]);
  });

  it('exportPng снимает холст через toBlob, не спрашивая воркер', async () => {
    const { viewer, worker } = await startXlsx();

    const png = await viewer.exportPng();

    expect([...png]).toEqual([9, 8, 7]);
    expect(worker.last('export-png')).toBeUndefined();
  });

  it('destroy освобождает painter', async () => {
    const { viewer } = await startXlsx();

    viewer.destroy();

    expect(painter.disposeMainPainter).toHaveBeenCalledTimes(1);
  });
});

describe('DOCX на main-потоке', () => {
  it('ресайз и снимок делает main, destroy освобождает painter', async () => {
    const { viewer, worker } = await startDocx();

    expect(viewer.renderMode).toBe('main');
    expect(painter.initMainPainter).toHaveBeenCalledTimes(1);
    expect(painter.resizeMainCanvas).toHaveBeenCalledWith(800, 600, 1);
    expect(worker.last('resize')).toBeUndefined();

    const png = await viewer.exportPng();
    expect([...png]).toEqual([9, 8, 7]);
    expect(worker.last('export-png')).toBeUndefined();

    viewer.destroy();
    expect(painter.disposeMainPainter).toHaveBeenCalledTimes(1);
  });
});
