import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { InMsg, OutMsg, RenderConfig, Viewport } from '../src/protocol.js';

/** Ответы wasm-модуля: воркер получает их вместо настоящего `@meerrec/wasm`. */
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

vi.mock('@meerrec/wasm', () => ({ default: wasm.init, ...wasm }));

/**
 * Отметки заглушки ленивого DOCX-модуля (`test/fixtures/docx-module.js`):
 * воркер грузит её по URL из `init`, поэтому из воркера её не видно — о вызовах
 * она пишет в `globalThis`. Пустой список вызовов — функцию не звали.
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
/** Ёмкость слота из `init`: воркер передаёт её в сборщик кадра как есть. */
const SLOT_CAPACITY = 1024;

/** Окно так, как его ждёт Rust. */
const rustViewport = { scrollX: 0, scrollY: 0, width: 800, height: 600, scale: 1 };

const initMsg = (): InMsg => ({
  type: 'init',
  canvas: { getContext: () => ({}) } as unknown as OffscreenCanvas,
  docx: { module: STUB_URL },
  slotCapacity: SLOT_CAPACITY,
});

const openMsg = (format: 'xlsx' | 'docx'): InMsg => ({
  type: 'open',
  id: 1,
  format,
  bytes: new ArrayBuffer(8),
});

const renderMsg = (target: { sheet?: number; gap?: number }): InMsg => ({
  type: 'render',
  req: { ...target, viewport, config },
});

const ticks = () => posted.filter((m) => m.type === 'tick');

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();

  // Заглушка живёт в реестре модулей рантайма: отметки начинаем с чистого листа.
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

  await handler({ data: initMsg() });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('открытие DOCX', () => {
  it('разбирает документ и отвечает страницами вместо листов', async () => {
    await handler({ data: openMsg('docx') });

    expect(stubCalls('docx_open_bytes')).toHaveLength(1);
    expect(wasm.xlsx_open).not.toHaveBeenCalled();
    expect(posted).toContainEqual({ type: 'docx-opened', id: 1, pages: docxStub()?.pages });
  });
});

describe('кадр DOCX', () => {
  it('идёт через сборщик стопки: окно и зазор, без настроек показа', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: renderMsg({ gap: 16 }) });

    expect(wasm.xlsx_build_display_list_sab).not.toHaveBeenCalled();
    expect(stubCalls('docx_build_display_list_sab')).toHaveLength(1);
    const call = stubCalls('docx_build_display_list_sab')[0];
    expect(call?.[1]).toBe(SLOT_CAPACITY);
    expect(call?.[2]).toEqual(rustViewport);
    expect(call?.[3]).toBe(16);
    // Лишнего аргумента (настроек показа) у былл-функции стопки нет.
    expect(call).toHaveLength(4);
    expect(ticks()).toHaveLength(1);
  });

  it('без зазора между страницами пропускается, а не собирает стопку встык', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: renderMsg({}) });

    expect(stubCalls('docx_build_display_list_sab')).toHaveLength(0);
    expect(ticks()).toHaveLength(0);
  });

  it('до открытия документа не собирается', async () => {
    await handler({ data: renderMsg({ gap: 16 }) });

    expect(stubCalls('docx_build_display_list_sab')).toHaveLength(0);
    expect(ticks()).toHaveLength(0);
  });

  it('после закрытия документа не собирается', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: { type: 'close' } });
    await handler({ data: renderMsg({ gap: 16 }) });

    expect(stubCalls('docx_close')).toHaveLength(1);
    expect(stubCalls('docx_build_display_list_sab')).toHaveLength(0);
  });
});

describe('смена формата', () => {
  it('книга по-прежнему идёт своим сборщиком с настройками показа', async () => {
    await handler({ data: openMsg('xlsx') });
    await handler({ data: renderMsg({ sheet: 1 }) });

    expect(stubCalls('docx_build_display_list_sab')).toHaveLength(0);
    expect(wasm.xlsx_build_display_list_sab).toHaveBeenCalledTimes(1);
    expect(wasm.xlsx_build_display_list_sab.mock.calls[0]?.[2]).toBe(1);
    expect(wasm.xlsx_build_display_list_sab.mock.calls[0]?.[4]).toEqual({
      showGrid: true,
      showHeaders: true,
      dark: false,
    });
  });

  it('открытие книги отпускает разобранный документ', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: openMsg('xlsx') });

    expect(stubCalls('docx_close')).toHaveLength(1);
    expect(wasm.xlsx_open).toHaveBeenCalledTimes(1);
    // Отвечает как на книгу: листы, а не страницы.
    expect(posted.at(-1)).toEqual({ type: 'opened', id: 1, sheets: [] });
  });
});
