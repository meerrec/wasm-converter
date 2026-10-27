import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import tsupConfig from '../tsup.config';
import { createDocxViewer, type DocxViewerHandle } from '../src/render/docx_viewer.js';
import { createXlsxViewer, type XlsxViewerHandle } from '../src/render/xlsx_viewer.js';

/**
 * Разделение воркеров по форматам (направление «воркер на формат»): каждая
 * фабрика поднимает свой бандл и шлёт только сообщения своего формата. Общий
 * диспетчер `worker.js` удалён, поэтому проверяем и конфиг сборки: tsup обязан
 * выпускать семь воркер-бандлов, а не один.
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

const PAGES = [{ width: 595, height: 842 }];

type MessageHandler = (ev: { data: unknown }) => void;

/** Двойник воркера: копит отправленное и запоминает URL бандла. */
class FakeWorker {
  static last: FakeWorker | null = null;
  readonly sent: Array<Record<string, unknown>> = [];
  private readonly handlers = new Set<MessageHandler>();

  constructor(readonly url: string | URL) {
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

/** Собрать XLSX-просмотрщик на двойниках; наружу — его воркер. */
async function startXlsx(): Promise<{ viewer: XlsxViewerHandle; worker: FakeWorker }> {
  const viewer = await createXlsxViewer(new FakeElement('div') as unknown as HTMLElement);
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

/** Собрать DOCX-просмотрщик на двойниках; наружу — его воркер. */
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
  vi.restoreAllMocks();
});

describe('бандлы воркеров', () => {
  it('tsup собирает три воркера и не собирает общий worker', () => {
    // Дефолтный экспорт конфига — объект `defineConfig`, а не массив/функция.
    const config = tsupConfig as { entry: Record<string, string> };
    const entry = config.entry;

    expect(entry.xlsx_worker).toBe('src/worker/xlsx_worker.ts');
    expect(entry.docx_worker).toBe('src/worker/docx_worker.ts');
    expect(entry.pptx_worker).toBe('src/worker/pptx_worker.ts');
    expect(entry.xlsb_worker).toBe('src/worker/xlsb_worker.ts');
    expect(entry.ods_worker).toBe('src/worker/ods_worker.ts');
    expect(entry.odp_worker).toBe('src/worker/odp_worker.ts');
    expect(entry.odt_worker).toBe('src/worker/odt_worker.ts');
    expect(entry.worker).toBeUndefined();
    expect(Object.keys(entry).filter((key) => key.endsWith('_worker'))).toEqual([
      'xlsx_worker',
      'docx_worker',
      'pptx_worker',
      'xlsb_worker',
      'ods_worker',
      'odp_worker',
      'odt_worker',
    ]);
  });

  it('фабрики указывают на свои бандлы', async () => {
    const xlsx = await startXlsx();
    expect(String(xlsx.worker.url)).toMatch(/\/worker\/xlsx_worker\.js$/);

    const docx = await startDocx();
    expect(String(docx.worker.url)).toMatch(/\/worker\/docx_worker\.js$/);
  });
});

describe('XLSX-фабрика', () => {
  it('открывает книгу сообщением своего формата и не шлёт DOCX-команды', async () => {
    const { viewer, worker } = await startXlsx();
    const opening = viewer.open(new ArrayBuffer(8));

    const open = worker.all('open')[0];
    expect(open?.format).toBe('xlsx');

    worker.reply({ type: 'opened', id: open?.id, sheets: [SHEET] });
    await expect(opening).resolves.toEqual([SHEET]);

    expect(worker.all('export-docx-pdf')).toHaveLength(0);
    expect(worker.all('export-docx-text')).toHaveLength(0);
  });

  it('экспортирует PNG и PDF через протокол XLSX-воркера', async () => {
    const { viewer, worker } = await startXlsx();

    const png = viewer.exportPng();
    const pngMsg = worker.all('export-png')[0];
    worker.reply({ type: 'png', id: pngMsg?.id, bytes: new Uint8Array([1, 2]) });
    await expect(png).resolves.toEqual(new Uint8Array([1, 2]));

    const pdf = viewer.exportPdf();
    const pdfMsg = worker.all('export-pdf')[0];
    worker.reply({ type: 'pdf', id: pdfMsg?.id, bytes: new Uint8Array([3, 4]) });
    await expect(pdf).resolves.toEqual(new Uint8Array([3, 4]));

    expect(worker.all('export-docx-pdf')).toHaveLength(0);
    expect(worker.all('export-docx-text')).toHaveLength(0);
  });
});

describe('DOCX-фабрика', () => {
  it('открывает документ сообщением своего формата и не шлёт XLSX-команды', async () => {
    const { viewer, worker } = await startDocx();
    const opening = viewer.open(new ArrayBuffer(8));

    const open = worker.all('open')[0];
    expect(open?.format).toBe('docx');

    worker.reply({ type: 'docx-opened', id: open?.id, pages: PAGES });
    await expect(opening).resolves.toEqual(PAGES);

    expect(worker.all('export-pdf')).toHaveLength(0);
    expect(worker.all('hit-test')).toHaveLength(0);
    expect(worker.all('hyperlink-at')).toHaveLength(0);
  });

  it('экспортирует PDF и текст через протокол DOCX-воркера', async () => {
    const { viewer, worker } = await startDocx();

    const pdf = viewer.exportPdf();
    const pdfMsg = worker.all('export-docx-pdf')[0];
    worker.reply({ type: 'pdf', id: pdfMsg?.id, bytes: new Uint8Array([5, 6]) });
    await expect(pdf).resolves.toEqual(new Uint8Array([5, 6]));

    const text = viewer.exportMarkdown();
    const textMsg = worker.all('export-docx-text')[0];
    expect(textMsg?.format).toBe('markdown');
    worker.reply({ type: 'docx-text', id: textMsg?.id, text: '# заголовок' });
    await expect(text).resolves.toBe('# заголовок');

    expect(worker.all('export-pdf')).toHaveLength(0);
  });
});
