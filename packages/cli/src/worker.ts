/**
 * Точка входа воркера пула.
 *
 * Воркер живёт долго и обрабатывает много файлов подряд: wasm-модуль
 * (`convert` → `wasm.ts`) грузится при первой задаче и остаётся в памяти
 * воркера, поэтому за файл не платят ни загрузкой, ни инициализацией.
 * Задачи воркер получает по одной — их раздаёт пул.
 */

import { parentPort } from 'node:worker_threads';

import { convertFile } from './convert.js';
import type { WorkerRequest, WorkerResponse } from './worker-protocol.js';

const port = parentPort;
if (port === null) {
  throw new Error('worker.js is meant to run as a worker thread');
}

port.on('message', (message: WorkerRequest) => {
  void handle(message);
});

async function handle(message: WorkerRequest): Promise<void> {
  try {
    const result = await convertFile(message.job);
    port!.postMessage({ id: message.id, ok: true, result } satisfies WorkerResponse);
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    port!.postMessage({ id: message.id, ok: false, error: text } satisfies WorkerResponse);
  }
}
