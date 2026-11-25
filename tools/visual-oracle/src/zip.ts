// Минимальное чтение ZIP: валидатору manifest нужно достать `xl/workbook.xml`
// из XLSX, чтобы проверить существование и видимость листа. Полноценный
// ZIP-ридер здесь был бы лишним — нужен один entry по имени, без записи и
// без потоков.

import { inflateRawSync } from 'node:zlib';

const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const LOCAL_SIGNATURE = 0x04034b50;
const EOCD_MIN_SIZE = 22;
/** Максимальная длина комментария ZIP — столько байтов назад ищем EOCD. */
const MAX_COMMENT = 0xffff;

/**
 * Прочитать один entry ZIP-архива по имени.
 *
 * Возвращает `null`, если архив не разобран или entry отсутствует: для
 * валидатора это обычный отрицательный результат, а не исключение.
 */
export function readZipEntry(bytes: Uint8Array, name: string): Uint8Array | null {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const eocd = findEocd(view);
  if (eocd === null) return null;
  const entryCount = view.getUint16(eocd + 10, true);
  let offset = view.getUint32(eocd + 16, true);
  for (let index = 0; index < entryCount; index += 1) {
    if (offset + 46 > bytes.byteLength) return null;
    if (view.getUint32(offset, true) !== CENTRAL_SIGNATURE) return null;
    const method = view.getUint16(offset + 10, true);
    const compressedSize = view.getUint32(offset + 20, true);
    const nameLength = view.getUint16(offset + 28, true);
    const extraLength = view.getUint16(offset + 30, true);
    const commentLength = view.getUint16(offset + 32, true);
    const localOffset = view.getUint32(offset + 42, true);
    const entryName = decodeUtf8(bytes.subarray(offset + 46, offset + 46 + nameLength));
    if (entryName === name) {
      return readLocalEntry(bytes, view, localOffset, method, compressedSize);
    }
    offset += 46 + nameLength + extraLength + commentLength;
  }
  return null;
}

function findEocd(view: DataView): number | null {
  const total = view.byteLength;
  if (total < EOCD_MIN_SIZE) return null;
  const lowest = Math.max(0, total - EOCD_MIN_SIZE - MAX_COMMENT);
  for (let offset = total - EOCD_MIN_SIZE; offset >= lowest; offset -= 1) {
    if (view.getUint32(offset, true) === EOCD_SIGNATURE) return offset;
  }
  return null;
}

function readLocalEntry(
  bytes: Uint8Array,
  view: DataView,
  localOffset: number,
  method: number,
  compressedSize: number,
): Uint8Array | null {
  if (localOffset + 30 > bytes.byteLength) return null;
  if (view.getUint32(localOffset, true) !== LOCAL_SIGNATURE) return null;
  const nameLength = view.getUint16(localOffset + 26, true);
  const extraLength = view.getUint16(localOffset + 28, true);
  const start = localOffset + 30 + nameLength + extraLength;
  const end = start + compressedSize;
  if (end > bytes.byteLength) return null;
  const payload = bytes.subarray(start, end);
  if (method === 0) return payload;
  if (method === 8) {
    try {
      return new Uint8Array(inflateRawSync(payload));
    } catch {
      return null;
    }
  }
  return null;
}

function decodeUtf8(bytes: Uint8Array): string {
  return new TextDecoder('utf-8').decode(bytes);
}
