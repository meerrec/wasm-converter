import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Mock } from 'vitest';

import type { InMsg, OutMsg } from '../src/protocol.js';

/**
 * Main-режим воркера: сборка остаётся здесь, painter живёт на странице.
 *
 * Проверяется протокол, а не картинка: `init` без холста поднимает воркер в
 * режиме `main`, шрифты и painter не трогаются, кадры уезжают буферами с
 * переводом владения, `frame-ack` возвращает буфер в тот же слот, а третий
 * кадр при обоих занятых буферах не собирается — следующий помечается
 * `dropped`.
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

const IMAGES = [{ id: 3, mime: 'image/png', byteLength: 4 }];

const VIEWPORT = { x: 0, y: 0, w: 800, h: 600, scale: 1 };
const CONFIG = { showGrid: false, showHeaders: false, theme: 'light', dpr: 1 };

/** Ответы wasm-модуля: воркер получает их вместо настоящего `@meerrec/wasm`. */
const wasm = vi.hoisted(() => ({
  init: vi.fn(async () => {}),
  alloc_sab: vi.fn(() => ({})),
  sab_total_bytes: vi.fn(() => 4096),
  init_painter: vi.fn(),
  resize_canvas: vi.fn(),
  register_bitmap: vi.fn(),
  drop_bitmap: vi.fn(() => true),
  font_bytes: vi.fn(() => new Uint8Array(4)),
  paint_display_list_bytes: vi.fn(() => ({ cmds: 1, dropped: false, paintMs: 0 })),
  paint_display_list_sab: vi.fn(() => ({ cmds: 1, dropped: false, paintMs: 0 })),
  xlsx_build_display_list_into: vi.fn(),
  xlsx_build_display_list_sab: vi.fn(),
  xlsx_hit_test: vi.fn(() => null),
  xlsx_hyperlink_at: vi.fn(() => null),
  xlsx_open: vi.fn(() => []),
  xlsx_images: vi.fn(() => []),
  xlsx_image_bytes: vi.fn(() => new Uint8Array()),
}));

vi.mock('@meerrec/wasm', () => ({ default: wasm.init, ...wasm }));

type Handler = (ev: { data: InMsg }) => Promise<void>;

interface SelfStub {
  onmessage: Handler | null;
  postMessage: (msg: OutMsg, transfer?: Transferable[]) => void;
}

let handler: Handler;
/** Отправленное воркером: сообщение и transfer-список. */
let posted: Array<{ msg: OutMsg; transfer: Transferable[] }>;
let decode: Mock<(blob: Blob) => Promise<ImageBitmap>>;

/** `init` main-режима: холста нет, транспорт — буферы. */
const initMsg = (): InMsg => ({
  type: 'init',
  backend: 'main',
  slotCapacity: 1024,
});

const openMsg = (): InMsg => ({
  type: 'open',
  id: 1,
  format: 'xlsx',
  bytes: new ArrayBuffer(8),
});

const renderMsg = (): InMsg => ({
  type: 'render',
  req: { viewport: VIEWPORT, config: CONFIG },
});

/** Дать осесть микрозадачам: в них воркер собирает кадр и отвечает на open. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

/** Кадры в порядке отправки. */
const frames = (): Array<Extract<OutMsg, { type: 'frame' }>> =>
  posted.flatMap(({ msg }) => (msg.type === 'frame' ? [msg] : []));

const sent = (type: OutMsg['type']): Array<{ msg: OutMsg; transfer: Transferable[] }> =>
  posted.filter(({ msg }) => msg.type === type);

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  wasm.xlsx_open.mockReturnValue([SHEET]);
  wasm.xlsx_images.mockReturnValue(IMAGES);
  wasm.xlsx_image_bytes.mockImplementation((id: number) => new Uint8Array([id]));
  // Кадр «пишется» в буфер вызывающего: три байта и статистика сборки.
  wasm.xlsx_build_display_list_into.mockImplementation((out: Uint8Array) => {
    out.set([1, 2, 3]);
    return { written: true, cmds: 5, buildMs: 2, bytes: 3 };
  });

  posted = [];
  const selfStub: SelfStub = {
    onmessage: null,
    postMessage: (msg, transfer = []) => posted.push({ msg, transfer }),
  };
  vi.stubGlobal('self', selfStub);
  vi.stubGlobal('requestAnimationFrame', () => 0);

  decode = vi.fn(async () => ({ close: vi.fn() }) as unknown as ImageBitmap);
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

