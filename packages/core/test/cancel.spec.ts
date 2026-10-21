import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PageInfo } from '../src/protocol.js';
import { createDocxViewer, type DocxViewerHandle } from '../src/render/docx_viewer.js';
import { createXlsxViewer, type XlsxViewerHandle } from '../src/render/xlsx_viewer.js';

/**
 * Отмена операций (ADR-0032): `AbortSignal` оборачивается в адресный `cancel`
 * воркеру, уже отменённый сигнал реджектит промис без сообщений, `destroy`
 * и новое открытие отменяют ожидающие запросы.
 */

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

const PAGES: PageInfo[] = [{ width: 595, height: 842 }];

type MessageHandler = (ev: { data: unknown }) => void;

/** Двойник воркера: копит отправленное, ответы даёт тест. */
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
        transport: 'bytes',
        slotCapacity: 0,
        totalBytes: 0,
        crossOriginIsolated: false,
      });
    }
  }

  /** Ответ воркера: подписчикам он приходит событием `message`. */
  reply(data: unknown): void {
    for (const fn of [...this.handlers]) fn({ data });
  }

  /** Все отправленные сообщения заданного типа — в порядке отправки. */
  all(type: string): Array<Record<string, unknown>> {
    return this.sent.filter((msg) => msg.type === type);
  }

  /** id последнего отправленного сообщения заданного типа. */
  lastId(type: string): number | undefined {
    return [...this.sent].reverse().find((msg) => msg.type === type)?.id as number | undefined;
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

const viewers: Array<{ destroy(): void }> = [];

/** Собрать XLSX-просмотрщик на двойниках; наружу — то, чем двигает тест. */
async function startXlsx(): Promise<{ viewer: XlsxViewerHandle; worker: FakeWorker }> {
  const viewer = await createXlsxViewer(new FakeElement('div') as unknown as HTMLElement);
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

/** Собрать DOCX-просмотрщик на двойниках; см. `startXlsx`. */
async function startDocx(): Promise<{ viewer: DocxViewerHandle; worker: FakeWorker }> {
  const viewer = await createDocxViewer(new FakeElement('div') as unknown as HTMLElement);
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

beforeEach(() => {
  FakeWorker.last = null;
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  vi.stubGlobal('devicePixelRatio', 1);
  vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void) => {
    cb(0);
    return 0;
  });
  vi.stubGlobal('document', {
    createElement: (tag: string) => new FakeElement(tag),
  });
});

afterEach(() => {
  for (const viewer of viewers.splice(0)) viewer.destroy();
  vi.unstubAllGlobals();
});

describe('отмена открытия', () => {
  it('уже отменённый сигнал реджектит промис без сообщений воркеру', async () => {
    const { viewer, worker } = await startXlsx();
    const controller = new AbortController();
    controller.abort();

    await expect(viewer.open(new ArrayBuffer(8), { signal: controller.signal })).rejects.toThrow(
      'operation cancelled',
    );
    expect(worker.all('open')).toHaveLength(0);
    expect(worker.all('cancel')).toHaveLength(0);
  });

  it('abort во время открытия шлёт адресный cancel и реджектит по ошибке воркера', async () => {
    const { viewer, worker } = await startDocx();
    const controller = new AbortController();
    const opening = viewer.open(new ArrayBuffer(8), { signal: controller.signal });
    controller.abort();

    await vi.waitFor(() => expect(worker.all('cancel')).toHaveLength(1));
    const id = worker.lastId('open');
    expect(worker.all('cancel')[0]?.id).toBe(id);

    const rejected = expect(opening).rejects.toThrow('операция прервана');
    worker.reply({ type: 'error', message: 'операция прервана', id, code: 'cancelled' });
    await rejected;
  });

  it('новое открытие отменяет предыдущее', async () => {
    const { viewer, worker } = await startXlsx();
    const first = viewer.open(new ArrayBuffer(8));
    const second = viewer.open(new ArrayBuffer(8));

    const [firstId, secondId] = worker.all('open').map((msg) => msg.id as number);
    const cancels = worker.all('cancel');
    expect(cancels.some((msg) => msg.id === firstId)).toBe(true);

    // Оба промиса обязаны осесть: первый — ошибкой, второй — ответом.
    const firstRejected = expect(first).rejects.toThrow('прежнее открытие отменено');
    worker.reply({ type: 'error', message: 'прежнее открытие отменено', id: firstId, code: 'cancelled' });
    worker.reply({ type: 'opened', id: secondId, sheets: [SHEET] });
    await firstRejected;
    await second;
  });

  it('destroy во время открытия шлёт cancel и реджектит промис', async () => {
    const { viewer, worker } = await startXlsx();
    const opening = viewer.open(new ArrayBuffer(8));

    const rejected = expect(opening).rejects.toThrow('viewer destroyed');
    viewer.destroy();
    await rejected;

    expect(worker.all('cancel')).toHaveLength(1);
    expect(worker.all('cancel')[0]?.id).toBe(worker.lastId('open'));
  });
});

describe('отмена экспорта', () => {
  it('abort шлёт cancel с id экспорта, ошибка воркера реджектит промис', async () => {
    const { viewer, worker } = await startXlsx();
    // Книга открыта — экспорт имеет смысл.
    const opening = viewer.open(new ArrayBuffer(8));
    worker.reply({ type: 'opened', id: worker.lastId('open'), sheets: [SHEET] });
    await opening;

    const controller = new AbortController();
    const exporting = viewer.exportPdf({}, { signal: controller.signal });
    controller.abort();

    await vi.waitFor(() => expect(worker.all('cancel')).toHaveLength(1));
    const id = worker.lastId('export-pdf');
    expect(worker.all('cancel')[0]?.id).toBe(id);

    const rejected = expect(exporting).rejects.toThrow('экспорт прерван');
    worker.reply({ type: 'error', message: 'экспорт прерван', id, code: 'cancelled' });
    await rejected;
  });

  it('уже отменённый сигнал реджектит экспорт без сообщений', async () => {
    const { viewer, worker } = await startXlsx();
    const opening = viewer.open(new ArrayBuffer(8));
    worker.reply({ type: 'opened', id: worker.lastId('open'), sheets: [SHEET] });
    await opening;

    const controller = new AbortController();
    controller.abort();
    await expect(viewer.exportPng({ signal: controller.signal })).rejects.toThrow(
      'operation cancelled',
    );
    expect(worker.all('cancel')).toHaveLength(0);
  });
});
