import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { BuildStatsWire, PaintStatsWire } from '../src/generated/wire.js';
import type { RenderRequest } from '../src/protocol.js';
import { ArrayBufferTransport } from '../src/execution/transport_array_buffer.js';
import { SharedArrayBufferTransport } from '../src/execution/transport_sab.js';
import type { TransportDeps } from '../src/execution/transport.js';
import type { RelayBuildOutcome } from '../src/worker/relay_loop.js';

/**
 * Транспорты кадра без браузера: проверяется контракт `TransportAdapter`, а не
 * картинка. Painter подменён моками `wasm_typed`, поэтому тест не тянет wasm и
 * работает в node.
 */

/** Моки painter'а: `vi.hoisted` нужен, потому что `vi.mock` поднимается выше. */
const painted = vi.hoisted(() => ({
  paintSab: vi.fn<(sab: SharedArrayBuffer, slotCapacity: number) => PaintStatsWire>(() => ({
    cmds: 2,
    dropped: false,
    paintMs: 1,
  })),
  paintBytes: vi.fn<(frame: Uint8Array) => PaintStatsWire>(() => ({
    cmds: 3,
    dropped: false,
    paintMs: 2,
  })),
}));

vi.mock('../src/worker/wasm_typed.js', () => ({
  paintSab: painted.paintSab,
  paintBytes: painted.paintBytes,
}));

const CAPACITY = 64;

const REQ: RenderRequest = {
  viewport: { x: 0, y: 0, w: 800, h: 600, scale: 1 },
  config: { showGrid: false, showHeaders: false, theme: 'light', dpr: 1 },
};

/** Статистика сборки: по умолчанию кадр уложился в буфер. */
function buildStats(overrides: Partial<BuildStatsWire> = {}): BuildStatsWire {
  return { written: true, cmds: 1, bytes: 32, buildMs: 0.25, ...overrides };
}

/** Зависимости SAB-транспорта: сборка в слот и выделение ring'а — подменяемые. */
function sabDeps(
  buildSab: TransportDeps['buildSab'],
  allocSab: TransportDeps['allocSab'] = () => null,
): TransportDeps {
  return {
    slotCapacity: CAPACITY,
    target: 'worker',
    sab: null,
    allocSab,
    buildInto: () => null,
    buildSab,
  };
}

/** Зависимости буферного транспорта: роль задаёт `target`. */
function bytesDeps(
  target: TransportDeps['target'],
  buildInto: TransportDeps['buildInto'],
): TransportDeps {
  return {
    slotCapacity: CAPACITY,
    target,
    sab: null,
    allocSab: () => null,
    buildInto,
    buildSab: () => null,
  };
}

