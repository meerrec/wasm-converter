/**
 * Граница рантайма CLI: Node/Bun и Deno.
 *
 * Общий код (`convert`, `run`, `wasm`) работает только с интерфейсами этого
 * модуля. Всё, что зависит от рантайма — пул воркеров, чтение wasm-байтов и
 * число ядер — живёт в `runtime-node.ts` / `runtime-deno.ts`. Выбор
 * реализации динамический: `node:worker_threads` не должен попадать в граф
 * модулей Deno, где его нет.
 *
 * Bun идёт по Node-ветке: он реализует нужные `node:*`-модули, отдельный
 * адаптер заводится только при фактической несовместимости.
 */

import type { ConvertJob, ConvertResult } from './convert.js';

/** Рантайм, под которым запущен CLI. */
export type RuntimeKind = 'node' | 'bun' | 'deno';

/** Пул воркеров глазами `run`: очередь задач и остановка. */
export interface PoolBackend {
  submit(job: ConvertJob): Promise<ConvertResult>;
  close(): Promise<void>;
}

/** Всё, что CLI спрашивает у рантайма. */
export interface CliRuntime {
  /** Создать пул; `workerUrl` — URL входа воркера строкой (Node/Bun). */
  createPool(workerUrl: string, size: number): PoolBackend;

  /** Байты `.wasm`, лежащего рядом с клеем `jsSpecifier`. */
  loadWasmBytes(jsSpecifier: string): Promise<Uint8Array>;

  /** Доступные ядра: умолчание `--workers`. */
  availableParallelism(): number;
}

/**
 * Определить рантайм по глобалям.
 *
 * `Deno` проверяется первым: в Deno с включённой Node-совместимостью может
 * быть и `process.versions`. `process` читается через каст, чтобы модуль не
 * зависел от типов конкретного рантайма.
 */
export function detectRuntime(): RuntimeKind {
  if ('Deno' in globalThis) return 'deno';
  // Глобалы рантаймов типизированы по-разному; читаем только поле Bun.
  const globals = globalThis as unknown as { process?: { versions?: { bun?: string } } };
  return globals.process?.versions?.bun === undefined ? 'node' : 'bun';
}

let runtime: Promise<CliRuntime> | undefined;

/**
 * Взять реализацию рантайма, загрузив её один раз на процесс.
 *
 * Импорт динамический: статический `runtime-node.ts` притянул бы
 * `node:worker_threads` в Deno, где модуль не загрузится.
 */
export function loadRuntime(): Promise<CliRuntime> {
  runtime ??=
    detectRuntime() === 'deno'
      ? import('./runtime-deno.js').then((module) => module.createRuntime())
      : import('./runtime-node.js').then((module) => module.createRuntime());
  return runtime;
}
