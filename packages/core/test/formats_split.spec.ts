import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { DocFormat, InMsg, OutMsg } from '../src/protocol.js';
import { createXlsbViewer, type XlsbViewerHandle } from '../src/render/xlsb_viewer.js';
import { createOdsViewer, type OdsViewerHandle } from '../src/render/ods_viewer.js';
import { createOdpViewer, type OdpViewerHandle } from '../src/render/odp_viewer.js';
import { createOdtViewer, type OdtViewerHandle } from '../src/render/odt_viewer.js';

/**
 * Интеграция новых форматов (XLSB/ODS/ODP/ODT) в TS-слой.
 *
 * Проверяются два направления: фабрики шлют `init` с URL своего ленивого
 * wasm-модуля и `open` со своим `format`, а воркеры принимают только свой
 * формат — чужое открытие отбрасывается ошибкой `open` ещё до загрузки модуля.
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

const SLIDES = [{ name: 'Слайд 1', index: 0, width: 960, height: 540 }];

/** Ответы shared-модуля: воркеры новых форматов получают их вместо `@meerrec/wasm`. */
const wasm = vi.hoisted(() => ({
  init: vi.fn(async () => {}),
  alloc_sab: vi.fn(() => ({})),
  sab_total_bytes: vi.fn(() => 4096),
  init_painter: vi.fn(),
  resize_canvas: vi.fn(),
  register_bitmap: vi.fn(),
  drop_bitmap: vi.fn(() => true),
  font_bytes: vi.fn(() => new Uint8Array(4)),
  paint_display_list_bytes: vi.fn(),
  paint_display_list_sab: vi.fn(),
  stream_open_xlsx: vi.fn(() => []),
  xlsx_build_display_list_into: vi.fn(),
  xlsx_build_display_list_sab: vi.fn(),
  xlsx_edits: vi.fn(() => []),
  xlsx_hit_test: vi.fn(() => null),
  xlsx_hyperlink_at: vi.fn(() => null),
  xlsx_images: vi.fn(() => []),
  xlsx_image_bytes: vi.fn(() => new Uint8Array()),
  xlsx_open: vi.fn(() => []),
  xlsx_set_cell_text: vi.fn(),
}));

vi.mock('@meerrec/wasm', () => ({ default: wasm.init, ...wasm }));

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

