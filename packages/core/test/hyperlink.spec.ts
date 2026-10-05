import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { HyperlinkInfo } from '../src/protocol.js';
import {
  createXlsxViewer,
  type XlsxViewerHandle,
  type XlsxViewerOptions,
} from '../src/render/xlsx_viewer.js';

const LINK: HyperlinkInfo = {
  target: 'https://example.com/report',
  display: 'Отчёт',
  tooltip: 'Открыть отчёт',
};

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

/** Двойник воркера: копит отправленное и отвечает так, как велит тест. */
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
      this.reply({ type: 'ready', sab: {}, slotCapacity: 0, totalBytes: 0 });
    }
    if (msg.type === 'open') {
      this.reply({ type: 'opened', sheets: [SHEET] });
    }
  }

  /** Ответ воркера: подписчикам он приходит событием `message`. */
  reply(data: unknown): void {
    for (const fn of [...this.handlers]) fn({ data });
  }

  /** Последнее отправленное сообщение заданного типа. */
  last(type: string): Record<string, unknown> | undefined {
    return [...this.sent].reverse().find((msg) => msg.type === type);
  }

  terminate(): void {}
}

/** Двойник элемента: у просмотрщика вся разметка — стили, дети и события. */
class FakeElement {
  readonly style: Record<string, string> = {};
  readonly children: FakeElement[] = [];
  private readonly listeners = new Map<string, Set<(ev: unknown) => void>>();
  clientWidth = 800;
  clientHeight = 600;
  scrollLeft = 0;
  scrollTop = 0;

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

  /** Довести событие до подписчиков — как это сделал бы браузер. */
  dispatch(type: string, ev: unknown): void {
    for (const fn of [...(this.listeners.get(type) ?? [])]) fn(ev);
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
): Promise<{ viewer: XlsxViewerHandle; canvas: FakeElement; worker: FakeWorker }> {
  viewer = await createXlsxViewer(new FakeElement('div') as unknown as HTMLElement, options);
  const canvas = elements.find((el) => el.tag === 'canvas');
  const worker = FakeWorker.last;
  if (!canvas || !worker) throw new Error('viewer did not build its markup');
  return { viewer, canvas, worker };
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

describe('клик по гиперссылке', () => {
  it('зовёт onHyperlink с целью ссылки под курсором', async () => {
    const onHyperlink = vi.fn();
    const { canvas, viewer: handle, worker } = await openViewer({ onHyperlink });
    await handle.open(new ArrayBuffer(8));

    canvas.dispatch('click', { clientX: 120, clientY: 40 });
    const request = worker.last('hyperlink-at');
    expect(request).toBeDefined();

    worker.reply({ type: 'hyperlink', id: request?.id, link: LINK });
    await flush();

    expect(onHyperlink).toHaveBeenCalledTimes(1);
    expect(onHyperlink).toHaveBeenCalledWith(LINK);
  });

  it('молчит, если под точкой нет ссылки', async () => {
    const onHyperlink = vi.fn();
    const { canvas, viewer: handle, worker } = await openViewer({ onHyperlink });
    await handle.open(new ArrayBuffer(8));

    canvas.dispatch('click', { clientX: 10, clientY: 20 });
    const request = worker.last('hyperlink-at');
    expect(request).toBeDefined();

    worker.reply({ type: 'hyperlink', id: request?.id, link: null });
    await flush();

    expect(onHyperlink).not.toHaveBeenCalled();
  });

  it('не падает и не спрашивает воркер, пока книга не открыта', async () => {
    const onHyperlink = vi.fn();
    const { canvas, worker } = await openViewer({ onHyperlink });

    expect(() => canvas.dispatch('click', { clientX: 5, clientY: 5 })).not.toThrow();
    await flush();

    expect(worker.last('hyperlink-at')).toBeUndefined();
    expect(onHyperlink).not.toHaveBeenCalled();
  });

  it('переводит точку клика в физические пиксели холста', async () => {
    vi.stubGlobal('devicePixelRatio', 2);
    const { canvas, viewer: handle, worker } = await openViewer({ onHyperlink: vi.fn() });
    await handle.open(new ArrayBuffer(8));

    const scroller = elements[0] as FakeElement;
    scroller.scrollLeft = 100;
    scroller.scrollTop = 50;

    canvas.dispatch('click', { clientX: 120, clientY: 40 });
    await flush();

    expect(worker.last('hyperlink-at')).toMatchObject({
      x: 240,
      y: 80,
      viewport: { x: 100, y: 50, w: 1600, h: 1200, scale: 2 },
    });
  });

  it('после destroy клик колбэк не трогает', async () => {
    const onHyperlink = vi.fn();
    const { canvas, viewer: handle, worker } = await openViewer({ onHyperlink });
    await handle.open(new ArrayBuffer(8));
    handle.destroy();

    canvas.dispatch('click', { clientX: 120, clientY: 40 });
    await flush();

    expect(worker.last('hyperlink-at')).toBeUndefined();
    expect(onHyperlink).not.toHaveBeenCalled();
  });
});
