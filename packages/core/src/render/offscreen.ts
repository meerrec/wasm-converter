import type { FrameTransport, RenderMode } from '../protocol.js';
import { resolveRenderTarget } from '../execution/plan.js';

/**
 * Решение о месте рендера кадра: воркер или main-поток.
 *
 * Сам выбор живёт в `execution/plan.ts`: он общий с воркером, который по тому
 * же плану выбирает транспорт. Здесь остаётся только то, что знает DOM, —
 * есть ли `transferControlToOffscreen`, — и предупреждение о fallback.
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
  const target = resolveRenderTarget({
    requested,
    canOffscreen: typeof canvas.transferControlToOffscreen === 'function',
    transport,
  });
  // `auto` без OffscreenCanvas — штатный fallback, но не молчаливый: смена
  // пути видна в консоли. Явные режимы сюда не попадают.
  if ((requested === undefined || requested === 'auto') && target === 'main') {
    console.warn(
      '[doc-converter] OffscreenCanvas (transferControlToOffscreen) недоступен: рендер на main-потоке',
    );
  }
  return target;
}
