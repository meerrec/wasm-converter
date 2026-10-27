import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { InMsg, OutMsg, WorkerErrorMsg } from '../src/protocol.js';

/**
 * Потоковое открытие на общем ядре (ADR-0049): пока модуль формата не загружен,
 * чанки копятся в записи потока, а как только он готов — уезжают в wasm по
 * порядку и дальше идут поштучно. Проверяются обе формы хуков: синхронная
 * (основной модуль уже в памяти) и ленивая (`streamId` появляется после
 * `moduleReady`), — потому что именно от неё зависит гонка с загрузкой.
 */

/** Ответы основного wasm-модуля: настоящего `@meerrec/wasm` в тесте нет. */
const wasm = vi.hoisted(() => ({
  init: vi.fn(async () => {}),
  alloc_sab: vi.fn(() => ({})),
  sab_total_bytes: vi.fn(() => 4096),
  drop_bitmap: vi.fn(() => true),
  register_bitmap: vi.fn(),
}));

vi.mock('@meerrec/wasm', () => ({ default: wasm.init, ...wasm }));

interface SelfStub {
  onmessage: ((ev: { data: InMsg }) => Promise<void>) | null;
  postMessage: (msg: OutMsg, transfer?: Transferable[]) => void;
}

let posted: OutMsg[];

/** Отложенная загрузка модуля: тест сам решает, когда она завершится. */
interface Gate {
  promise: Promise<void>;
  open: () => void;
  fail: (e: Error) => void;
}

function gate(): Gate {
  // `Promise.withResolvers` появился в Node 22, а `engines` репозитория — «>=20»
  // и CI гоняет обе ноги: executor-форма здесь единственная переносимая.
  let open!: () => void;
  let fail!: (e: Error) => void;
  const promise = new Promise<void>((resolve, reject) => {
    open = resolve;
    fail = reject;
  });
  // Отказ разбирает само ядро (запись `failure`); здесь гасим повторно, чтобы
  // необработанный реджект не валил прогон.
  promise.catch(() => {});
  return { promise, open, fail };
}

/** Записанные в wasm байты: по ним видно и порядок, и поштучность. */
interface Recorded {
  streamId: number;
  bytes: number[];
}

interface Harness {
  core: { handle: (msg: InMsg) => Promise<boolean> };
  appends: Recorded[];
  freed: number[];
  finishes: number[];
  /** Ворота ленивой загрузки; `null` в синхронном сценарии. */
  gate: Gate | null;
}

/**
 * Поднять ядро с фиктивными хуками.
 *
 * `lazy` — та же форма, что у ленивых воркеров: `beginStream` выставляет
 * `moduleReady`, а `streamId` — только внутри его продолжения, поэтому до
 * загрузки запись потока живёт с нулём.
 */
async function startCore(lazy: boolean): Promise<Harness> {
  vi.resetModules();
  const selfStub: SelfStub = { onmessage: null, postMessage: (msg) => posted.push(msg) };
  vi.stubGlobal('self', selfStub);
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});

  const appends: Recorded[] = [];
  const freed: number[] = [];
  const finishes: number[] = [];
  const { WorkerCore } = await import('../src/worker/worker_core.js');

  const pendingGate = lazy ? gate() : null;
  const streamId = 7;

  const core = new WorkerCore({
    format: 'docx',
    openBlockedReason: () => null,
    open: async () => ({ type: 'docx-opened', id: 0, pages: [] }),
    beginStream: (_msg, entry) => {
      if (!pendingGate) {
        // Основной модуль уже в памяти: буфер потока выделяется сразу.
        entry.streamId = streamId;
        return;
      }
      entry.moduleReady = pendingGate.promise.then(() => {
        entry.streamId = streamId;
      });
    },
    appendStream: (id, bytes) => {
      appends.push({ streamId: id, bytes: Array.from(bytes) });
    },
    freeStream: (id) => {
      freed.push(id);
    },
    finishStream: (id) => {
      finishes.push(id);
      return { type: 'docx-opened', id: 1, pages: [] };
    },
    images: () => ({ tag: 'docx', list: () => [], bytes: () => new Uint8Array() }),
    buildInto: () => null,
    buildSab: () => null,
    release: () => {},
    cancel: () => {},
  });

  return { core, appends, freed, finishes, gate: pendingGate };
}

/** Ошибки, ушедшие на main: сужение по дискриминанту, без каста. */
const errors = (): WorkerErrorMsg[] =>
  posted.filter((m): m is WorkerErrorMsg => m.type === 'error');

/** Догон идёт продолжением `moduleReady`: двух микротактов хватает. */
const settle = async (): Promise<void> => {
  await Promise.resolve();
  await Promise.resolve();
};

const openStreamMsg = (id: number, totalSize?: number): InMsg =>
  totalSize === undefined
    ? { type: 'open-stream', id, format: 'docx' }
    : { type: 'open-stream', id, format: 'docx', totalSize };

