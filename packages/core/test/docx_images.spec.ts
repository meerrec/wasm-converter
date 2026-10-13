import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Mock } from 'vitest';

import type { ImageInfo, InMsg, OutMsg } from '../src/protocol.js';

/**
 * Картинки DOCX: воркер забирает их из ленивого модуля и отдаёт painter'у.
 * Заглушка модуля — тот же файл, что грузит боевой воркер (`fixtures/
 * docx-module.js`): `vi.mock` подменил бы модуль, но не проверил бы путь
 * «список из wasm → `createImageBitmap` → `register_bitmap`».
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
    xlsx_build_display_list_sab: vi.fn(() => ({ written: true, cmds: 1, buildMs: 0 })),
    xlsx_hit_test: vi.fn(() => null),
    xlsx_hyperlink_at: vi.fn(() => null),
    xlsx_open: vi.fn(() => []),
    xlsx_images: vi.fn(() => []),
    xlsx_image_bytes: vi.fn(() => new Uint8Array()),
  };
});

vi.mock('@doc-converter/wasm', () => ({ default: wasm.init, ...wasm }));

/** Отметки заглушки DOCX-модуля: те же, что у `docx_lazy.spec.ts`. */
interface DocxStub {
  loads: number;
  initCalls: number;
  pages: { width: number; height: number }[];
  calls: Record<string, unknown[][]>;
}

/**
 * Глобалы заглушки. Список картинок и «битые» id тест задаёт сам: до функций
 * заглушки из теста не дотянуться — воркер грузит её по URL из `init`.
 */
interface DocxGlobals {
  __docxStub?: DocxStub;
  __docxImages?: ImageInfo[];
  __docxImagesBroken?: number[];
}

const docxGlobals = (): DocxGlobals => globalThis as unknown as DocxGlobals;

/** Вызовы функции заглушки: аргументы каждого вызова. */
const stubCalls = (name: string): unknown[][] => docxGlobals().__docxStub?.calls[name] ?? [];

/** URL заглушки — тот же путь, каким его увидит воркер: рантайм-строкой. */
const STUB_URL = new URL('./fixtures/docx-module.js', import.meta.url).href;

const IMAGES: ImageInfo[] = [
  { id: 3, mime: 'image/png', byteLength: 4 },
  { id: 7, mime: 'image/jpeg', byteLength: 2 },
];

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
  docx: { module: STUB_URL },
  slotCapacity: 1024,
});

const openMsg = (): InMsg => ({ type: 'open', format: 'docx', bytes: new ArrayBuffer(8) });

/** Дать осесть микрозадачам: в них воркер отвечает и резолвятся промисы. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

/** Поднять воркер заново: своя копия модуля и чистые отметки заглушки. */
async function startWorker(): Promise<void> {
  vi.resetModules();
  vi.clearAllMocks();
  // Модуль заглушки рантайм кэширует по URL: отметки заводим заново, чтобы
  // `loads` считал вычисления только этого сценария.
  const globals = docxGlobals();
  globals.__docxStub = undefined;
  globals.__docxImages = IMAGES;
  globals.__docxImagesBroken = [];

  posted = [];
  const selfStub: SelfStub = {
    onmessage: null,
    postMessage: (msg) => posted.push(msg),
  };
  vi.stubGlobal('self', selfStub);
  vi.stubGlobal('requestAnimationFrame', () => 0);
  // Пропущенный кадр воркер логирует — для теста это ожидаемый ход событий.
  vi.spyOn(console, 'error').mockImplementation(() => {});

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
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('картинки документа DOCX', () => {
  it('запрашивает и регистрирует битмапы с их id и mime', async () => {
    await startWorker();

    await handler({ data: openMsg() });

    expect(stubCalls('docx_images')).toHaveLength(1);
    expect(stubCalls('docx_image_bytes').map(([id]) => id)).toEqual([3, 7]);
    expect(wasm.register_bitmap).toHaveBeenCalledWith(
      3,
      expect.objectContaining({ id: 3, mime: 'image/png' }),
    );
    expect(wasm.register_bitmap).toHaveBeenCalledWith(
      7,
      expect.objectContaining({ id: 7, mime: 'image/jpeg' }),
    );
    expect(posted).toContainEqual({ type: 'docx-opened', pages: docxGlobals().__docxStub?.pages });
  });

  it('пропускает битую картинку и открывает документ с остальными', async () => {
    await startWorker();
    docxGlobals().__docxImagesBroken = [3];
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    await handler({ data: openMsg() });

    expect(wasm.register_bitmap).toHaveBeenCalledTimes(1);
    expect(wasm.register_bitmap).toHaveBeenCalledWith(7, expect.objectContaining({ id: 7 }));
    expect(posted).toContainEqual({ type: 'docx-opened', pages: docxGlobals().__docxStub?.pages });
    expect(posted.some((msg) => msg.type === 'error')).toBe(false);
    expect(warn).toHaveBeenCalled();
  });

  it('возвращает битмапы после ресайза', async () => {
    await startWorker();
    await handler({ data: openMsg() });
    const registered = wasm.register_bitmap.mock.calls.map((call) => [...call]);
    expect(registered).toHaveLength(2);

    wasm.register_bitmap.mockClear();

    await handler({ data: { type: 'resize', cssW: 800, cssH: 600, dpr: 2 } });

    expect(wasm.resize_canvas).toHaveBeenCalledTimes(1);
    expect(wasm.register_bitmap.mock.calls.map((call) => [...call])).toEqual(registered);
  });

  it('не регистрирует картинки документа, который закрыли во время загрузки', async () => {
    await startWorker();
    const resolvers: Array<() => void> = [];
    decode.mockImplementation(
      (blob: Blob) =>
        new Promise<ImageBitmap>((resolve) => {
          const bmp = { mime: blob.type, close: vi.fn() };
          resolvers.push(() => resolve(bmp as unknown as ImageBitmap));
        }),
    );

    const opened = handler({ data: openMsg() });
    await vi.waitFor(() => expect(resolvers).toHaveLength(2));

    await handler({ data: { type: 'close' } });
    for (const resolve of resolvers) resolve();
    await opened;
    await flush();

    expect(wasm.register_bitmap).not.toHaveBeenCalled();
    expect(posted.some((msg) => msg.type === 'docx-opened')).toBe(false);
  });

  it('снимает битмапы при закрытии документа', async () => {
    await startWorker();
    await handler({ data: openMsg() });

    await handler({ data: { type: 'close' } });

    expect(wasm.drop_bitmap).toHaveBeenCalledWith(3);
    expect(wasm.drop_bitmap).toHaveBeenCalledWith(7);
    expect(wasm.drop_bitmap).toHaveBeenCalledTimes(2);
  });
});
