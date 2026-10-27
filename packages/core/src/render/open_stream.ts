// Прокачка `ReadableStream` в воркер чанками (ADR-0031).
//
// Один и тот же цикл у обоих просмотрщиков: `open-stream` → `append` ×N →
// `end-stream`. Чанк, владеющий своим `ArrayBuffer` целиком, уезжает переводом
// владения — копии на main нет. Копируется только срез вью поверх большего
// буфера (transfer унёс бы буфер целиком): это единственный случай, когда на
// main остаётся копия, и по размеру она равна чанку, а не документу.

import type { DocFormat, InMsg } from '../protocol.js';

/** Размер чанка: крупные куски без лишних сообщений, прогресс остаётся плавным. */
export const STREAM_CHUNK_BYTES = 1024 * 1024;

/** Параметры стримовой прокачки. */
export interface PumpOptions {
  /** Полный размер входа, если известен: его носит `onProgress`. */
  size?: number;
  /** Прогресс: вызывается на каждый отправленный чанк. */
  onProgress?: (loaded: number, total?: number) => void;
}

/**
 * Отправить поток в воркер чанками.
 *
 * Оборванный читателем поток (отмена или ошибка чтения) помечается
 * `aborted: true`: воркер отвечает ошибкой открытия «stream aborted», а
 * здесь исключение поднимается вызывающему — промис открытия реджектится
 * ошибкой воркера через обычный механизм `WorkerErrorMsg`.
 *
 * # Errors
 * Если чтение потока упало — сеть, отмена читателя, умерший воркер.
 */
export async function pumpStream(
  post: (msg: InMsg, transfer?: Transferable[]) => void,
  id: number,
  format: DocFormat,
  source: ReadableStream<Uint8Array>,
  options: PumpOptions = {},
): Promise<void> {
  // `exactOptionalPropertyTypes`: необязательные поля не принимают `undefined`
  // явно, поэтому сообщение собирается условно.
  const openMsg: InMsg =
    options.size === undefined
      ? { type: 'open-stream', id, format }
      : { type: 'open-stream', id, format, totalSize: options.size };
  post(openMsg);
  const reader = source.getReader();
  let loaded = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      for (let off = 0; off < value.byteLength; off += STREAM_CHUNK_BYTES) {
        const end = Math.min(off + STREAM_CHUNK_BYTES, value.byteLength);
        const buffer = value.buffer;
        if (
          off === 0 &&
          end === value.byteLength &&
          value.byteOffset === 0 &&
          buffer instanceof ArrayBuffer &&
          // Буфер обязан быть ровно этого размера: вью короче своего буфера
          // отправила бы transfer'ом лишний хвост.
          buffer.byteLength === value.byteLength
        ) {
          // Чанк владеет буфером целиком — так отдают `fetch().body` и
          // `File.stream()`. Переводим владение: копии на main нет вовсе.
          post({ type: 'append', id, bytes: buffer }, [buffer]);
        } else {
          // Вью поверх большего буфера или чанк, разрезанный на несколько
          // частей: transfer невозможен (уехал бы весь буфер), поэтому копируем
          // ровно свой срез.
          const copy = new Uint8Array(end - off);
          copy.set(value.subarray(off, end));
          post({ type: 'append', id, bytes: copy.buffer }, [copy.buffer]);
        }
        loaded += end - off;
        options.onProgress?.(loaded, options.size);
      }
    }
    post({ type: 'end-stream', id });
  } catch (e) {
    try {
      post({ type: 'end-stream', id, aborted: true });
    } catch {
      // Воркер уже не принимает сообщений — отвергнуть должен вызывающий.
    }
    throw e;
  }
}
