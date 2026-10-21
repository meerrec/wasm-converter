import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { trace } from '@opentelemetry/api';
import {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';

import type { PageInfo } from '../src/protocol.js';
import {
  createDocxViewer,
  type DocxViewerHandle,
} from '../src/render/docx_viewer.js';
import {
  createXlsxViewer,
  type XlsxViewerHandle,
} from '../src/render/xlsx_viewer.js';
import { startSpan } from '../src/telemetry.js';

/**
 * Инструментация OpenTelemetry — opt-in (ADR-0037).
 *
 * Без зарегистрированного провайдера tracer — no-op: спаны не записываются,
 * открытие и экспорт ведут себя как раньше. С провайдером спаны `viewer.open`
 * и `viewer.export` существуют, а их атрибуты — только формат, размеры и
 * счётчики: байты документа в атрибуты не попадают никогда.
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

describe('телеметрия без провайдера', () => {
  it('startSpan отдаёт не записывающий спан, и открытие работает как раньше', async () => {
    // Провайдер не зарегистрирован: API отдаёт no-op — ничего не записывается
    // и ничего никуда не уходит (ADR-0037).
    const span = startSpan('viewer.open', { format: 'xlsx', bytes: 8 });
    expect(span.isRecording()).toBe(false);
    span.end();

    const { viewer, worker } = await startXlsx();
    await openXlsx(viewer, worker);
    expect(viewer.sheets).toEqual([SHEET]);
  });
});

describe('телеметрия с провайдером', () => {
  let exporter: InMemorySpanExporter;
  let provider: BasicTracerProvider;

  beforeEach(() => {
    // Свежий экспортёр на каждый тест: shutdown() провайдера останавливает
    // экспортёр, а reset() обратно его не запускает.
    exporter = new InMemorySpanExporter();
    provider = new BasicTracerProvider({
      spanProcessors: [new SimpleSpanProcessor(exporter)],
    });
    provider.register();
  });

  afterEach(async () => {
    await provider.shutdown();
    // Убрать глобальный провайдер: следующий файл тестов живёт без него.
    trace.disable();
  });

  it('открытие XLSX и экспорт PNG дают спаны с размерами, без байтов документа', async () => {
    const { viewer, worker } = await startXlsx();

    const opening = viewer.open(new ArrayBuffer(8));
    worker.reply({ type: 'opened', id: worker.last('open')?.id, sheets: [SHEET] });
    await opening;

    const exporting = viewer.exportPng();
    worker.reply({ type: 'png', id: worker.ids('export-png')[0], bytes: new Uint8Array([1, 2]) });
    await exporting;

    const spans = exporter.getFinishedSpans();
    const openSpan = spans.find((s) => s.name === 'viewer.open');
    expect(openSpan).toBeDefined();
    expect(openSpan?.attributes).toMatchObject({ format: 'xlsx', bytes: 8, sheetCount: 1 });

    const exportSpan = spans.find((s) => s.name === 'viewer.export');
    expect(exportSpan).toBeDefined();
    expect(exportSpan?.attributes).toMatchObject({ format: 'xlsx', kind: 'png', bytes: 2 });

    // Атрибуты — числа и строки из закрытого набора: содержимое документа
    // (байты, буферы) в спаны не попадает никогда.
    for (const span of spans) {
      for (const [key, value] of Object.entries(span.attributes ?? {})) {
        expect(value, `атрибут ${key} спана ${span.name} не должен нести байты`).not.toBeInstanceOf(
          Uint8Array,
        );
        expect(value).not.toBeInstanceOf(ArrayBuffer);
      }
    }
  });

  it('открытие DOCX и экспорт PDF дают спаны с числом страниц и без байтов', async () => {
    const { viewer, worker } = await startDocx();

    const opening = viewer.open(new ArrayBuffer(8));
    worker.reply({ type: 'docx-opened', id: worker.last('open')?.id, pages: PAGES });
    await opening;

    const exporting = viewer.exportPdf();
    worker.reply({
      type: 'pdf',
      id: worker.ids('export-docx-pdf')[0],
      bytes: new Uint8Array([3, 4, 5]),
    });
    await exporting;

    const spans = exporter.getFinishedSpans();
    const openSpan = spans.find((s) => s.name === 'viewer.open');
    expect(openSpan?.attributes).toMatchObject({ format: 'docx', bytes: 8, pageCount: 1 });

    const exportSpan = spans.find((s) => s.name === 'viewer.export');
    expect(exportSpan?.attributes).toMatchObject({ format: 'docx', kind: 'pdf', bytes: 3 });
  });
});
