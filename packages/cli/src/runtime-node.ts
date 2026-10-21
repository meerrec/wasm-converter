/**
 * Рантайм CLI под Node и Bun.
 *
 * Пул — на `node:worker_threads`, wasm-байты читаются рядом с клеем через
 * `createRequire` (в pnpm-раскладке пакет лежит в сторе, и путь из
 * `import.meta.url` туда не ведёт), число ядер — у `node:os`.
 *
 * Bun идёт этой же веткой: перечисленные модули он реализует; отдельный
 * `runtime-bun.ts` появится только при фактической несовместимости.
 */

import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { availableParallelism } from 'node:os';
import { Worker } from 'node:worker_threads';

import { WorkerPool, type WorkerLike } from './pool.js';
import type { CliRuntime } from './runtime.js';

export function createRuntime(): CliRuntime {
  return {
    createPool(workerUrl, size) {
      const pool = new WorkerPool(size, () => new Worker(new URL(workerUrl)) as unknown as WorkerLike);
      return { submit: (job) => pool.run(job), close: () => pool.close() };
    },

    async loadWasmBytes(jsSpecifier) {
      const require = createRequire(import.meta.url);
      const jsPath = require.resolve(jsSpecifier);
      return readFile(jsPath.replace(/\.js$/, '_bg.wasm'));
    },

    availableParallelism: () => availableParallelism(),
  };
}
