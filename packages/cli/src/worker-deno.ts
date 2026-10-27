/**
 * Точка входа воркера пула под Deno.
 *
 * Зеркало `worker.ts` без `node:worker_threads`: Deno запускает веб-воркер,
 * поэтому канал — `self.onmessage` / `self.postMessage`, а протокол тот же
 * (`worker-protocol.ts`). Воркер живёт долго и переиспользует wasm-модуль,
 * как и Node-версия.
 */

import { toDiagnostic } from '@meerrec/core';

import { convertFile } from './convert.js';
import type { WorkerRequest, WorkerResponse } from './worker-protocol.js';

/** Часть API `DedicatedWorkerGlobalScope`, которой пользуется воркер. */
interface WorkerScope {
  onmessage: ((event: { data: WorkerRequest }) => void) | null;
  postMessage(message: WorkerResponse): void;
}

const scope = globalThis as unknown as Partial<WorkerScope>;
if (typeof scope.postMessage !== 'function') {
  throw new Error('worker-deno.js is meant to run as a web worker');
}

const port = scope as WorkerScope;
port.onmessage = (event) => {
  void handle(event.data);
};

async function handle(message: WorkerRequest): Promise<void> {
  try {
    const result = await convertFile(message.job);
    port.postMessage({ id: message.id, ok: true, result } satisfies WorkerResponse);
  } catch (error) {
    // Зеркало `worker.ts`: строка для совместимости и машинная диагностика.
    const diagnostic = toDiagnostic(error, { code: 'internal', layer: 'cli' });
    port.postMessage({
      id: message.id,
      ok: false,
      error: diagnostic.message,
      diagnostic,
    } satisfies WorkerResponse);
  }
}
