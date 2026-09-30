import { HEADER_BYTES, canRead, readHeader, readerSlot } from './protocol';

/** Симметричен Rust `SabRing` — для дебага и тестов. */
export class SabReader {
  private readonly i32: Int32Array;
  private readonly slot0: Uint8Array;
  private readonly slot1: Uint8Array;

  constructor(sab: SharedArrayBuffer, slotCapacity: number) {
    this.i32 = new Int32Array(sab);
    this.slot0 = new Uint8Array(sab, HEADER_BYTES, slotCapacity);
    this.slot1 = new Uint8Array(sab, HEADER_BYTES + slotCapacity, slotCapacity);
  }

  hasData(): boolean {
    return canRead(readHeader(this.i32));
  }

  /** Возвращает view без копирования. Вызывающий должен вызвать release(). */
  peek(): Uint8Array | null {
    const h = readHeader(this.i32);
    if (!canRead(h)) return null;
    const s = readerSlot(h);
    const len = h.slotLen[s];
    const buf = s === 0 ? this.slot0 : this.slot1;
    return buf.subarray(0, len);
  }

  release(): void {
    const h = readHeader(this.i32);
    if (!canRead(h)) return;
    Atomics.store(this.i32, 1, (h.seqReader + 1) | 0);
  }
}
