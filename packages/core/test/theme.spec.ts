import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { InMsg, OutMsg, RenderConfig, Viewport } from '../src/protocol.js';

/** Ответы wasm-модуля: воркер получает их вместо настоящего `@doc-converter/wasm`. */
const wasm = vi.hoisted(() => ({
  init: vi.fn(async () => {}),
  alloc_sab: vi.fn(() => ({})),
  sab_total_bytes: vi.fn(() => 4096),
  init_painter: vi.fn(),
  resize_canvas: vi.fn(),
  register_bitmap: vi.fn(),
  drop_bitmap: vi.fn(() => true),
  paint_display_list_sab: vi.fn(() => ({ cmds: 1, dropped: false, paintMs: 0 })),
  xlsx_build_display_list_sab: vi.fn(() => ({ written: true, cmds: 1, buildMs: 0 })),
  xlsx_hit_test: vi.fn(() => null),
  xlsx_hyperlink_at: vi.fn(() => null),
  xlsx_open: vi.fn(() => []),
  xlsx_images: vi.fn(() => []),
  xlsx_image_bytes: vi.fn(() => new Uint8Array()),
}));

vi.mock('@doc-converter/wasm', () => ({ default: wasm.init, ...wasm }));

type Handler = (ev: { data: InMsg }) => Promise<void>;

interface SelfStub {
  onmessage: Handler | null;
  postMessage: (msg: OutMsg, transfer?: Transferable[]) => void;
}

let handler: Handler;
let posted: OutMsg[];

const initMsg = (): InMsg => ({
  type: 'init',
  canvas: { getContext: () => ({}) } as unknown as OffscreenCanvas,
  slotCapacity: 1024,
});

const viewport: Viewport = { x: 0, y: 0, w: 800, h: 600, scale: 1 };

const renderMsg = (theme: RenderConfig['theme']): InMsg => ({
  type: 'render',
  req: {
    sheet: 0,
    viewport,
    config: { showGrid: true, showHeaders: true, theme, dpr: 1 },
  },
});

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();

  posted = [];
  const selfStub: SelfStub = {
    onmessage: null,
    postMessage: (msg) => posted.push(msg),
  };
  vi.stubGlobal('self', selfStub);
  // Кадр собирается сразу: тесту нужен вызов wasm с настройками, а не кадры.
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });

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

describe('тёмная тема', () => {
  it('передаёт тёмное оформление в настройках кадра', async () => {
    await handler({ data: renderMsg('dark') });

    expect(wasm.xlsx_build_display_list_sab).toHaveBeenCalledTimes(1);
    const options = wasm.xlsx_build_display_list_sab.mock.calls[0]?.[4];
    expect(options).toEqual({ showGrid: true, showHeaders: true, dark: true });
  });

  it('по умолчанию оставляет светлое оформление', async () => {
    await handler({ data: renderMsg('light') });

    const options = wasm.xlsx_build_display_list_sab.mock.calls[0]?.[4];
    expect(options).toEqual({ showGrid: true, showHeaders: true, dark: false });
  });

  it('смена оформления на лету попадает в следующий кадр', async () => {
    await handler({ data: renderMsg('light') });
    await handler({ data: renderMsg('dark') });

    const options = wasm.xlsx_build_display_list_sab.mock.calls[1]?.[4];
    expect(options).toMatchObject({ dark: true });
  });
});
