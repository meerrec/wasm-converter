import { describe, expect, it } from 'vitest';

import {
  HEADER_BYTES,
  HEADER_INTS,
  SLOT_COUNT,
} from '../src/sab/protocol.js';
import { SabReader } from './sab_reader.js';
import { FramePool } from '../src/worker/frame_pool.js';

/** Кадр-образец: у настоящего DisplayList тот же вид — заголовок и команды. */
function sampleFrame(length = 64): Uint8Array {
  const bytes = new Uint8Array(length);
  bytes.set([0x44, 0x4c, 0x53, 0x54]); // "DLST"
  for (let i = 4; i < length; i++) bytes[i] = i & 0xff;
  return bytes;
}

/**
 * Запись в ring — зеркало Rust `SabRing::try_write`: `false` означает, что
 * свободного слота не было и кадр пропущен (backpressure).
 */
function ringWrite(sab: SharedArrayBuffer, capacity: number, bytes: Uint8Array): boolean {
  const header = new Int32Array(sab);
  const seqWriter = Atomics.load(header, 0);
  const seqReader = Atomics.load(header, 1);
  if (seqWriter - seqReader >= SLOT_COUNT) return false;
  const slot = seqWriter & 1;
  new Uint8Array(sab, HEADER_BYTES + slot * capacity, capacity).set(bytes);
  Atomics.store(header, HEADER_INTS - 2 + slot, bytes.length);
  Atomics.store(header, 0, seqWriter + 1);
  return true;
}

describe('FramePool', () => {
  it('держит два буфера и переиспользует их, а не аллоцирует новые', () => {
    const pool = new FramePool(1024);
    const first = pool.beginBuild();
    pool.commitBuild(8);
    const second = pool.beginBuild();
    pool.commitBuild(8);
    expect(second.buffer).not.toBe(first.buffer);

    // Оба заняты готовыми кадрами: под третью сборку освобождается самый
    // старый — он устарел раньше всех.
    const third = pool.beginBuild();
    expect(third.buffer).toBe(first.buffer);
  });

  it('takeLatest отдаёт самый свежий кадр, остальные отпускает', () => {
    const pool = new FramePool(1024);
    const older = pool.beginBuild();
    older.set([1, 2, 3]);
    pool.commitBuild(3);
    const newer = pool.beginBuild();
    newer.set([9, 9]);
    pool.commitBuild(2);

    const frame = pool.takeLatest();
    expect(frame).not.toBeNull();
    expect(Array.from(frame ?? [])).toEqual([9, 9]);

    // Свежий кадр уже забрали, старый отброшен: отдавать больше нечего.
    expect(pool.takeLatest()).toBeNull();

    // Оба буфера снова свободны: подряд идущие сборки идут в разные.
    const one = pool.beginBuild();
    pool.commitBuild(1);
    const other = pool.beginBuild();
    expect(other.buffer).not.toBe(one.buffer);
  });

  it('abortBuild возвращает буфер в пул', () => {
    const pool = new FramePool(64);
    const first = pool.beginBuild();
    pool.abortBuild();
    expect(pool.takeLatest()).toBeNull();
    expect(pool.beginBuild().buffer).toBe(first.buffer);
  });

  it('grow даёт кадру буфер больше прежнего', () => {
    const pool = new FramePool(64);
    pool.grow(4096);
    expect(pool.slotCapacity).toBe(4096);
    expect(pool.beginBuild().byteLength).toBeGreaterThanOrEqual(4096);
  });

  it('кадр длиннее буфера не портит готовые кадры', () => {
    const pool = new FramePool(64);
    const frame = pool.beginBuild();
    frame.set([7, 7, 7]);
    pool.commitBuild(3);
    pool.grow(256);
    const next = pool.beginBuild();
    expect(next.byteLength).toBeGreaterThanOrEqual(256);
    expect(Array.from(pool.takeLatest() ?? []).length).toBeGreaterThan(0);
  });

  it('за сотню кадров не заводит больше двух буферов', () => {
    const pool = new FramePool(256);
    const buffers = new Set<ArrayBuffer>();
    for (let i = 0; i < 100; i++) {
      // Как в бою: сборщик кладёт кадр, painter забирает самый свежий.
      const buffer = pool.beginBuild();
      buffers.add(buffer.buffer);
      pool.commitBuild(4);
      if (i % 3 === 0) {
        const second = pool.beginBuild();
        buffers.add(second.buffer);
        pool.commitBuild(4);
      }
      pool.takeLatest();
    }
    expect(buffers.size).toBeLessThanOrEqual(2);
  });
});

describe('оба транспорта', () => {
  it('отдают painter одни и те же байты кадра', () => {
    const frame = sampleFrame();
    const capacity = 4096;
    const pool = new FramePool(capacity);
    const buffer = pool.beginBuild();
    buffer.set(frame);
    pool.commitBuild(frame.length);
    const viaBytes = pool.takeLatest();
    expect(viaBytes).not.toBeNull();

    const sab = new SharedArrayBuffer(HEADER_BYTES + capacity * SLOT_COUNT);
    expect(ringWrite(sab, capacity, frame)).toBe(true);
    const reader = new SabReader(sab, capacity);
    const viaSab = reader.peek();
    expect(viaSab).not.toBeNull();
    const sabBytes = Uint8Array.from(viaSab ?? []);
    reader.release();

    expect(Array.from(viaBytes ?? [])).toEqual(Array.from(sabBytes));
    expect(Array.from(sabBytes)).toEqual(Array.from(frame));
    expect(reader.hasData()).toBe(false);
  });

  it('держат глубину два: третий кадр ждёт освобождения слота', () => {
    const capacity = 64;
    const sab = new SharedArrayBuffer(HEADER_BYTES + capacity * SLOT_COUNT);
    expect(ringWrite(sab, capacity, sampleFrame(8))).toBe(true);
    expect(ringWrite(sab, capacity, sampleFrame(8))).toBe(true);
    expect(ringWrite(sab, capacity, sampleFrame(8))).toBe(false);

    const reader = new SabReader(sab, capacity);
    reader.release();
    expect(ringWrite(sab, capacity, sampleFrame(8))).toBe(true);

    const pool = new FramePool(capacity);
    const first = pool.beginBuild();
    first.set([1]);
    pool.commitBuild(1);
    const second = pool.beginBuild();
    second.set([2]);
    pool.commitBuild(1);
    // Пул той же глубины: третья сборка вытесняет самый старый кадр, а
    // painter'у достаётся свежий.
    pool.abortBuild();
    pool.beginBuild();
    pool.abortBuild();
    expect(Array.from(pool.takeLatest() ?? [])).toEqual([2]);
  });
});
