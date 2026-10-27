// Двойной буфер кадров для пути без SAB.
//
// Роли те же, что у слотов ring'а: сборщик пишет в свободный буфер, painter
// читает самый свежий готовый, промежуточные кадры отбрасываются — догонять
// прокрутку устаревшими кадрами незачем. Буферы одни и те же на все кадры,
// поэтому на кадр не приходится ни аллокации, ни работы сборщика мусора:
// буфер, отданный painter'у, тут же годится под следующую сборку.
//
// Кадр живёт в `Uint8Array` (его заполняет `*_build_display_list_into`), а
// владение буфером переходит от сборки к отрисовке и обратно — на main кадры
// не уезжают вовсе, painter живёт в том же воркере.
export class FramePool {
  /** Буферы кадра: их ровно два и они не пересоздаются без нужды. */
  private buffers: Uint8Array[];
  /** Готовые кадры, старые первыми. */
  private ready: { slot: number; len: number }[] = [];
  /** Буфер текущей сборки; `null`, когда сборки нет. */
  private building: number | null = null;
  private capacity: number;

  constructor(capacity: number) {
    this.capacity = capacity;
    this.buffers = [new Uint8Array(capacity), new Uint8Array(capacity)];
  }

  /** Размер буфера: кадр длиннее него в буфер не влезет. */
  get slotCapacity(): number {
    return this.capacity;
  }

  /**
   * Буфер под следующую сборку. Оба буфера заняты готовыми кадрами — самый
   * старый отбрасывается: он устарел раньше того, что в работе.
   */
  beginBuild(): Uint8Array {
    let slot = this.buffers.findIndex(
      (_, index) => !this.ready.some((frame) => frame.slot === index),
    );
    if (slot < 0) {
      slot = this.ready.shift()?.slot ?? 0;
    }
    this.building = slot;
    const buffer = this.buffers[slot];
    if (buffer && buffer.byteLength >= this.capacity) {
      return buffer;
    }
    // Буфер выращен после `grow`: маленький не переиспользуем.
    const grown = new Uint8Array(this.capacity);
    this.buffers[slot] = grown;
    return grown;
  }

  /** Кадр собран: `len` байт в буфере готовы к отрисовке. */
  commitBuild(len: number): void {
    if (this.building === null) return;
    this.ready.push({ slot: this.building, len });
    this.building = null;
  }

  /** Кадр собрать не удалось (не влез в буфер) — буфер возвращается в пул. */
  abortBuild(): void {
    this.building = null;
  }

  /**
   * Самый свежий готовый кадр; остальные отбрасываются. Возвращает вид на
   * буфер — painter только читает его и не удерживает.
   */
  takeLatest(): Uint8Array | null {
    const latest = this.ready.pop();
    this.ready.length = 0;
    if (!latest) return null;
    const buffer = this.buffers[latest.slot];
    return buffer ? buffer.subarray(0, latest.len) : null;
  }

  /** Вырасти под кадр длиной `bytes`: буферы меньше него больше не годятся. */
  grow(bytes: number): void {
    this.capacity = Math.max(bytes, this.capacity * 2);
  }
}
