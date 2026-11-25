// Декодирование и кодирование PNG для движка сравнения.
//
// Профиль `visual-v1` работает только с 8-битными RGB8 и RGBA8: manifest
// разрешает ровно эти два типа цвета. Всё остальное (палитра, grayscale,
// 16 бит) отвергается типизированной ошибкой, а не молча приводится к RGBA:
// иначе сравнение шло бы с другим сэмплингом, чем записан в эталоне, и
// «совпадение» ничего не доказывало бы.
//
// RGB8 расширяется до RGBA с alpha = 255 — это делает сам pngjs при разборе
// (bitmapper), здесь остаётся только проверить длину результата.

import { PNG } from 'pngjs';

import { parsePngHeader } from '../png.js';

/** Изображение в RGBA8: 4 байта на пиксель, строки сверху вниз. */
export interface RgbaImage {
  width: number;
  height: number;
  /** Длина ровно `width * height * 4`. */
  data: Uint8Array;
}

/** Ошибка PNG: формат вне профиля `visual-v1` или битые байты. */
export class PngError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = 'PngError';
  }
}

/**
 * Декодировать PNG в RGBA8.
 *
 * @throws {PngError} если сигнатура или IHDR не разобраны, тип цвета вне
 * RGB8/RGBA8 либо данные не декодируются.
 */
export function decodePng(bytes: Uint8Array): RgbaImage {
  const header = parsePngHeader(bytes);
  if (header === null) {
    throw new PngError('PNG signature or IHDR chunk is invalid');
  }
  if (header.colorTypeName === null) {
    throw new PngError(
      `unsupported PNG color type ${header.colorType} at bit depth ${header.bitDepth}; only RGB8 and RGBA8 are accepted`,
    );
  }

  let decoded: PNG;
  try {
    decoded = PNG.sync.read(toBuffer(bytes));
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new PngError(`cannot decode PNG: ${detail}`, { cause: error });
  }

  const expected = decoded.width * decoded.height * 4;
  const data = new Uint8Array(decoded.data.buffer, decoded.data.byteOffset, decoded.data.byteLength);
  if (data.byteLength !== expected) {
    throw new PngError(
      `decoded PNG has ${data.byteLength} bytes, expected ${expected} for ${decoded.width}x${decoded.height} RGBA`,
    );
  }
  return { width: decoded.width, height: decoded.height, data };
}

/**
 * Закодировать RGBA8 в PNG.
 *
 * Выход детерминирован: одинаковые пиксели и одни и те же параметры deflate
 * дают одинаковые байты, поэтому отчёт воспроизводим.
 *
 * @throws {PngError} если длина буфера не соответствует заявленным размерам.
 */
export function encodePng(image: RgbaImage): Uint8Array {
  const expected = image.width * image.height * 4;
  if (image.data.byteLength !== expected) {
    throw new PngError(
      `RGBA buffer has ${image.data.byteLength} bytes, expected ${expected} for ${image.width}x${image.height}`,
    );
  }

  // `PNG.sync.write` принимает объект с полями width/height/data, но типы
  // пакета требуют экземпляр PNG. Создаём пустой экземпляр и подставляем
  // буфер: конструктор с нулевыми размерами пиксельной памяти не выделяет.
  const png = new PNG();
  png.width = image.width;
  png.height = image.height;
  png.data = toBuffer(image.data);

  const encoded = PNG.sync.write(png, {
    colorType: 6,
    inputColorType: 6,
    inputHasAlpha: true,
    deflateLevel: 9,
    deflateStrategy: 3,
  });
  return new Uint8Array(encoded.buffer, encoded.byteOffset, encoded.byteLength);
}

/** Буфер без копирования: pngjs работает с `Buffer`, а не с `Uint8Array`. */
function toBuffer(bytes: Uint8Array): Buffer {
  return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}
