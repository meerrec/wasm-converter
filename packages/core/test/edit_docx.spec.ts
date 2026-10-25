import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { OutMsg, PageInfo } from '../src/protocol.js';
import { createDocxViewer, type DocxViewerHandle } from '../src/render/docx_viewer.js';

/** Страница раскладки — размер один; после правки воркер вернёт новые. */
const PAGES: PageInfo[] = [{ width: 595, height: 842 }];
const EDITED_PAGES: PageInfo[] = [{ width: 595, height: 900 }];

type MessageHandler = (ev: { data: unknown }) => void;

/** Двойник воркера: копит отправленное и отвечает на init/open/edit-docx. */
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
      this.reply({ type: 'docx-opened', id: msg.id, pages: PAGES } satisfies OutMsg);
    }
    if (msg.type === 'edit-docx') {
      this.reply({ type: 'docx-edited', id: msg.id, pages: EDITED_PAGES } satisfies OutMsg);
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

let viewer: DocxViewerHandle | null = null;

/** Собрать просмотрщик на двойниках; наружу — то, чем двигает тест. */
async function openViewer(): Promise<{ viewer: DocxViewerHandle; worker: FakeWorker }> {
  viewer = await createDocxViewer(new FakeElement('div') as unknown as HTMLElement);
  const worker = FakeWorker.last;
  if (!worker) throw new Error('viewer did not build its worker');
  return { viewer, worker };
}

/** Дать осесть микрозадачам: в них воркер отвечает и резолвятся промисы. */
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
  viewer?.destroy();
  viewer = null;
  vi.unstubAllGlobals();
});

describe('правка run в DOCX', () => {
  it('отправляет edit-docx, ждёт ответ и перестраивает страницы', async () => {
    const { viewer: handle, worker } = await openViewer();
    await handle.open(new ArrayBuffer(8));

    await handle.editRun(1, 2, 'новый текст');
    await flush();

    expect(worker.last('edit-docx')).toMatchObject({
      type: 'edit-docx',
      paragraph: 1,
      run: 2,
      text: 'новый текст',
    });
    // Ответ воркера принёс новые размеры страниц — стопка перестроена.
    expect(handle.pages).toEqual(EDITED_PAGES);
    // Кадр заказан сразу: сообщения воркер читает по порядку, и правка ляжет
    // в документ раньше сборки.
    expect(worker.last('render')).toBeDefined();
  });

  it('молчит, пока документ не открыт', async () => {
    const { viewer: handle, worker } = await openViewer();

    await handle.editRun(0, 0, 'некуда писать');
    await flush();

    expect(worker.last('edit-docx')).toBeUndefined();
  });

  it('после destroy воркер не трогает', async () => {
    const { viewer: handle, worker } = await openViewer();
    await handle.open(new ArrayBuffer(8));
    handle.destroy();

    await handle.editRun(0, 0, 'поздно');
    await flush();

    expect(worker.last('edit-docx')).toBeUndefined();
  });
});
