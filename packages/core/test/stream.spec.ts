import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PageInfo } from '../src/protocol.js';
import { createDocxViewer, type DocxViewerHandle } from '../src/render/docx_viewer.js';
import { createXlsxViewer, type XlsxViewerHandle } from '../src/render/xlsx_viewer.js';

/**
 * Стримовое открытие (ADR-0031): main читает `ReadableStream` и шлёт воркеру
 * `open-stream` → `append` ×N → `end-stream`; прогресс — через `onProgress`.
 * Оборванный читателем поток помечается `aborted: true`, и промис открытия
 * реджектится — ошибкой воркера или, если воркер промолчал, локально.
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
async function startXlsx(
  options: Parameters<typeof createXlsxViewer>[1] = {},
): Promise<{ viewer: XlsxViewerHandle; worker: FakeWorker }> {
  const viewer = await createXlsxViewer(new FakeElement('div') as unknown as HTMLElement, options);
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

/** Поток из готовых чанков; `fail` — читатель падает на первом чтении. */
function streamOf(chunks: Uint8Array[], fail?: Error): ReadableStream<Uint8Array> {
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      const next = chunks.shift();
      if (fail) controller.error(fail);
      else if (next) controller.enqueue(next);
      else controller.close();
    },
  });
}

/** Микротакты: прокачка стрима идёт асинхронно, сообщения копятся не сразу. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

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
  for (const viewer of viewers) viewer.destroy();
  viewers.length = 0;
  vi.unstubAllGlobals();
});

describe('стримовое открытие XLSX', () => {
  it('шлёт open-stream, append-сообщения и end-stream; прогресс идёт по чанкам', async () => {
    const onProgress = vi.fn();
    const { viewer, worker } = await startXlsx({ onProgress });
    const bytes = new Uint8Array(16).fill(7);
    const opening = viewer.open(streamOf([bytes.subarray(0, 8), bytes.subarray(8)]), { size: 16 });

    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    const id = worker.lastId('open-stream');
    expect(id).toBeDefined();
    expect(worker.all('append').map((m) => m.id)).toEqual([id, id]);
    expect(worker.all('append').map((m) => (m.bytes as ArrayBuffer).byteLength)).toEqual([8, 8]);
    expect(worker.all('open-stream')[0]?.totalSize).toBe(16);
    expect(onProgress.mock.calls).toEqual([
      [8, 16],
      [16, 16],
    ]);

    worker.reply({ type: 'opened', id, sheets: [SHEET] });
    await expect(opening).resolves.toEqual([SHEET]);
  });

  it('чанк, владеющий буфером, уезжает без копии; вью — срезом', async () => {
    const { viewer, worker } = await startXlsx();
    // Два самостоятельных чанка: каждый владеет своим ArrayBuffer целиком —
    // так отдают fetch().body и File.stream(). ADR-0049: transfer без копии.
    const first = new Uint8Array([1, 2, 3, 4]);
    const second = new Uint8Array([5, 6, 7, 8]);
    const opening = viewer.open(streamOf([first, second]), { size: 8 });

    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    const posted = worker.all('append').map((m) => m.bytes as ArrayBuffer);
    // Тождество, а не равенство: копия была бы другим ArrayBuffer.
    expect(posted[0]).toBe(first.buffer);
    expect(posted[1]).toBe(second.buffer);

    const id = worker.lastId('open-stream');
    worker.reply({ type: 'opened', id, sheets: [SHEET] });
    await expect(opening).resolves.toEqual([SHEET]);
  });

  it('вью поверх общего буфера копируется ровно своим срезом', async () => {
    const { viewer, worker } = await startXlsx();
    // Один буфер на два чанка: transfer унёс бы весь буфер, поэтому копия.
    const shared = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    const opening = viewer.open(streamOf([shared.subarray(0, 4), shared.subarray(4)]), { size: 8 });

    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    const posted = worker.all('append').map((m) => m.bytes as ArrayBuffer);
    expect(posted[0]).not.toBe(shared.buffer);
    expect(posted[0]?.byteLength).toBe(4);
    expect(Array.from(new Uint8Array(posted[0]!))).toEqual([1, 2, 3, 4]);
    expect(Array.from(new Uint8Array(posted[1]!))).toEqual([5, 6, 7, 8]);
    // Исходный буфер не тронут: копия отдельная.
    expect(Array.from(shared)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);

    const id = worker.lastId('open-stream');
    worker.reply({ type: 'opened', id, sheets: [SHEET] });
    await expect(opening).resolves.toEqual([SHEET]);
  });

  it('обрыв читателя помечается aborted и реджектит промис ошибкой чтения', async () => {
    const { viewer, worker } = await startXlsx();
    // Локальный reject срабатывает раньше ответа воркера: промис реджектится
    // ошибкой читателя, а воркеру уходит метка aborted — буфер он освободит сам.
    const opening = viewer.open(streamOf([], new Error('network down')));

    await expect(opening).rejects.toThrow('network down');
    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    expect(worker.all('end-stream')[0]?.aborted).toBe(true);
  });

  it('ошибка воркера на потоке реджектит промис по id', async () => {
    const { viewer, worker } = await startXlsx();
    const opening = viewer.open(streamOf([]));

    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    worker.reply({
      type: 'error',
      message: 'stream aborted',
      id: worker.lastId('open-stream'),
      code: 'open',
    });
    await expect(opening).rejects.toThrow('stream aborted');
  });
});

describe('стримовое открытие DOCX', () => {
  it('шлёт open-stream с форматом docx и резолвит docx-opened', async () => {
    const { viewer, worker } = await startDocx();
    const opening = viewer.open(streamOf([new Uint8Array(4)]));

    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    const open = worker.all('open-stream')[0];
    expect(open?.format).toBe('docx');
    expect(open?.totalSize).toBeUndefined();

    worker.reply({ type: 'docx-opened', id: open?.id, pages: PAGES });
    await expect(opening).resolves.toEqual(PAGES);
  });

  it('пустой поток — та же последовательность, отказ отвечает воркер', async () => {
    const { viewer, worker } = await startDocx();
    const opening = viewer.open(streamOf([]));

    await vi.waitFor(() => expect(worker.all('end-stream')).toHaveLength(1));
    expect(worker.all('append')).toHaveLength(0);
    expect(worker.all('end-stream')[0]?.aborted).toBeUndefined();

    // Отказ пустого потока — за воркером: он отвечает ошибкой открытия.
    worker.reply({
      type: 'error',
      message: 'stream aborted',
      id: worker.lastId('open-stream'),
      code: 'open',
    });
    await expect(opening).rejects.toThrow('stream aborted');
  });
});
