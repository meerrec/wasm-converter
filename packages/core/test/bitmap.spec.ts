import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Mock } from 'vitest';

import type { InMsg, OutMsg } from '../src/protocol.js';

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

const IMAGES = [
  { id: 3, mime: 'image/png', byteLength: 4 },
  { id: 7, mime: 'image/jpeg', byteLength: 2 },
];

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
let decode: Mock<(blob: Blob) => Promise<ImageBitmap>>;

const initMsg = (): InMsg => ({
  type: 'init',
  canvas: { getContext: () => ({}) } as unknown as OffscreenCanvas,
  slotCapacity: 1024,
});

const openMsg = (): InMsg => ({ type: 'open', format: 'xlsx', bytes: new ArrayBuffer(8) });

/** Дать осесть микрозадачам: в них воркер отвечает и резолвятся промисы. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  wasm.xlsx_open.mockReturnValue([SHEET]);
  wasm.xlsx_images.mockReturnValue(IMAGES);
  wasm.xlsx_image_bytes.mockImplementation((id: number) => new Uint8Array([id]));

  posted = [];
  const selfStub: SelfStub = {
    onmessage: null,
    postMessage: (msg) => posted.push(msg),
  };
  vi.stubGlobal('self', selfStub);
  vi.stubGlobal('requestAnimationFrame', () => 0);

  // Двойник декодирования: id берётся из байтов, MIME — из Blob, чтобы тест
  // видел, какая картинка с каким mime дошла до регистрации.
  decode = vi.fn(async (blob: Blob) => {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const bmp = { id: bytes[0] ?? 0, mime: blob.type, close: vi.fn() };
    return bmp as unknown as ImageBitmap;
  });
  vi.stubGlobal('createImageBitmap', decode);

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

describe('картинки книги', () => {
  it('запрашивает и регистрирует битмапы с их id и mime', async () => {
    await handler({ data: openMsg() });

    expect(wasm.xlsx_images).toHaveBeenCalledTimes(1);
    expect(wasm.xlsx_image_bytes).toHaveBeenCalledWith(3);
    expect(wasm.xlsx_image_bytes).toHaveBeenCalledWith(7);
    expect(wasm.register_bitmap).toHaveBeenCalledWith(
      3,
      expect.objectContaining({ id: 3, mime: 'image/png' }),
    );
    expect(wasm.register_bitmap).toHaveBeenCalledWith(
      7,
      expect.objectContaining({ id: 7, mime: 'image/jpeg' }),
    );
    expect(posted).toContainEqual({ type: 'opened', sheets: [SHEET] });
  });

  it('пропускает битую картинку и открывает книгу с остальными', async () => {
    decode.mockImplementation(async (blob: Blob) => {
      if (blob.type === 'image/png') throw new Error('не декодируется');
      return { id: 7, mime: blob.type, close: vi.fn() } as unknown as ImageBitmap;
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    await handler({ data: openMsg() });

    expect(wasm.register_bitmap).toHaveBeenCalledTimes(1);
    expect(wasm.register_bitmap).toHaveBeenCalledWith(7, expect.objectContaining({ id: 7 }));
    expect(posted).toContainEqual({ type: 'opened', sheets: [SHEET] });
    expect(posted.some((msg) => msg.type === 'error')).toBe(false);
    expect(warn).toHaveBeenCalled();
  });

  it('возвращает битмапы после ресайза', async () => {
    await handler({ data: openMsg() });
    const registered = wasm.register_bitmap.mock.calls.map((call) => [...call]);
    expect(registered).toHaveLength(2);

    wasm.register_bitmap.mockClear();

    await handler({ data: { type: 'resize', cssW: 800, cssH: 600, dpr: 2 } });

    expect(wasm.resize_canvas).toHaveBeenCalledTimes(1);
    expect(wasm.register_bitmap.mock.calls.map((call) => [...call])).toEqual(registered);
  });

  it('не регистрирует картинки книги, которую закрыли во время загрузки', async () => {
    const resolvers: Array<() => void> = [];
    decode.mockImplementation(
      (blob: Blob) =>
        new Promise<ImageBitmap>((resolve) => {
          const bmp = { mime: blob.type, close: vi.fn() };
          resolvers.push(() => resolve(bmp as unknown as ImageBitmap));
        }),
    );

    const opened = handler({ data: openMsg() });
    await flush();
    expect(resolvers).toHaveLength(2);

    await handler({ data: { type: 'close' } });
    for (const resolve of resolvers) resolve();
    await opened;

    expect(wasm.register_bitmap).not.toHaveBeenCalled();
    expect(posted.some((msg) => msg.type === 'opened')).toBe(false);
  });

  it('снимает битмапы при закрытии книги', async () => {
    await handler({ data: openMsg() });

    await handler({ data: { type: 'close' } });

    expect(wasm.drop_bitmap).toHaveBeenCalledWith(3);
    expect(wasm.drop_bitmap).toHaveBeenCalledWith(7);
    expect(wasm.drop_bitmap).toHaveBeenCalledTimes(2);
  });
});
