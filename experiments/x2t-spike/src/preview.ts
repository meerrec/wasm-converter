/**
 * Разбор результата для предпросмотра на странице. Просмотрщик здесь не нужен — достаточно
 * убедиться, что выход похож на то, чем его объявили: пакет, текст или сырые байты.
 */

/** Запись zip: имени и размера хватает, чтобы отличить пакет от мусора. */
export interface ZipEntry {
  readonly name: string;
  readonly size: number;
}

const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_HEADER_SIGNATURE = 0x02014b50;
/** EOCD — последняя структура файла, но за ней может лежать комментарий до 64 КиБ. */
const MAX_COMMENT = 0xffff;
const EOCD_MIN_SIZE = 22;

/**
 * Перечисляет записи zip по центральному каталогу. Распаковывать нечего: выходы конвертера —
 * OOXML/ODF-пакеты, и проверка «zip да zip, с `mimetype`/`[Content_Types].xml`» по каталогу
 * достаточна. Возвращает `null`, если сигнатуры не сошлись, — тогда вызывающий показывает
 * байты как есть.
 */
export function listZipEntries(bytes: Uint8Array): ZipEntry[] | null {
  if (bytes.byteLength < EOCD_MIN_SIZE) {
    return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  let eocd = -1;
  const lastStart = bytes.byteLength - EOCD_MIN_SIZE;
  for (let i = lastStart; i >= Math.max(0, lastStart - MAX_COMMENT); i--) {
    if (view.getUint32(i, true) === EOCD_SIGNATURE) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) {
    return null;
  }

  const count = view.getUint16(eocd + 10, true);
  let offset = view.getUint32(eocd + 16, true);
  // Имена в OOXML/ODF пишутся в UTF-8; флаг 0x800 проверять не будем — он у наших выходов
  // выставлен, а на «не выставлен» UTF-8 всё равно даёт читаемую строку для ASCII-имён.
  const decoder = new TextDecoder('utf-8');
  const entries: ZipEntry[] = [];
  for (let i = 0; i < count; i++) {
    if (offset + 46 > bytes.byteLength || view.getUint32(offset, true) !== CENTRAL_HEADER_SIGNATURE) {
      return null;
    }
    const size = view.getUint32(offset + 24, true);
    const nameLength = view.getUint16(offset + 28, true);
    const extraLength = view.getUint16(offset + 30, true);
    const commentLength = view.getUint16(offset + 32, true);
    const nameStart = offset + 46;
    entries.push({
      name: decoder.decode(bytes.subarray(nameStart, nameStart + nameLength)),
      size,
    });
    offset = nameStart + nameLength + extraLength + commentLength;
  }
  return entries;
}

/** Первые `limit` байт как текст: x2t пишет txt/csv в UTF-8 (кодовая страница 46 в params). */
export function decodeTextPreview(bytes: Uint8Array, limit = 4000): string {
  const slice = bytes.subarray(0, Math.min(bytes.byteLength, limit));
  const text = new TextDecoder('utf-8', { fatal: false }).decode(slice);
  return bytes.byteLength > limit ? `${text}\n… (показаны первые ${limit} байт)` : text;
}

/** Первые байты в hex — для выходов, которые не текст и не пакет. */
export function hexPreview(bytes: Uint8Array, limit = 32): string {
  const parts: string[] = [];
  for (const byte of bytes.subarray(0, Math.min(bytes.byteLength, limit))) {
    parts.push(byte.toString(16).padStart(2, '0'));
  }
  return parts.join(' ');
}

export function formatBytes(size: number): string {
  if (size < 1024) {
    return `${size} Б`;
  }
  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(1)} КиБ`;
  }
  return `${(size / (1024 * 1024)).toFixed(1)} МиБ`;
}
