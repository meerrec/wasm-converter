// Подписка отмены запроса на `AbortSignal` (ADR-0032).
//
// На `abort` воркеру уходит адресный `cancel` — кооперативный флаг остановит
// операцию в ближайшей точке проверки (блок раскладки DOCX, полоса кадра
// XLSX). Обёрнутые resolve/reject снимают listener при исходе запроса: без
// этого он висел бы до сборки мусора, а воркер получал бы `cancel` на уже
// завершённые id.

import type { InMsg } from '../protocol.js';

/** Пара колбэков исхода запроса; listener `abort` снят при любом исходе. */
export interface AbortGuard<T> {
  resolve: (value: T) => void;
  reject: (error: Error) => void;
}

/**
 * Взвести отмену запроса и обернуть колбэки исхода.
 *
 * Возвращает `null`, если сигнал уже отменён: вызывающий реджектит промис
 * сразу, не отправляя воркеру ни одного сообщения.
 */
export function armAbort<T>(
  signal: AbortSignal | undefined,
  post: (msg: InMsg) => void,
  id: number,
  resolve: (value: T) => void,
  reject: (error: Error) => void,
): AbortGuard<T> | null {
  if (!signal) return { resolve, reject };
  if (signal.aborted) return null;
  const onAbort = () => post({ type: 'cancel', id });
  signal.addEventListener('abort', onAbort, { once: true });
  return {
    resolve: (value) => {
      signal.removeEventListener('abort', onAbort);
      resolve(value);
    },
    reject: (error) => {
      signal.removeEventListener('abort', onAbort);
      reject(error);
    },
  };
}
