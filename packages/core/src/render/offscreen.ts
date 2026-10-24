import type { FrameTransport, RenderMode } from '../protocol.js';

/**
 * Решение о месте рендера кадра: воркер или main-поток.
 *
 * Фабрики просмотрщиков (`createXlsxViewer`/`createDocxViewer`/`createPptxViewer`)
 * спрашивают здесь режим и сами поднимают воркер; отдельной legacy-фабрики
 * `createViewer` и её обвязки (`initOffscreen`, `supportsOffscreen`,
 * `assertFallbackAvailable`) больше нет.
 */

/**
 * Выбрать, где рисовать кадр.
 *
 * `auto` переходит на main молча для хоста, но с одним `console.warn`: тихая
 * смена пути — это то, что потом ищут в отладчике. Явный `'worker'` без
 * `transferControlToOffscreen` — ошибка инициализации, а не подмена, по той же
 * причине, что и явный `sab` без заголовков (ADR-0021).
 *
 * `'main'` вместе с `transport: 'sab'` — ошибка конфигурации: в main-режиме
 * кадр едет буфером на другой поток, SAB там не участвует.
 *
 * @throws Error, если запрошенный режим несовместим с окружением или
 *   транспортом
 */
export function resolveRenderMode(
  canvas: HTMLCanvasElement,
  requested: RenderMode | undefined,
  transport?: 'auto' | FrameTransport,
): 'worker' | 'main' {
  const canWorker = typeof canvas.transferControlToOffscreen === 'function';
  if (requested === 'main') {
    if (transport === 'sab') {
      throw new Error(
        "renderMode: 'main' несовместим с transport: 'sab' — в main-режиме кадры едут буферами",
      );
    }
    return 'main';
  }
  if (requested === 'worker') {
    if (!canWorker) {
      throw new Error(
        'этот браузер не поддерживает OffscreenCanvas (transferControlToOffscreen) — просмотрщик не запустится в режиме worker',
      );
    }
    return 'worker';
  }
  if (canWorker) return 'worker';
  console.warn(
    '[doc-converter] OffscreenCanvas (transferControlToOffscreen) недоступен: рендер на main-потоке',
  );
  return 'main';
}
