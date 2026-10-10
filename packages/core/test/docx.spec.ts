import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { InMsg, OutMsg, RenderConfig, Viewport } from '../src/protocol.js';

/** Ответы wasm-модуля: воркер получает их вместо настоящего `@doc-converter/wasm`. */
const wasm = vi.hoisted(() => {
  const pages = [
    { width: 595, height: 842 },
    { width: 595, height: 842 },
  ];
  return {
    pages,
    init: vi.fn(async () => {}),
    alloc_sab: vi.fn(() => ({})),
    sab_total_bytes: vi.fn(() => 4096),
    init_painter: vi.fn(),
    resize_canvas: vi.fn(),
    register_bitmap: vi.fn(),
    drop_bitmap: vi.fn(() => true),
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
    docx_open_bytes: vi.fn(() => pages),
    /** Страничный сборщик кадра: настроек показа у него в подписи нет. */
    docx_build_display_list_sab: vi.fn(
      (_sab: unknown, _slot: number, _page: number, _viewport: unknown) => ({
        written: true,
        cmds: 1,
        buildMs: 0,
      }),
    ),
    docx_close: vi.fn(),
  };
});

vi.mock('@doc-converter/wasm', () => ({ default: wasm.init, ...wasm }));

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
  slotCapacity: SLOT_CAPACITY,
});

const openMsg = (format: 'xlsx' | 'docx'): InMsg => ({
  type: 'open',
  format,
  bytes: new ArrayBuffer(8),
});

const renderMsg = (target: { sheet?: number; page?: number }): InMsg => ({
  type: 'render',
  req: { ...target, viewport, config },
});

const ticks = () => posted.filter((m) => m.type === 'tick');

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();

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

    expect(wasm.docx_open_bytes).toHaveBeenCalledTimes(1);
    expect(wasm.xlsx_open).not.toHaveBeenCalled();
    expect(posted).toContainEqual({ type: 'docx-opened', pages: wasm.pages });
  });
});

describe('кадр DOCX', () => {
  it('идёт через сборщик страниц: страница и окно, без настроек показа', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: renderMsg({ page: 1 }) });

    expect(wasm.xlsx_build_display_list_sab).not.toHaveBeenCalled();
    expect(wasm.docx_build_display_list_sab).toHaveBeenCalledTimes(1);
    const call = wasm.docx_build_display_list_sab.mock.calls[0];
    expect(call?.[1]).toBe(SLOT_CAPACITY);
    expect(call?.[2]).toBe(1);
    expect(call?.[3]).toEqual(rustViewport);
    // Лишнего аргумента (настроек показа) у былл-функции страницы нет.
    expect(call).toHaveLength(4);
    expect(ticks()).toHaveLength(1);
  });

  it('без номера страницы пропускается, а не рисует первую', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: renderMsg({}) });

    expect(wasm.docx_build_display_list_sab).not.toHaveBeenCalled();
    expect(ticks()).toHaveLength(0);
  });

  it('до открытия документа не собирается', async () => {
    await handler({ data: renderMsg({ page: 0 }) });

    expect(wasm.docx_build_display_list_sab).not.toHaveBeenCalled();
    expect(ticks()).toHaveLength(0);
  });

  it('после закрытия документа не собирается', async () => {
    await handler({ data: openMsg('docx') });
    await handler({ data: { type: 'close' } });
    await handler({ data: renderMsg({ page: 0 }) });

    expect(wasm.docx_close).toHaveBeenCalledTimes(1);
    expect(wasm.docx_build_display_list_sab).not.toHaveBeenCalled();
  });
});

describe('смена формата', () => {
  it('книга по-прежнему идёт своим сборщиком с настройками показа', async () => {
    await handler({ data: openMsg('xlsx') });
    await handler({ data: renderMsg({ sheet: 1 }) });

    expect(wasm.docx_build_display_list_sab).not.toHaveBeenCalled();
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

    expect(wasm.docx_close).toHaveBeenCalledTimes(1);
    expect(wasm.xlsx_open).toHaveBeenCalledTimes(1);
    // Отвечает как на книгу: листы, а не страницы.
    expect(posted.at(-1)).toEqual({ type: 'opened', sheets: [] });
  });
});
