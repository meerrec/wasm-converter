/**
 * Рантайм CLI под Deno.
 *
 * Пул — на веб-воркерах (`worker-deno.js`): протокол `worker-protocol.ts` от
 * транспорта не зависит. `workerUrl` обязателен только Node-ветке; здесь
 * адрес воркера берётся от `import.meta.url` этого модуля и указывает на
 * соседний `worker-deno.js`.
 *
 * wasm-байты ищутся через `import.meta.resolve` — в Deno нет `createRequire`
 * как основного пути разрешения пакетов. Разрешённый URL может быть
 * `file:` (локальная сборка, `node_modules`) или `npm:` (пакет из реестра);
 * для обоих пробуем `Deno.readFile`, затем `fetch`. Если разрешение не
 * удалось, последний шанс — `createRequire` из Node-совместимости.
 */

import { createRequire } from 'node:module';

import { WorkerPool, type WorkerLike } from './pool.js';
import type { CliRuntime } from './runtime.js';
import type { WorkerRequest, WorkerResponse } from './worker-protocol.js';

/** Часть API `Deno`, которой пользуется загрузчик. */
interface DenoApi {
  readFile(path: string | URL): Promise<Uint8Array>;
}

interface WebWorkerMessageEvent {
  data: WorkerResponse;
}

interface WebWorkerErrorEvent {
  error?: unknown;
  message?: string;
}

/** Часть API веб-воркера, которой пользуется пул (в тестах — двойник). */
interface WebWorkerLike {
  postMessage(message: WorkerRequest): void;
  addEventListener(type: 'message', listener: (event: WebWorkerMessageEvent) => void): void;
  addEventListener(type: 'error', listener: (event: WebWorkerErrorEvent) => void): void;
  terminate(): void;
}

interface WebWorkerConstructor {
  new (url: URL, options: { type: 'module' }): WebWorkerLike;
}

/** Глобалы Deno: в сборку не тянутся ни `@types/deno`, ни `lib.webworker`. */
interface DenoGlobals {
  Deno?: DenoApi;
  Worker?: WebWorkerConstructor;
  navigator?: { hardwareConcurrency?: number };
}

const globals = globalThis as unknown as DenoGlobals;

/**
 * Обёртка веб-воркера под `WorkerLike` пула.
 *
 * У веб-воркера нет события `exit`: `terminate()` синхронный, а падение
 * приходит событием `error` (или молчанием — пул тогда ждёт ответа).
 */
class DenoWorker implements WorkerLike {
  readonly #worker: WebWorkerLike;

  constructor(url: URL) {
    const ctor = globals.Worker;
    if (ctor === undefined) throw new Error('the Deno runtime has no Web Worker global');
    this.#worker = new ctor(url, { type: 'module' });
  }

  postMessage(message: WorkerRequest): void {
    this.#worker.postMessage(message);
  }

  on(event: 'message', listener: (message: WorkerResponse) => void): void;
  on(event: 'error', listener: (error: Error) => void): void;
  on(event: 'exit', listener: (code: number) => void): void;
  on(
    event: 'message' | 'error' | 'exit',
    listener: ((message: WorkerResponse) => void) | ((error: Error) => void) | ((code: number) => void),
  ): void {
    if (event === 'message') {
      const receive = listener as (message: WorkerResponse) => void;
      this.#worker.addEventListener('message', (message) => receive(message.data));
    } else if (event === 'error') {
      const fail = listener as (error: Error) => void;
      this.#worker.addEventListener('error', (event_) => {
        fail(event_.error instanceof Error ? event_.error : new Error(event_.message ?? 'worker failed'));
      });
    }
    // 'exit' у веб-воркера не приходит: остановку инициирует только пул.
  }

  terminate(): Promise<number> {
    this.#worker.terminate();
    return Promise.resolve(0);
  }
}

export function createRuntime(): CliRuntime {
  return {
    createPool(_workerUrl, size) {
      const pool = new WorkerPool(size, () => new DenoWorker(new URL('./worker-deno.js', import.meta.url)));
      return { submit: (job) => pool.run(job), close: () => pool.close() };
    },

    loadWasmBytes: wasmBytes,

    availableParallelism() {
      const value = globals.navigator?.hardwareConcurrency;
      return typeof value === 'number' && value > 0 ? value : 1;
    },
  };
}

/** Байты `.wasm` по спеке клея: `import.meta.resolve`, затем `createRequire`. */
async function wasmBytes(jsSpecifier: string): Promise<Uint8Array> {
  const errors: string[] = [];
  for (const candidate of resolveCandidates(jsSpecifier)) {
    try {
      return await readBytes(candidate);
    } catch (error) {
      errors.push(`${candidate}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  throw new Error(`cannot read wasm bytes for '${jsSpecifier}' under Deno (${errors.join('; ')})`);
}

/** URL'ы `.wasm`, лежащего рядом с клеем: от `import.meta.resolve` и `createRequire`. */
function resolveCandidates(jsSpecifier: string): string[] {
  const jsUrls: string[] = [];
  try {
    jsUrls.push(import.meta.resolve(jsSpecifier));
  } catch {
    // Разрешение отдадим `createRequire` ниже.
  }
  try {
    // Deno реализует `createRequire`; для npm-пакетов он отдаёт путь в кэше.
    jsUrls.push(createRequire(import.meta.url).resolve(jsSpecifier));
  } catch {
    // Оба способа не сработали — об этом скажет итоговая ошибка.
  }
  // Спека указывает на клей: `.wasm` лежит рядом с тем же именем.
  return [...new Set(jsUrls.map((url) => url.replace(/\.js$/, '_bg.wasm')))];
}

async function readBytes(url: string): Promise<Uint8Array> {
  const deno = globals.Deno;
  if (deno === undefined) throw new Error('the Deno runtime adapter is loaded outside Deno');

  if (url.startsWith('file:')) return deno.readFile(new URL(url));

  // `npm:`/`http(s):`-URL пакетного кэша: `readFile` может его не понять,
  // `fetch` отдаёт те же байты.
  try {
    return await deno.readFile(new URL(url));
  } catch {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return new Uint8Array(await response.arrayBuffer());
  }
}
