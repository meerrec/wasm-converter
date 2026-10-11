/**
 * Сообщения main ↔ worker пула. Отдельный модуль, чтобы пул и воркер
 * сходились по типам, а `worker_threads` не тянулся в main только ради
 * описания протокола.
 */

import type { ConvertJob, ConvertResult } from './convert.js';

export interface WorkerRequest {
  id: number;
  job: ConvertJob;
}

export type WorkerResponse =
  | { id: number; ok: true; result: ConvertResult }
  | { id: number; ok: false; error: string };