describe('init main-режима', () => {
  it('поднимает воркер без холста и не трогает painter со шрифтами', () => {
    const ready = sent('ready').map(({ msg }) => msg);
    expect(ready).toHaveLength(1);
    expect(ready[0]).toEqual({
      type: 'ready',
      backend: 'main',
      transport: 'bytes',
      slotCapacity: 1024,
      totalBytes: 2048,
      crossOriginIsolated: false,
    });

    // Painter и шрифты живут на main: воркер их не инициализирует.
    expect(wasm.init_painter).not.toHaveBeenCalled();
    expect(wasm.register_bitmap).not.toHaveBeenCalled();
    expect(wasm.resize_canvas).not.toHaveBeenCalled();
    expect(wasm.font_bytes).not.toHaveBeenCalled();
  });
});

describe('кадры main-режима', () => {
  beforeEach(async () => {
    await handler({ data: openMsg() });
  });

  it('отправляет кадр переводом владения буфером', async () => {
    await handler({ data: renderMsg() });
    await flush();

    const built = frames();
    expect(built).toHaveLength(1);
    const frame = built[0]!;
    expect(frame.frameId).toBe(1);
    // Пул отдаёт буферы с конца: [0, 1] → первый кадр едет из слота 1.
    expect(frame.slot).toBe(1);
    expect(frame.bytes).toBeInstanceOf(ArrayBuffer);
    expect(frame.len).toBe(3);
    expect(frame.cmds).toBe(5);
    expect(frame.buildMs).toBe(2);
    expect(frame.dropped).toBe(false);

    const message = sent('frame')[0]!;
    expect(message.transfer).toEqual([frame.bytes]);
  });

  it('frame-ack возвращает буфер в тот же слот, и он идёт в следующий кадр', async () => {
    await handler({ data: renderMsg() });
    await flush();
    const first = frames()[0]!;

    // Как и в настоящем postMessage, обратно приходит другой объект с той же
    // памятью: воркер узнаёт буфер по номеру слота, а не по идентичности.
    const returned = new ArrayBuffer(1024);
    await handler({ data: { type: 'frame-ack', slot: first.slot, buffer: returned } });

    await handler({ data: renderMsg() });
    await flush();

    const second = frames()[1]!;
    expect(second.slot).toBe(first.slot);
    expect(second.bytes).toBe(returned);
  });

  it('третий кадр при обоих занятых буферах пропускается и помечается dropped', async () => {
    // Два кадра уходят, ack'ов нет: оба буфера в полёте.
    await handler({ data: renderMsg() });
    await flush();
    await handler({ data: renderMsg() });
    await flush();
    expect(frames()).toHaveLength(2);

    // Третий запрос упирается в backpressure: сборки нет, пропуск запомнен.
    await handler({ data: renderMsg() });
    await flush();
    expect(frames()).toHaveLength(2);

    // Первый буфер вернулся — следующий кадр собирается и говорит о пропуске.
    const first = frames()[0]!;
    await handler({
      data: { type: 'frame-ack', slot: first.slot, buffer: new ArrayBuffer(1024) },
    });
    await flush();

    const built = frames();
    expect(built).toHaveLength(3);
    const third = built[2]!;
    expect(third.slot).toBe(first.slot);
    expect(third.dropped).toBe(true);
  });

  it('дубликат frame-ack не открывает третий буфер', async () => {
    await handler({ data: renderMsg() });
    await flush();
    const first = frames()[0]!;
    const returned = new ArrayBuffer(1024);

    await handler({ data: { type: 'frame-ack', slot: first.slot, buffer: returned } });
    await handler({ data: { type: 'frame-ack', slot: first.slot, buffer: returned } });

    // Оба слота свободны, но третий кадр берёт один из двух: пул по-прежнему
    // двухбуферный, а лишний ack ничего не сломал.
    await handler({ data: renderMsg() });
    await flush();
    await handler({ data: renderMsg() });
    await flush();
    expect(frames()).toHaveLength(3);
    expect(new Set(frames().map((frame) => frame.slot))).toEqual(new Set([0, 1]));
  });
});

describe('битмапы main-режима', () => {
  it('отправляет картинки на main до opened и не регистрирует их в воркере', async () => {
    await handler({ data: openMsg() });

    const order = posted.map(({ msg }) => msg.type);
    const bitmapAt = order.indexOf('bitmap');
    const openedAt = order.indexOf('opened');
    expect(bitmapAt).toBeGreaterThan(-1);
    expect(order.indexOf('clear-bitmaps')).toBeLessThan(bitmapAt);
    expect(bitmapAt).toBeLessThan(openedAt);

    const bitmap = sent('bitmap')[0]!;
    expect(bitmap.msg).toMatchObject({ type: 'bitmap', id: 3 });
    expect(bitmap.transfer).toHaveLength(1);

    // Воркер картинку не удерживает: painter на другой стороне.
    expect(wasm.register_bitmap).not.toHaveBeenCalled();
  });
});
