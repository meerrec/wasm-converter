/**
 * Точка входа воркера пула под Deno.
 *
 * Зеркало `worker.ts` без `node:worker_threads`: Deno запускает веб-воркер,
 * поэтому канал — `self.onmessage` / `self.postMessage`, а протокол тот же
 * (`worker-protocol.ts`). Воркер живёт долго и переиспользует wasm-модуль,
 * как и Node-версия.
 */

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
    const text = error instanceof Error ? error.message : String(error);
    port.postMessage({ id: message.id, ok: false, error: text } satisfies WorkerResponse);
  }
}
