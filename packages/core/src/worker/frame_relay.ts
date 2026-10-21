/**
 * Два буфера кадра для main-режима.
 *
 * Роли те же, что у `FramePool`, но владение буфером уходит на другой поток:
 * воркер заполняет буфер, отправляет его main'у и ждёт `frame-ack`, прежде чем
 * буфер снова станет свободным. Глубина — два кадра: пока main рисует один,
 * воркер успевает собрать второй. Третий запрос, когда оба буфера в полёте,
 * отбрасывается (`beginBuild` возвращает `null`) — догонять прокрутку
 * устаревшими кадрами незачем.
 *
 * Выросший буфер заменяет маленькие: `beginBuild` пересоздаёт буфер, если он
 * меньше текущей ёмкости, иначе кадр никогда бы не влез после `grow`.
 */
export class FrameRelay {
  /** Буферы кадра: их ровно два и они не пересоздаются без нужды. */
  private buffers: ArrayBuffer[];
  /** Индексы свободных буферов; остальные — в сборке, готовые или в полёте. */
  private free: number[] = [0, 1];
  /** Буфер текущей сборки; `null`, когда сборки нет. */
  private building: number | null = null;
  /** Собранный, но ещё не отправленный кадр. */
  private ready: { slot: number; len: number } | null = null;
  /**
   * Слоты, чьи буферы уехали на main и ждут `frame-ack`. Идентичность самого
   * `ArrayBuffer` через `postMessage` не сохраняется — на другой стороне это
   * новый объект, — поэтому слот возвращается в ack'е номером.
   */
  private inFlight = new Set<number>();
  private capacity: number;

  constructor(capacity: number) {
    this.capacity = capacity;
    this.buffers = [new ArrayBuffer(capacity), new ArrayBuffer(capacity)];
  }

  /** Размер буфера: кадр длиннее него в буфер не влезет. */
  get slotCapacity(): number {
    return this.capacity;
  }

  /**
   * Свободный буфер под следующую сборку либо `null`, если оба буфера в
   * полёте. Это и есть backpressure main-пути: кадр пропускается, а не ждёт.
   */
  beginBuild(): Uint8Array | null {
    if (this.building !== null) {
      throw new Error('frame build already in progress');
    }
    const slot = this.free.pop();
    if (slot === undefined) return null;
    let buffer = this.buffers[slot];
    if (!buffer || buffer.byteLength < this.capacity) {
      buffer = new ArrayBuffer(this.capacity);
      this.buffers[slot] = buffer;
    }
    this.building = slot;
    return new Uint8Array(buffer);
  }

  /** Кадр собран: `len` байт готовы к отправке. */
  commit(len: number): void {
    if (this.building === null) return;
    this.ready = { slot: this.building, len };
    this.building = null;
  }

  /** Кадр не собрался — буфер возвращается в пул. */
  abort(): void {
    if (this.building === null) return;
    this.free.push(this.building);
    this.building = null;
  }

  /**
   * Забрать готовый кадр: буфер покидает пул и возвращается только через
   * `adopt` после `frame-ack`.
   */
  takeFrame(): { bytes: ArrayBuffer; len: number; slot: number } | null {
    const ready = this.ready;
    this.ready = null;
    if (!ready) return null;
    const buffer = this.buffers[ready.slot];
    if (!buffer) return null;
    this.inFlight.add(ready.slot);
    return { bytes: buffer, len: ready.len, slot: ready.slot };
  }

  /**
   * Вернуть буфер, пришедший в `frame-ack`, в слот `slot`. Ack на слот,
   * который не в полёте (повторный, чужой или уже свободный), игнорируется:
   * иначе один дубль открыл бы третий буфер.
   */
  adopt(slot: number, buffer: ArrayBuffer): void {
    if (!this.inFlight.has(slot)) return;
    this.inFlight.delete(slot);
    // Буфер на этой стороне — новый объект с той же памятью за ним: кладём
    // его в слот, чтобы следующая сборка писала в возвращённую память.
    this.buffers[slot] = buffer;
    this.free.push(slot);
  }

  /** Вырасти под кадр длиной `bytes`: буферы меньше него больше не годятся. */
  grow(bytes: number): void {
    const capacity = Math.max(bytes, this.capacity * 2);
    if (capacity <= this.capacity) return;
    this.capacity = capacity;
    // Буферы в полёте не трогаем: их вернёт main, а заменятся они при
    // следующем `beginBuild`.
    for (const slot of this.free) this.buffers[slot] = new ArrayBuffer(capacity);
  }
}
