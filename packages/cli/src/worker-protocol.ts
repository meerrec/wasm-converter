/**
 * Сообщения main ↔ worker пула. Отдельный модуль, чтобы пул и воркер
 * сходились по типам, а `worker_threads` не тянулся в main только ради
 * описания протокола.
 */

import type { Diagnostic } from '@meerrec/core';

import type { ConvertJob, ConvertResult } from './convert.js';

export interface WorkerRequest {
  id: number;
  job: ConvertJob;
}

/**
 * `error` — строка для совместимости со старыми воркерами; `diagnostic` —
 * машиночитаемая причина, если воркер её разметил. Поле необязательное:
 * ответ без диагностики пул превращает в `CliError` без неё.
 */
export type WorkerResponse =
  | { id: number; ok: true; result: ConvertResult }
  | { id: number; ok: false; error: string; diagnostic?: Diagnostic };
