/**
 * Пул воркеров для `batch`.
 *
 * Каждая задача — конвертация одного файла. Воркеров ровно `size`, очередь
 * общая: воркер, закончивший файл, берёт следующий, поэтому медленный файл
 * не держит остальных. Упавший воркер не роняет прогон — его задача
 * помечается неудачной, а место занимает новый воркер, пока есть работа.
 */

import { createRequire } from 'node:module';
import type * as NodeWorkerThreads from 'node:worker_threads';

import type { ConvertJob, ConvertResult } from './convert.js';
import { CliError } from './errors.js';
import type { WorkerRequest, WorkerResponse } from './worker-protocol.js';

/**
 * Часть API воркера, которой пользуется пул (в тестах — двойник).
 *
 * Модуль не знает о транспорте: `node:worker_threads`, веб-воркер Deno или
 * заглушка теста прячутся за этим интерфейсом.
 */
export interface WorkerLike {
  postMessage(message: WorkerRequest): void;
  on(event: 'message', listener: (message: WorkerResponse) => void): void;
  on(event: 'error', listener: (error: Error) => void): void;
  on(event: 'exit', listener: (code: number) => void): void;
  terminate(): Promise<number>;
}

export type WorkerFactory = () => WorkerLike;

interface Task {
  job: ConvertJob;
  resolve: (result: ConvertResult) => void;
  reject: (error: Error) => void;
}

export class WorkerPool {
  readonly #factory: WorkerFactory;
  readonly #live = new Set<WorkerLike>();
  readonly #idle: WorkerLike[] = [];
  readonly #busy = new Map<WorkerLike, Task>();
  readonly #dead = new Set<WorkerLike>();
  readonly #queue: Task[] = [];
  #respawnBudget: number;
  #nextId = 0;
  #closed = false;

  constructor(
    readonly size: number,
    factory: WorkerFactory = nodeWorkerFactory,
  ) {
    if (size < 1) throw new CliError('worker pool needs at least one worker');
    // Запас на замену: воркер, падающий на старте, не должен крутить пул вечно.
    this.#respawnBudget = size * 4 + 4;
    this.#factory = factory;
    for (let i = 0; i < size; i += 1) this.#idle.push(this.#spawn());
  }

  /** Сколько воркеров живо: тесты и диагностика. */
  get live(): number {
    return this.#live.size;
  }

  /** Поставить задачу в очередь; ответ придёт по завершении файла. */
  run(job: ConvertJob): Promise<ConvertResult> {
    return new Promise<ConvertResult>((resolve, reject) => {
      if (this.#closed) {
        reject(new CliError('worker pool is closed', 1));
        return;
      }
      this.#queue.push({ job, resolve, reject });
      this.#dispatch();
    });
  }

  /** Дождаться завершения всех задач и погасить воркеры. */
  async close(): Promise<void> {
    this.#closed = true;
    const error = new CliError('worker pool is closed', 1);
    for (const task of this.#queue.splice(0)) task.reject(error);
    const workers = [...this.#live];
    await Promise.all(workers.map(async (worker) => this.#stop(worker)));
  }

  #spawn(): WorkerLike {
    const worker = this.#factory();
    worker.on('message', (message) => this.#onMessage(worker, message));
    worker.on('error', (error) => this.#retire(worker, `worker failed: ${error.message}`));
    worker.on('exit', (code) => this.#retire(worker, `worker exited with code ${code}`));
    this.#live.add(worker);
    return worker;
  }

  #dispatch(): void {
    this.#ensureWorkers();
    while (this.#queue.length > 0 && this.#idle.length > 0) {
      const worker = this.#idle.pop()!;
      const task = this.#queue.shift()!;
      this.#busy.set(worker, task);
      this.#nextId += 1;
      worker.postMessage({ id: this.#nextId, job: task.job });
    }
  }

  /** Добрать воркеров до `size`, если есть работа: замена упавшим. */
  #ensureWorkers(): void {
    while (this.#live.size < this.size && (this.#queue.length > 0 || this.#busy.size > 0)) {
      if (this.#respawnBudget === 0) {
        const error = new CliError('no workers left: the pool could not restart them', 1);
        for (const task of this.#queue.splice(0)) task.reject(error);
        return;
      }
      this.#respawnBudget -= 1;
      this.#idle.push(this.#spawn());
    }
  }

  #onMessage(worker: WorkerLike, message: WorkerResponse): void {
    const task = this.#busy.get(worker);
    if (task === undefined) return;
    this.#busy.delete(worker);
    if (message.ok) task.resolve(message.result);
    else task.reject(new CliError(message.error, 1));
    if (!this.#dead.has(worker)) this.#idle.push(worker);
    if (!this.#closed) this.#dispatch();
  }

  /** Вывести воркер из строя: его задача — с ошибкой, место — под замену. */
  #retire(worker: WorkerLike, message: string): void {
    if (this.#dead.has(worker)) return;
    this.#dead.add(worker);
    this.#live.delete(worker);
    const index = this.#idle.indexOf(worker);
    if (index >= 0) this.#idle.splice(index, 1);

    const task = this.#busy.get(worker);
    if (task !== undefined) {
      this.#busy.delete(worker);
      task.reject(new CliError(message, 1));
    }
    worker.terminate().catch(() => {
      // Воркер уже мёртв — гасить нечего.
    });
    if (!this.#closed) this.#dispatch();
  }

  async #stop(worker: WorkerLike): Promise<void> {
    this.#dead.add(worker);
    this.#live.delete(worker);
    const task = this.#busy.get(worker);
    if (task !== undefined) {
      this.#busy.delete(worker);
      task.reject(new CliError('worker pool is closed', 1));
    }
    try {
      await worker.terminate();
    } catch {
      // Воркер уже мёртв — гасить нечего.
    }
  }
}

/**
 * Умолчание для программного API: `new WorkerPool(size)` как раньше.
 *
 * `node:worker_threads` подгружается `require`-ом только при вызове: Deno
 * всегда передаёт свою фабрику из `runtime-deno`, поэтому модуль воркеров
 * Node в его граф не попадает. `node:module` есть во всех трёх рантаймах.
 */
function nodeWorkerFactory(): WorkerLike {
  const require = createRequire(import.meta.url);
  const workerThreads = require('node:worker_threads') as typeof NodeWorkerThreads;
  return new workerThreads.Worker(new URL('./worker.js', import.meta.url)) as unknown as WorkerLike;
}


