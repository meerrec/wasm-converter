import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createXlsxViewer } from '../src/render/xlsx_viewer.js';

// main-режим поднимает painter, а тот тянет wasm-модуль динамическим импортом:
// в тестах без собранного пакета он подменён (как в `main_fallback.spec.ts`).
// Без подмены подъём падает раньше, чем дело доходит до воркера.
vi.mock('../src/render/main_painter.js', () => ({
  initMainPainter: vi.fn(async () => {}),
  paintDisplayList: vi.fn(() => ({ cmds: 0, dropped: false, paintMs: 0 })),
  resizeMainCanvas: vi.fn(),
  registerMainBitmap: vi.fn(),
  dropMainBitmap: vi.fn(() => true),
  disposeMainPainter: vi.fn(),
}));

/**
 * Отказ загрузки скрипта воркера.
 *
 * О таком отказе браузер сообщает событием `error` на самом `Worker`, а не
 * сообщением по каналу: скрипт, заблокированный COEP-проверкой или не
 * доехавший по сети, не исполняется вовсе, и `init` уходит в никуда. Без
 * слушателя `createViewerBase` ждала бы `ready` вечно — на WebKit так
 * зависало открытие `#/pptx` при повторной навигации (ответ статики, приходивший
 * уже изолированному документу, не проходил проверку).
 */

type Handler = (ev: { data?: unknown; message?: string }) => void;

/** Двойник воркера: `ready` не отвечает, но умеет сообщить об отказе загрузки. */
class FakeWorker {
  static last: FakeWorker | null = null;
  readonly sent: Array<Record<string, unknown>> = [];
  private readonly handlers = new Map<string, Set<Handler>>();

  constructor() {
    FakeWorker.last = this;
  }

  addEventListener(type: string, fn: Handler): void {
    const set = this.handlers.get(type) ?? new Set();
    this.handlers.set(type, set);
    set.add(fn);
  }

  removeEventListener(type: string, fn: Handler): void {
    this.handlers.get(type)?.delete(fn);
  }

  postMessage(msg: Record<string, unknown>): void {
    this.sent.push(msg);
  }

  /** Браузер не загрузил скрипт: событие `error` на воркере. */
  failLoad(message = ''): void {
    for (const fn of [...(this.handlers.get('error') ?? [])]) fn({ message });
  }

  /** Слушает ли кто-то отказ загрузки: без слушателя подъём висит. */
  listensForError(): boolean {
    return (this.handlers.get('error')?.size ?? 0) > 0;
  }

  terminate(): void {}
}

/** Холст main-режима: painter рисует в его `2d`-контекст. */
class FakeCanvas {
  readonly style: Record<string, string> = {};
  clientWidth = 800;
  clientHeight = 600;
  scrollLeft = 0;
  scrollTop = 0;
  append(): void {}
  remove(): void {}
  setAttribute(): void {}
  addEventListener(): void {}
  removeEventListener(): void {}
  getBoundingClientRect(): DOMRect {
    return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight } as DOMRect;
  }
  getContext(kind: string): unknown {
    return kind === '2d' ? { canvas: this } : null;
  }
}

beforeEach(() => {
  FakeWorker.last = null;
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('ResizeObserver', class {
    observe(): void {}
    disconnect(): void {}
  });
  vi.stubGlobal('devicePixelRatio', 1);
  vi.stubGlobal('document', { createElement: () => new FakeCanvas() });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('подъём воркера', () => {
  it('отказ загрузки скрипта — ошибка подъёма, а не вечное ожидание ready', async () => {
    const pending = createXlsxViewer(new FakeCanvas() as unknown as HTMLElement, {
      renderMode: 'main',
    });
    // Подъём идёт через `await` подменённого painter'а: дать ему дойти до
    // подписки — иначе `init` уже отправлен, а ответа ждать некому.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(FakeWorker.last?.listensForError()).toBe(true);
    FakeWorker.last?.failLoad();
    await pending.catch(() => {});

    await expect(pending).rejects.toThrow(/failed to load/);
  });

  it('сообщение браузера об отказе доносится как есть', async () => {
    const pending = createXlsxViewer(new FakeCanvas() as unknown as HTMLElement, {
      renderMode: 'main',
    });
    // Подъём идёт через `await` подменённого painter'а: дать ему дойти до
    // подписки. Без слушателя `init` уже отправлен, а ответа ждать некому.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(FakeWorker.last?.listensForError()).toBe(true);
    FakeWorker.last?.failLoad('Worker load was blocked by Cross-Origin-Embedder-Policy');
    await pending.catch(() => {});

    await expect(pending).rejects.toThrow(/blocked by Cross-Origin-Embedder-Policy/);
  });
});
