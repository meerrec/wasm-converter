import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { InMsg, OutMsg, RenderConfig, Viewport } from '../src/protocol.js';

/**
 * Ленивость DOCX-модуля: заглушка `test/fixtures/docx-module.js` — это
 * настоящий файл, который воркер грузит рантайм-URL из `init` тем же
 * динамическим импортом, что и `@doc-converter/wasm-docx`. `vi.mock` тут не
 * годится: он подменил бы модуль, но не проверил бы, что до открытия `.docx`
 * его никто не грузит.
 */

/** Ответы основного wasm-модуля: настоящего `@doc-converter/wasm` в тесте нет. */
const wasm = vi.hoisted(() => {
  return {
    init: vi.fn(async () => {}),
    alloc_sab: vi.fn(() => ({})),
    sab_total_bytes: vi.fn(() => 4096),
    init_painter: vi.fn(),
    resize_canvas: vi.fn(),
    register_bitmap: vi.fn(),
    drop_bitmap: vi.fn(() => true),
    font_bytes: vi.fn(() => new Uint8Array(4)),
    paint_display_list_sab: vi.fn(() => ({ cmds: 1, dropped: false, paintMs: 0 })),
    xlsx_build_display_list_sab: vi.fn(
      (_sab: unknown, _slot: number, _sheet: number, _viewport: unknown, _options: unknown) => ({
        written: true,
        cmds: 1,
        buildMs: 0,
      }),
    ),
    xlsx_hit_test: vi.fn(() => null),
    xlsx_hyperlink_at: vi.fn(() => null),
    xlsx_open: vi.fn(() => []),
    xlsx_images: vi.fn(() => []),
    xlsx_image_bytes: vi.fn(() => new Uint8Array()),
  };
});

vi.mock('@doc-converter/wasm', () => ({ default: wasm.init, ...wasm }));

/**
 * Отметки заглушки: вычисление модуля отмечается в `loads`, `init()` — в
 * `initCalls`, каждый вызов функции — в `calls`. Пока отметок нет, модуль не
 * грузили и его функции не звали.
 */
interface DocxStub {
  loads: number;
  initCalls: number;
  pages: { width: number; height: number }[];
  calls: Record<string, unknown[][]>;
}

const docxStub = (): DocxStub | undefined =>
  (globalThis as { __docxStub?: DocxStub }).__docxStub;

/** Вызовы функции заглушки: аргументы каждого вызова. */
const stubCalls = (name: string): unknown[][] => docxStub()?.calls[name] ?? [];

/** URL заглушки — тот же путь, каким его увидит воркер: рантайм-строкой. */
const STUB_URL = new URL('./fixtures/docx-module.js', import.meta.url).href;

type Handler = (ev: { data: InMsg }) => Promise<void>;

interface SelfStub {
  onmessage: Handler | null;
  postMessage: (msg: OutMsg, transfer?: Transferable[]) => void;
}

let handler: Handler;
let posted: OutMsg[];

const viewport: Viewport = { x: 0, y: 0, w: 800, h: 600, scale: 1 };
const config: RenderConfig = { showGrid: true, showHeaders: true, theme: 'light', dpr: 1 };
const SLOT_CAPACITY = 1024;

const initMsg = (withDocx: boolean): InMsg => ({
  type: 'init',
  canvas: { getContext: () => ({}) } as unknown as OffscreenCanvas,
  docx: withDocx ? { module: STUB_URL } : undefined,
  slotCapacity: SLOT_CAPACITY,
});

const openMsg = (format: 'xlsx' | 'docx'): InMsg => ({
  type: 'open',
  format,
  bytes: new ArrayBuffer(8),
});

const renderMsg = (target: { sheet?: number; gap?: number }): InMsg => ({
  type: 'render',
  req: { ...target, viewport, config },
});

/** Поднять воркер заново: своя копия модуля и чистые отметки заглушки. */
async function startWorker(withDocx = true): Promise<void> {
  vi.resetModules();
  vi.clearAllMocks();
  // Модуль заглушки рантайм кэширует по URL: отметки заводим заново, чтобы
  // `loads` считал вычисления только этого сценария.
  (globalThis as { __docxStub?: DocxStub }).__docxStub = undefined;

  posted = [];
  const selfStub: SelfStub = {
    onmessage: null,
    postMessage: (msg) => posted.push(msg),
  };
  vi.stubGlobal('self', selfStub);
  // Кадр собирается сразу: тесту нужен вызов wasm, а не кадры.
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });
  // Пропущенный кадр воркер логирует — для теста это ожидаемый ход событий.
  vi.spyOn(console, 'error').mockImplementation(() => {});

  await import('../src/worker/worker.js');
  const onmessage = selfStub.onmessage;
  if (!onmessage) throw new Error('worker did not subscribe to messages');
  handler = onmessage;

  await handler({ data: initMsg(withDocx) });
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('ленивая загрузка DOCX-модуля', () => {
  it('после init заглушка не загружена и её функции не вызваны', async () => {
    await startWorker();

    // Заглушка заводит отметки при вычислении модуля: объекта нет — модуль не грузили.
    expect(docxStub()).toBeUndefined();
    expect(docxStub()?.calls).toBeUndefined();
    expect(posted.some((m) => m.type === 'error')).toBe(false);
  });

  it('открытие DOCX грузит заглушку ровно один раз и разбирает документ', async () => {
    await startWorker();
    await handler({ data: openMsg('docx') });

    // Модуль вычислен ровно раз, и `init()` звался из `loadDocxWasm` — это и есть загрузка.
    expect(docxStub()?.loads).toBe(1);
    expect(docxStub()?.initCalls).toBe(1);
    expect(stubCalls('docx_open_bytes')).toHaveLength(1);
    expect(posted).toContainEqual({ type: 'docx-opened', pages: docxStub()?.pages });
  });

  it('повторное открытие DOCX не грузит заглушку второй раз', async () => {
    await startWorker();
    await handler({ data: openMsg('docx') });
    const loadsAfterFirst = docxStub()?.loads;
    expect(loadsAfterFirst).toBe(1);

    await handler({ data: openMsg('docx') });

    expect(docxStub()?.initCalls).toBe(1);
    expect(docxStub()?.loads).toBe(loadsAfterFirst);
    expect(stubCalls('docx_open_bytes')).toHaveLength(2);
  });

  it('открытие и кадр XLSX заглушку не трогают', async () => {
    await startWorker();
    await handler({ data: openMsg('xlsx') });
    await handler({ data: renderMsg({ sheet: 0 }) });

    expect(docxStub()).toBeUndefined();
    expect(stubCalls('docx_open_bytes')).toHaveLength(0);
    expect(stubCalls('docx_build_display_list_sab')).toHaveLength(0);
    expect(wasm.xlsx_build_display_list_sab).toHaveBeenCalledTimes(1);
  });

  it('без docx в init открытие отвечает ошибкой, а не падением', async () => {
    await startWorker(false);
    await handler({ data: openMsg('docx') });

    expect(posted).toContainEqual({
      type: 'error',
      message: 'docx module is not configured: init has no docx urls',
    });
    expect(posted.filter((m) => m.type === 'docx-opened')).toHaveLength(0);
    expect(docxStub()).toBeUndefined();
  });
});
