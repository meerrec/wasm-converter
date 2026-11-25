// Разбор заголовка PNG без декодера: валидатору manifest нужны только
// ширина, высота и тип цвета из IHDR. Полное декодирование здесь было бы
// лишним — пиксели читает движок сравнения, а не проверка схемы.

/** Сигнатура PNG: 8 фиксированных байтов в начале файла. */
const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] as const;

/** Длина IHDR-данных по спецификации PNG. */
const IHDR_LENGTH = 13;

/** Что удалось прочитать из IHDR. */
export interface PngHeader {
  width: number;
  height: number;
  bitDepth: number;
  /** Числовой код типа цвета PNG: 2 — RGB, 6 — RGBA. */
  colorType: number;
  /** Имя типа цвета в терминах manifest либо `null`, если формат не поддержан. */
  colorTypeName: 'RGB8' | 'RGBA8' | null;
}

/**
 * Прочитать IHDR из начала PNG.
 *
 * Возвращает `null`, если сигнатура не совпала, файл короче заголовка или
 * первый чанк — не IHDR. CRC не проверяется: битые пиксели поймает декодер,
 * а здесь важна только форма.
 */
export function parsePngHeader(bytes: Uint8Array): PngHeader | null {
  if (bytes.byteLength < 8 + 8 + IHDR_LENGTH) return null;
  for (let i = 0; i < PNG_SIGNATURE.length; i += 1) {
    if (bytes[i] !== PNG_SIGNATURE[i]) return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const chunkLength = view.getUint32(8, false);
  if (chunkLength !== IHDR_LENGTH) return null;
  const chunkType = String.fromCharCode(bytes[12] ?? 0, bytes[13] ?? 0, bytes[14] ?? 0, bytes[15] ?? 0);
  if (chunkType !== 'IHDR') return null;
  const width = view.getUint32(16, false);
  const height = view.getUint32(20, false);
  const bitDepth = bytes[24] ?? 0;
  const colorType = bytes[25] ?? 0;
  return {
    width,
    height,
    bitDepth,
    colorType,
    colorTypeName: colorTypeName(bitDepth, colorType),
  };
}

/** Тип цвета manifest: только 8-битные RGB и RGBA. */
function colorTypeName(bitDepth: number, colorType: number): 'RGB8' | 'RGBA8' | null {
  if (bitDepth !== 8) return null;
  if (colorType === 2) return 'RGB8';
  if (colorType === 6) return 'RGBA8';
  return null;
}
