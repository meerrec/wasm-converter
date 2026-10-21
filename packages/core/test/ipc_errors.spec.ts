import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PageInfo } from '../src/protocol.js';
import {
  createDocxViewer,
  type DocxViewerHandle,
} from '../src/render/docx_viewer.js';
import {
  createXlsxViewer,
  type XlsxViewerHandle,
} from '../src/render/xlsx_viewer.js';

/**
 * Корреляция ответов и ошибок воркера по id.
 *
 * Ошибка с id обязана отклонять только свой запрос: при двух параллельных
 * экспортах отказ одного не должен рушить второй, иначе один сбойный файл
 * уносит с собой всю очередь. Ошибка без id — сбой воркера целиком (init):
 * ответа не будет никому, поэтому отклоняется всё ожидающее. `destroy` тоже
 * обязан завершить незавершённые промисы, иначе они повисают навсегда.
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

/** Двойник воркера: копит отправленное, ответы на открытие/экспорт даёт тест. */
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
    // На `open` и экспорт тест отвечает сам: ему нужен контроль над id и
    // порядком ответов, автоподтверждение лишило бы его обоих.
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

  /** Последнее отправленное сообщение заданного типа. */
  last(type: string): Record<string, unknown> | undefined {
    return [...this.sent].reverse().find((msg) => msg.type === type);
  }

  /** id всех сообщений заданного типа — в порядке отправки. */
  ids(type: string): number[] {
    return this.sent.flatMap((msg) => (msg.type === type ? [msg.id as number] : []));
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

/** Открыть книгу и ответить её id — дальше тест работает с открытой книгой. */
async function openXlsx(viewer: XlsxViewerHandle, worker: FakeWorker): Promise<void> {
  const opening = viewer.open(new ArrayBuffer(8));
  worker.reply({ type: 'opened', id: worker.last('open')?.id, sheets: [SHEET] });
  await opening;
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

describe('адресные ошибки IPC', () => {
  it('ошибка с id отклоняет только свой PDF-экспорт, второй дожидается ответа', async () => {
    const { viewer, worker } = await startXlsx();
    await openXlsx(viewer, worker);

    const first = viewer.exportPdf();
    const second = viewer.exportPdf();
    const [firstId, secondId] = worker.ids('export-pdf');
    expect(firstId).toBeDefined();
    expect(secondId).toBeDefined();
    expect(firstId).not.toBe(secondId);

    // Подписки на исход ставим до ответа: иначе отказ остался бы без обработчика.
    const firstRejected = expect(first).rejects.toThrow('первый лист не выгрузился');
    worker.reply({
      type: 'error',
      message: 'первый лист не выгрузился',
      id: firstId,
      code: 'export',
    });
    await firstRejected;

    const secondResolved = expect(second).resolves.toEqual(new Uint8Array([7, 8]));
    worker.reply({ type: 'pdf', id: secondId, bytes: new Uint8Array([7, 8]) });
    await secondResolved;
  });

  it('ошибка без id отклоняет все ожидающие запросы', async () => {
    const { viewer, worker } = await startXlsx();
    await openXlsx(viewer, worker);

    const pdf = viewer.exportPdf();
    const png = viewer.exportPng();
    const pdfRejected = expect(pdf).rejects.toThrow('воркер не поднялся');
    const pngRejected = expect(png).rejects.toThrow('воркер не поднялся');

    // Без id ошибка относится к воркеру целиком: ответа не будет никому.
    worker.reply({ type: 'error', message: 'воркер не поднялся', code: 'init' });
    await Promise.all([pdfRejected, pngRejected]);
  });

  it('exportPng отклоняется адресной ошибкой и разрешается png-ответом', async () => {
    const { viewer, worker } = await startXlsx();
    await openXlsx(viewer, worker);

    const failed = viewer.exportPng();
    const failedId = worker.ids('export-png')[0];
    const failedRejected = expect(failed).rejects.toThrow('снимок не снялся');
    worker.reply({ type: 'error', message: 'снимок не снялся', id: failedId, code: 'export' });
    await failedRejected;

    const succeeded = viewer.exportPng();
    const succeededId = worker.ids('export-png')[1];
    const succeededResolved = expect(succeeded).resolves.toEqual(new Uint8Array([1, 2]));
    worker.reply({ type: 'png', id: succeededId, bytes: new Uint8Array([1, 2]) });
    await succeededResolved;
  });

  it('opened с id разрешает только своё открытие, а не все ожидающие', async () => {
    const { viewer, worker } = await startXlsx();

    const first = viewer.open(new ArrayBuffer(8));
    const second = viewer.open(new ArrayBuffer(8));
    const [firstId, secondId] = worker.ids('open');

    // Ответ второго открытия не должен задеть первое: оно ещё не завершено.
    const secondResolved = expect(second).resolves.toEqual([SHEET]);
    worker.reply({ type: 'opened', id: secondId, sheets: [SHEET] });
    await secondResolved;

    const firstRejected = expect(first).rejects.toThrow('открытие отменено');
    worker.reply({ type: 'error', message: 'открытие отменено', id: firstId, code: 'open' });
    await firstRejected;
  });

  it('destroy отклоняет незавершённые промисы', async () => {
    const { viewer, worker } = await startXlsx();
    await openXlsx(viewer, worker);

    const pdf = viewer.exportPdf();
    const png = viewer.exportPng();
    const pdfRejected = expect(pdf).rejects.toThrow('viewer destroyed');
    const pngRejected = expect(png).rejects.toThrow('viewer destroyed');

    viewer.destroy();
    await Promise.all([pdfRejected, pngRejected]);
  });
});

describe('адресные ошибки IPC в DOCX', () => {
  it('docx-opened с id разрешает только своё открытие, а не все ожидающие', async () => {
    const { viewer, worker } = await startDocx();

    const first = viewer.open(new ArrayBuffer(8));
    const second = viewer.open(new ArrayBuffer(8));
    const [firstId, secondId] = worker.ids('open');

    const secondResolved = expect(second).resolves.toEqual(PAGES);
    worker.reply({ type: 'docx-opened', id: secondId, pages: PAGES });
    await secondResolved;

    const firstRejected = expect(first).rejects.toThrow('открытие отменено');
    worker.reply({ type: 'error', message: 'открытие отменено', id: firstId, code: 'open' });
    await firstRejected;
  });

  it('ошибка с id отклоняет только свою текстовую выгрузку', async () => {
    const { viewer, worker } = await startDocx();
    const opening = viewer.open(new ArrayBuffer(8));
    worker.reply({ type: 'docx-opened', id: worker.last('open')?.id, pages: PAGES });
    await opening;

    const markdown = viewer.exportMarkdown();
    const html = viewer.exportHtml();
    const [markdownId, htmlId] = worker.ids('export-docx-text');

    const markdownRejected = expect(markdown).rejects.toThrow('markdown не выгрузился');
    worker.reply({
      type: 'error',
      message: 'markdown не выгрузился',
      id: markdownId,
      code: 'export',
    });
    await markdownRejected;

    const htmlResolved = expect(html).resolves.toBe('<html>stub</html>');
    worker.reply({ type: 'docx-text', id: htmlId, text: '<html>stub</html>' });
    await htmlResolved;
  });
});