const appendMsg = (id: number, ...values: number[]): InMsg => ({
  type: 'append',
  id,
  bytes: new Uint8Array(values).buffer,
});

beforeEach(() => {
  posted = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('поток в ядре: синхронный модуль', () => {
  it('каждый append сразу уезжает в wasm, в порядке прихода', async () => {
    const { core, appends } = await startCore(false);

    await core.handle(openStreamMsg(1));
    await core.handle(appendMsg(1, 1, 2));
    await core.handle(appendMsg(1, 3, 4));

    // Два append — два вызова wasm: ожидания end-stream нет.
    expect(appends.map((a) => a.bytes)).toEqual([[1, 2], [3, 4]]);
    expect(appends.every((a) => a.streamId === 7)).toBe(true);
  });

  it('путь с известным размером досылает только новый кусок', async () => {
    const { core, appends } = await startCore(false);

    await core.handle(openStreamMsg(1, 4));
    await core.handle(appendMsg(1, 9, 9));
    await core.handle(appendMsg(1, 8, 8));

    // target пишется по смещению, в wasm уходит ровно приращение.
    expect(appends.map((a) => a.bytes)).toEqual([[9, 9], [8, 8]]);
  });

  it('превышение totalSize отклоняется до любого append в wasm', async () => {
    const { core, appends, freed } = await startCore(false);

    await core.handle(openStreamMsg(1, 2));
    await core.handle(appendMsg(1, 1, 2, 3));

    expect(appends).toEqual([]);
    expect(freed).toEqual([7]);
    expect(errors()).toContainEqual({
      type: 'error',
      message: 'stream exceeds declared size',
      id: 1,
      code: 'open',
    });
  });
});

describe('поток в ядре: ленивый модуль', () => {
  it('до загрузки модуля в wasm не уходит ни байта, после — догон по порядку', async () => {
    const { core, appends, gate: pendingGate } = await startCore(true);

    await core.handle(openStreamMsg(1));
    await core.handle(appendMsg(1, 1, 2));
    await core.handle(appendMsg(1, 3, 4));
    // Модуль ещё грузится: чанки ждут в записи потока.
    expect(appends).toEqual([]);

    pendingGate!.open();
    await settle();

    expect(appends.map((a) => a.bytes)).toEqual([[1, 2], [3, 4]]);
  });

  it('append после загрузки уходит сразу, без ожидания end-stream', async () => {
    const { core, appends, gate: pendingGate } = await startCore(true);

    await core.handle(openStreamMsg(1));
    pendingGate!.open();
    await settle();

    await core.handle(appendMsg(1, 5, 6));

    expect(appends.map((a) => a.bytes)).toEqual([[5, 6]]);
  });

  it('отказ загрузки модуля: end-stream отвечает ошибкой открытия ровно раз', async () => {
    const { core, appends, freed, finishes, gate: pendingGate } = await startCore(true);

    await core.handle(openStreamMsg(1));
    await core.handle(appendMsg(1, 1, 2));
    pendingGate!.fail(new Error('module load failed'));
    await settle();

    await core.handle({ type: 'end-stream', id: 1 });

    expect(appends).toEqual([]);
    expect(finishes).toEqual([]);
    const seen = errors();
    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatchObject({ id: 1, code: 'open' });
    expect(seen[0]?.message).toContain('module load failed');
    // Буфер в wasm не выделялся — освобождать нечего.
    expect(freed).toEqual([]);
  });

  it('закрытие во время загрузки освобождает поток ровно один раз', async () => {
    const { core, freed, gate: pendingGate } = await startCore(true);

    await core.handle(openStreamMsg(1));
    pendingGate!.open();
    await settle();
    expect(freed).toEqual([]);

    await core.handle({ type: 'close' });
    expect(freed).toEqual([7]);

    // end-stream после закрытия уже не находит запись: второго free нет.
    await core.handle({ type: 'end-stream', id: 1 });
    expect(freed).toEqual([7]);
  });

  it('оборванный поток освобождает буфер и отвечает ошибкой', async () => {
    const { core, freed, appends, gate: pendingGate } = await startCore(true);

    await core.handle(openStreamMsg(1));
    await core.handle(appendMsg(1, 1, 2));
    pendingGate!.open();
    await settle();

    await core.handle({ type: 'end-stream', id: 1, aborted: true });

    expect(freed).toEqual([7]);
    // Чанк уехал в wasm ещё до обрыва (модуль успел загрузиться): буфер
    // освобождается, но данных это не отменяет.
    expect(appends.map((a) => a.bytes)).toEqual([[1, 2]]);
    expect(errors()).toContainEqual({
      type: 'error',
      message: 'stream aborted',
      id: 1,
      code: 'open',
    });
  });
});
