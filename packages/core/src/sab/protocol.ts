/** Заголовок ring'а. Смещения совпадают с Rust. */
export const HEADER_INTS = 4;
export const HEADER_BYTES = HEADER_INTS * 4;
export const SLOT_COUNT = 2;

export interface SlotHeader {
  seqWriter: number;
  seqReader: number;
  slotLen: [number, number];
}

export function readHeader(i32: Int32Array): SlotHeader {
  return {
    seqWriter: Atomics.load(i32, 0),
    seqReader: Atomics.load(i32, 1),
    slotLen: [Atomics.load(i32, 2), Atomics.load(i32, 3)],
  };
}

export function hasFreeSlot(h: SlotHeader): boolean {
  return h.seqWriter - h.seqReader < SLOT_COUNT;
}

export function canRead(h: SlotHeader): boolean {
  return h.seqReader < h.seqWriter;
}

export function writerSlot(h: SlotHeader): 0 | 1 {
  return (h.seqWriter & 1) as 0 | 1;
}

export function readerSlot(h: SlotHeader): 0 | 1 {
  return (h.seqReader & 1) as 0 | 1;
}
