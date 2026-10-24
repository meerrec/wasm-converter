import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { OutMsg } from '../src/protocol.js';
import {
  createXlsxViewer,
  type XlsxViewerHandle,
  type XlsxViewerOptions,
} from '../src/render/xlsx_viewer.js';

const SHEET = {
  name: 'Лист1',
  index: 0,
  hidden: false,
  width: 800,
  height: 600,
  frozenCols: 0,
  frozenRows: 0,
  frozenWidth: 0,
  frozenHeight: 0,
};

type MessageHandler = (ev: { data: unknown }) => void;

/** Двойник воркера: копит отправленное и отвечает на init/open. */
class FakeWorker {
  static last: FakeWorker | null = null;
  readonly sent: Array<Record<string, unknown>> = [];
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

  postMessage(msg: Record<string, unknown>, ..._rest: unknown[]): void {
    this.sent.push(msg);
    if (msg.type === 'init') {
      this.reply({
        type: 'ready',
        backend: 'worker',
        transport: 'bytes',
        slotCapacity: 0,
        totalBytes: 0,
        crossOriginIsolated: false,
      } satisfies OutMsg);
    }
    if (msg.type === 'open') {
      this.reply({ type: 'opened', id: msg.id, sheets: [SHEET] } satisfies OutMsg);
    }
  }

  reply(data: unknown): void {
    for (const fn of [...this.handlers]) fn({ data });
  }

  last(type: string): Record<string, unknown> | undefined {
    return [...this.sent].reverse().find((msg) => msg.type === type);
  }

  terminate(): void {}
}

class FakeElement {
  readonly style: Record<string, string> = {};
  readonly children: FakeElement[] = [];
  private readonly listeners = new Map<string, Set<(ev: unknown) => void>>();
  clientWidth = 800;
  clientHeight = 600;
  scrollLeft = 0;
  scrollTop = 0;

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

  transferControlToOffscreen(): OffscreenCanvas {
    return { width: 0, height: 0 } as OffscreenCanvas;
  }
}

class FakeResizeObserver {
  observe(): void {}
  disconnect(): void {}
}

let elements: FakeElement[] = [];
let viewer: XlsxViewerHandle | null = null;

/** Собрать просмотрщик на двойниках; наружу — то, чем двигает тест. */
async function openViewer(
  options: XlsxViewerOptions = {},
): Promise<{ viewer: XlsxViewerHandle; worker: FakeWorker }> {
  viewer = await createXlsxViewer(new FakeElement('div') as unknown as HTMLElement, options);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not build its worker');
  return { viewer, worker };
}

/** Дать осесть микрозадачам: в них воркер отвечает и резолвятся промисы. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  elements = [];
  FakeWorker.last = null;
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  vi.stubGlobal('devicePixelRatio', 1);
  vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void) => {
    cb(0);
    return 0;
  });
  vi.stubGlobal('document', {
    createElement: (tag: string) => {
      const el = new FakeElement(tag);
      elements.push(el);
      return el;
    },
  });
});

afterEach(() => {
  viewer?.destroy();
  viewer = null;
  vi.unstubAllGlobals();
});

describe('правка ячейки', () => {
  it('отправляет edit-cell и сразу перерисовывает кадр', async () => {
    const { viewer: handle, worker } = await openViewer();
    await handle.open(new ArrayBuffer(8));

    handle.setCellText(0, 2, 3, 'новое значение');
    await flush();

    expect(worker.last('edit-cell')).toMatchObject({
      type: 'edit-cell',
      sheet: 0,
      row: 2,
      col: 3,
      text: 'новое значение',
    });
    // Кадр заказан сразу: воркер читает сообщения по порядку, и правка ляжет
    // в книгу раньше сборки.
    expect(worker.last('render')).toBeDefined();
  });

  it('молчит, пока книга не открыта', async () => {
    const { viewer: handle, worker } = await openViewer();

    handle.setCellText(0, 0, 0, 'некуда писать');
    await flush();

    expect(worker.last('edit-cell')).toBeUndefined();
  });

  it('после destroy воркер не трогает', async () => {
    const { viewer: handle, worker } = await openViewer();
    await handle.open(new ArrayBuffer(8));
    handle.destroy();

    handle.setCellText(0, 0, 0, 'поздно');
    await flush();

    expect(worker.last('edit-cell')).toBeUndefined();
  });
});