/** Собрать XLSB-просмотрщик на двойниках; наружу — его воркер. */
async function startXlsb(): Promise<{ viewer: XlsbViewerHandle; worker: FakeWorker }> {
  const viewer = await createXlsbViewer(new FakeElement('div') as unknown as HTMLElement, {
    xlsbWasm: { module: 'xlsb.js' },
  });
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

/** Собрать ODS-просмотрщик на двойниках; наружу — его воркер. */
async function startOds(): Promise<{ viewer: OdsViewerHandle; worker: FakeWorker }> {
  const viewer = await createOdsViewer(new FakeElement('div') as unknown as HTMLElement, {
    odsWasm: { module: 'ods.js' },
  });
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

/** Собрать ODP-просмотрщик на двойниках; наружу — его воркер. */
async function startOdp(): Promise<{ viewer: OdpViewerHandle; worker: FakeWorker }> {
  const viewer = await createOdpViewer(new FakeElement('div') as unknown as HTMLElement, {
    odpWasm: { module: 'odp.js' },
  });
  viewers.push(viewer);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not create a worker');
  return { viewer, worker };
}

/** Собрать ODT-просмотрщик на двойниках; наружу — его воркер. */
async function startOdt(): Promise<{ viewer: OdtViewerHandle; worker: FakeWorker }> {
  const viewer = await createOdtViewer(new FakeElement('div') as unknown as HTMLElement, {
    odtWasm: { module: 'odt.js' },
  });
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

describe('новые фабрики', () => {
  it('XLSB-фабрика шлёт init со своим модулем и open со своим форматом', async () => {
    const { viewer, worker } = await startXlsb();

    expect(worker.all('init')[0]).toMatchObject({ xlsb: { module: 'xlsb.js' } });

    const opening = viewer.open(new ArrayBuffer(8));
    const open = worker.all('open')[0];
    expect(open?.format).toBe('xlsb');

    worker.reply({ type: 'opened', id: open?.id, sheets: [SHEET] });
    await expect(opening).resolves.toEqual([SHEET]);
  });

  it('ODS-фабрика шлёт init со своим модулем и open со своим форматом', async () => {
    const { viewer, worker } = await startOds();

    expect(worker.all('init')[0]).toMatchObject({ ods: { module: 'ods.js' } });

    const opening = viewer.open(new ArrayBuffer(8));
    const open = worker.all('open')[0];
    expect(open?.format).toBe('ods');

    worker.reply({ type: 'opened', id: open?.id, sheets: [SHEET] });
    await expect(opening).resolves.toEqual([SHEET]);
  });

  it('ODP-фабрика шлёт init со своим модулем и open со своим форматом', async () => {
    const { viewer, worker } = await startOdp();

    expect(worker.all('init')[0]).toMatchObject({ odp: { module: 'odp.js' } });

    const opening = viewer.open(new ArrayBuffer(8));
    const open = worker.all('open')[0];
    expect(open?.format).toBe('odp');

    worker.reply({ type: 'pptx-opened', id: open?.id, slides: SLIDES });
    await expect(opening).resolves.toEqual(SLIDES);
  });

  it('ODT-фабрика шлёт init со своим модулем и open со своим форматом', async () => {
    const { viewer, worker } = await startOdt();

    expect(worker.all('init')[0]).toMatchObject({ odt: { module: 'odt.js' } });

    const opening = viewer.open(new ArrayBuffer(8));
    const open = worker.all('open')[0];
    expect(open?.format).toBe('odt');

    worker.reply({ type: 'docx-opened', id: open?.id, pages: PAGES });
    await expect(opening).resolves.toEqual(PAGES);
  });
});

type Handler = (ev: { data: InMsg }) => Promise<void>;

interface SelfStub {
  onmessage: Handler | null;
  postMessage: (msg: OutMsg, transfer?: Transferable[]) => void;
}

/** Воркеры новых форматов и чужой формат, который они обязаны отвергнуть. */
const WORKERS: Array<{ name: string; path: string; own: DocFormat; foreign: DocFormat }> = [
  { name: 'xlsb', path: '../src/worker/xlsb_worker.js', own: 'xlsb', foreign: 'docx' },
  { name: 'ods', path: '../src/worker/ods_worker.js', own: 'ods', foreign: 'xlsx' },
  { name: 'odp', path: '../src/worker/odp_worker.js', own: 'odp', foreign: 'pptx' },
  { name: 'odt', path: '../src/worker/odt_worker.js', own: 'odt', foreign: 'docx' },
];

describe('новые воркеры: свой и чужой формат', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  for (const worker of WORKERS) {
    it(`${worker.name}-воркер отвергает чужой формат и принимает свой`, async () => {
      const posted: Array<{ msg: OutMsg; transfer: Transferable[] }> = [];
      const selfStub: SelfStub = {
        onmessage: null,
        postMessage: (msg, transfer = []) => posted.push({ msg, transfer }),
      };
      vi.stubGlobal('self', selfStub);
      vi.stubGlobal('requestAnimationFrame', () => 0);

      await import(worker.path);
      const onmessage = selfStub.onmessage;
      if (!onmessage) throw new Error('worker did not subscribe to messages');

      // Чужой формат: ошибка `open` «unsupported format», модуль не грузится.
      await onmessage({
        data: { type: 'open', id: 1, format: worker.foreign, bytes: new ArrayBuffer(8) },
      });
      const foreignErrors = posted.filter(({ msg }) => msg.type === 'error');
      expect(foreignErrors).toHaveLength(1);
      expect(foreignErrors[0]?.msg).toMatchObject({ type: 'error', id: 1, code: 'open' });
      expect(
        (foreignErrors[0]?.msg as { message: string } | undefined)?.message,
      ).toContain('unsupported format');

      // Свой формат: проходит гейт формата, но без `init` нет URL модуля —
      // ошибка «not configured», а не «unsupported format».
      posted.length = 0;
      await onmessage({
        data: { type: 'open', id: 2, format: worker.own, bytes: new ArrayBuffer(8) },
      });
      const ownErrors = posted.filter(({ msg }) => msg.type === 'error');
      expect(ownErrors).toHaveLength(1);
      expect(ownErrors[0]?.msg).toMatchObject({ type: 'error', id: 2, code: 'open' });
      expect(
        (ownErrors[0]?.msg as { message: string } | undefined)?.message,
      ).toContain('not configured');
    });
  }
});
