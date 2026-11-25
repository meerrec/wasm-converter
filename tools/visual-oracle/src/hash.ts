// Хеши артефактов: один и тот же SHA-256 нужен и валидатору manifest, и
// импортёру эталонов, и алгоритму `baselineId`. Отдельный модуль, чтобы
// канонизация и хеширование не разъезжались между потребителями.

import { createHash } from 'node:crypto';

/** SHA-256 байтов в нижнем hex. */
export function sha256Hex(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

/** SHA-256 UTF-8 текста в нижнем hex. */
export function sha256Text(text: string): string {
  return createHash('sha256').update(text, 'utf8').digest('hex');
}