/** Кадр из исхода отправки; иначе — падение с внятным видом исхода. */
function frameOf(outcome: RelayBuildOutcome): Extract<RelayBuildOutcome, { kind: 'frame' }>['frame'] {
  if (outcome.kind !== 'frame') throw new Error(`ожидался кадр, получен ${outcome.kind}`);
  return outcome.frame;
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('SharedArrayBufferTransport', () => {
  it('отдаёт built, когда кадр уложился в слот', () => {
    const stats = buildStats({ written: true, bytes: 16, cmds: 5 });
    const buildSab = vi.fn<TransportDeps['buildSab']>(() => stats);
    const sab = new SharedArrayBuffer(CAPACITY);
    const transport = new SharedArrayBufferTransport(sab, sabDeps(buildSab));

    expect(transport.kind).toBe('sab');
    expect(transport.slotCapacity).toBe(CAPACITY);
    expect(transport.buildLocal(REQ)).toEqual({ outcome: 'built', stats });
    expect(buildSab).toHaveBeenCalledWith(sab, CAPACITY, REQ);
  });

  it('пропускает не влезший кадр и растит ring', () => {
    const grown = new SharedArrayBuffer(4096);
    const allocSab = vi.fn<TransportDeps['allocSab']>(() => grown);
    const buildSab = vi.fn<TransportDeps['buildSab']>(() =>
      buildStats({ written: false, bytes: 4096, cmds: 0 }),
    );
    const old = new SharedArrayBuffer(CAPACITY);
    const transport = new SharedArrayBufferTransport(old, sabDeps(buildSab, allocSab));

    expect(transport.buildLocal(REQ).outcome).toBe('dropped');
    // Кадр пропущен, но ring уже вырос: следующий кадр такой длины влезет.
    expect(transport.slotCapacity).toBe(4096);
    expect(allocSab).toHaveBeenCalledWith(4096);
    expect(transport.sab).toBe(grown);
  });

  it('оставляет прежний ring, если память под рост не нашлась', () => {
    const old = new SharedArrayBuffer(CAPACITY);
    const buildSab = vi.fn<TransportDeps['buildSab']>(() =>
      buildStats({ written: false, bytes: 4096 }),
    );
    const transport = new SharedArrayBufferTransport(old, sabDeps(buildSab));

    expect(transport.buildLocal(REQ).outcome).toBe('dropped');
    // Пропуск кадра дешевле падения: следующий кадр может оказаться меньше.
    // Ёмкость не растёт вместе с неудавшейся аллокацией: painter читает ring
    // по фактическому размеру, и «обещанный» больший слот сломал бы разметку.
    expect(transport.sab).toBe(old);
    expect(transport.slotCapacity).toBe(CAPACITY);
  });

  it('не собирает кадр, когда собирать нечего', () => {
    const transport = new SharedArrayBufferTransport(
      new SharedArrayBuffer(CAPACITY),
      sabDeps(() => null),
    );

    expect(transport.buildLocal(REQ)).toEqual({ outcome: 'idle', stats: null });
    expect(transport.slotCapacity).toBe(CAPACITY);
  });

  it('рисует ring воркерным painter\'ом', () => {
    const sab = new SharedArrayBuffer(CAPACITY);
    const transport = new SharedArrayBufferTransport(sab, sabDeps(() => null));

    expect(transport.paintLocal()).toEqual({ cmds: 2, dropped: false, paintMs: 1 });
    expect(painted.paintSab).toHaveBeenCalledWith(sab, CAPACITY);
  });

  it('не отправляет кадры на main и не ждёт подтверждений', () => {
    const transport = new SharedArrayBufferTransport(
      new SharedArrayBuffer(CAPACITY),
      sabDeps(() => null),
    );

    expect(() => transport.buildForward()).toThrow(/не отправляет кадры на main/);
    expect(() => transport.ack(0, new ArrayBuffer(8))).not.toThrow();
    expect(() => transport.close()).not.toThrow();
  });
});

describe("ArrayBufferTransport target: 'worker'", () => {
  it('пишет кадр в пул и отдаёт его painter\'у', () => {
    const buildInto = vi.fn<TransportDeps['buildInto']>((out) => {
      out.set([1, 2, 3]);
      return buildStats({ written: true, bytes: 3, cmds: 7 });
    });
    const transport = new ArrayBufferTransport(bytesDeps('worker', buildInto));

    expect(transport.buildLocal(REQ).outcome).toBe('built');

    // Сборщик получил буфер пула: painter читает ту же память без копии.
    expect(buildInto.mock.calls[0]?.[0].byteLength).toBe(CAPACITY);

    expect(transport.paintLocal()).toEqual({ cmds: 3, dropped: false, paintMs: 2 });
    expect(painted.paintBytes).toHaveBeenCalledTimes(1);
    const frame = painted.paintBytes.mock.calls[0]?.[0];
    expect(Array.from(frame ?? [])).toEqual([1, 2, 3]);
  });

  it('без готового кадра сообщает о пропуске, не зовя painter', () => {
    const transport = new ArrayBufferTransport(bytesDeps('worker', () => null));

    expect(transport.paintLocal()).toEqual({ cmds: 0, dropped: true, paintMs: 0 });
    expect(painted.paintBytes).not.toHaveBeenCalled();
  });

  it('пропускает не влезший кадр и растит пул', () => {
    const transport = new ArrayBufferTransport(
      bytesDeps('worker', () => buildStats({ written: false, bytes: 4096 })),
    );

    expect(transport.buildLocal(REQ).outcome).toBe('dropped');
    expect(transport.slotCapacity).toBe(4096);
  });

  it('grow увеличивает буфер пула', () => {
    const transport = new ArrayBufferTransport(bytesDeps('worker', () => null));

    expect(transport.slotCapacity).toBe(CAPACITY);
    transport.grow(4096);
    expect(transport.slotCapacity).toBe(4096);
  });
});

describe("ArrayBufferTransport target: 'main'", () => {
  it('отдаёт кадр с байтами релея и статистикой сборки', () => {
    const buildInto = vi.fn<TransportDeps['buildInto']>((out) => {
      out.set([4, 5, 6]);
      return buildStats({ written: true, bytes: 3, cmds: 9, buildMs: 1.5 });
    });
    const transport = new ArrayBufferTransport(bytesDeps('main', buildInto));

    const frame = frameOf(transport.buildForward(REQ));

    expect(frame.len).toBe(3);
    expect(frame.cmds).toBe(9);
    expect(frame.buildMs).toBe(1.5);
    expect(Array.from(new Uint8Array(frame.bytes, 0, frame.len))).toEqual([4, 5, 6]);
  });

  it('держит два буфера в полёте, а ack возвращает буфер в тот же слот', () => {
    const buildInto = vi.fn<TransportDeps['buildInto']>((out) => {
      out.set([7]);
      return buildStats({ written: true, bytes: 1 });
    });
    const transport = new ArrayBufferTransport(bytesDeps('main', buildInto));

    const first = frameOf(transport.buildForward(REQ));
    const second = frameOf(transport.buildForward(REQ));
    expect(first.slot).not.toBe(second.slot);

    // Оба буфера в полёте — третий кадр не ждёт, а пропускается.
    expect(transport.buildForward(REQ)).toEqual({ kind: 'busy' });

    transport.ack(first.slot, first.bytes);
    expect(frameOf(transport.buildForward(REQ)).slot).toBe(first.slot);
  });

  it('просит повторить сборку, если кадр не влез и релей вырос', () => {
    let tooBig = true;
    const buildInto = vi.fn<TransportDeps['buildInto']>((out) => {
      if (tooBig) return buildStats({ written: false, bytes: 4096, cmds: 0 });
      out.set([1, 2]);
      return buildStats({ written: true, bytes: 2 });
    });
    const transport = new ArrayBufferTransport(bytesDeps('main', buildInto));

    expect(transport.buildForward(REQ)).toEqual({ kind: 'retry' });
    expect(transport.slotCapacity).toBe(4096);

    tooBig = false;
    expect(frameOf(transport.buildForward(REQ)).len).toBe(2);
  });

  it('сообщает busy, если кадр не собрался, но буфер уже достаточен', () => {
    const buildInto = vi.fn<TransportDeps['buildInto']>(() =>
      buildStats({ written: false, bytes: CAPACITY }),
    );
    const transport = new ArrayBufferTransport(bytesDeps('main', buildInto));

    expect(transport.buildForward(REQ)).toEqual({ kind: 'busy' });

    // Буфер вернулся в пул: следующий кадр не ждёт ack.
    buildInto.mockImplementation((out) => {
      out.set([3]);
      return buildStats({ written: true, bytes: 1 });
    });
    expect(frameOf(transport.buildForward(REQ)).len).toBe(1);
  });

  it('не собирает кадр, когда собирать нечего', () => {
    const transport = new ArrayBufferTransport(bytesDeps('main', () => null));

    expect(transport.buildForward(REQ)).toEqual({ kind: 'idle' });
  });
});

describe('чужой путь', () => {
  it('main-транспорт не собирает и не рисует локально', () => {
    const transport = new ArrayBufferTransport(bytesDeps('main', () => null));

    expect(() => transport.buildLocal(REQ)).toThrow(/локальная сборка/);
    expect(() => transport.paintLocal()).toThrow(/локальная отрисовка/);
  });

  it('воркерный транспорт не отправляет кадры на main', () => {
    const transport = new ArrayBufferTransport(bytesDeps('worker', () => null));

    expect(() => transport.buildForward(REQ)).toThrow(/отправка кадра/);
  });

  it('ack и close на воркерном пути безопасны', () => {
    const transport = new ArrayBufferTransport(bytesDeps('worker', () => null));

    expect(() => transport.ack(0, new ArrayBuffer(8))).not.toThrow();
    expect(() => transport.close()).not.toThrow();
  });
});
