// Единая точка выбора пути кадра: где рисуем и каким транспортом возим.
//
// До этого решения было два независимых места (`resolveRenderMode` на main и
// `pickTransport` в воркере) и четыре ad-hoc ветки в `WorkerCore`. Здесь
// матрица описана один раз: недопустимая пара `main + sab` не «проверяется
// позже», а не может быть построена.
//
// Модуль чистый: ни DOM, ни wasm. `canOffscreen` и `sabAvailable` приходят
// снаружи, поэтому обе стороны (main и воркер) проверяются одним и тем же
// кодом и тестируются без браузера.

import type { FrameTransport, RenderMode } from '../protocol.js';

/** Где рисуется кадр. */
export type RenderTarget = 'worker' | 'main';

/** Результат выбора: место рендера и транспорт кадра. */
export interface ExecutionPlan {
  readonly render: RenderTarget;
  readonly transport: FrameTransport;
}

/** Вход выбора места рендера. */
export interface RenderTargetInput {
  /** Что попросил хост: `auto` — по возможностям окружения. */
  readonly requested: RenderMode | undefined;
  /** Есть ли `canvas.transferControlToOffscreen`. */
  readonly canOffscreen: boolean;
  /** Транспорт из опций: нужен, чтобы отклонить `main + sab` сразу. */
  readonly transport: 'auto' | FrameTransport | undefined;
}

/**
 * Выбрать место рендера.
 *
 * `auto` уходит на main только без `OffscreenCanvas`; явный `worker` без него —
 * ошибка инициализации, а не тихий переход. `main` вместе с `sab` — ошибка
 * конфигурации: кольцо живёт в воркере, main-путь всегда возит буферами.
 *
 * @throws Error, если запрошенный режим несовместим с окружением или
 *   транспортом
 */
export function resolveRenderTarget(input: RenderTargetInput): RenderTarget {
  if (input.requested === 'main') {
    rejectMainSab(input.transport);
    return 'main';
  }
  if (input.requested === 'worker') {
    if (!input.canOffscreen) {
      throw new Error(
        'этот браузер не поддерживает OffscreenCanvas (transferControlToOffscreen) — просмотрщик не запустится в режиме worker',
      );
    }
    return 'worker';
  }
  // auto: воркер, если холст можно отдать ему; иначе main. Проверка `sab`
  // здесь тоже нужна: fallback на main несовместим с SAB так же, как явный.
  if (input.canOffscreen) return 'worker';
  rejectMainSab(input.transport);
  return 'main';
}

/** Вход выбора транспорта на уже известном месте рендера. */
export interface ExecutionPlanInput {
  readonly render: RenderTarget;
  readonly requested: 'auto' | FrameTransport | undefined;
  /** Удалось ли выделить `SharedArrayBuffer`. */
  readonly sabAvailable: boolean;
}

/**
 * Выбрать транспорт кадра.
 *
 * `main` всегда получает `bytes`: DisplayList уезжает на другой поток
 * переводом владения, SAB-кольцо там не участвует. В воркере `auto` берёт SAB,
 * если он есть, иначе буферы; явный `sab` без SAB — ошибка, а не подмена.
 *
 * @throws Error, если `main` запросил `sab` или явный `sab` недоступен
 */
export function resolveExecutionPlan(input: ExecutionPlanInput): ExecutionPlan {
  if (input.render === 'main') {
    rejectMainSab(input.requested);
    return { render: 'main', transport: 'bytes' };
  }
  if (input.requested === 'bytes') return { render: 'worker', transport: 'bytes' };
  if (input.sabAvailable) return { render: 'worker', transport: 'sab' };
  if (input.requested === 'sab') {
    throw new Error(
      'SharedArrayBuffer недоступен: странице нужны заголовки COOP/COEP (crossOriginIsolated)',
    );
  }
  return { render: 'worker', transport: 'bytes' };
}

/** `main` и `sab` несовместимы: у main-пути нет ring'а. */
function rejectMainSab(transport: 'auto' | FrameTransport | undefined): void {
  if (transport === 'sab') {
    throw new Error(
      "renderMode 'main' несовместим с transport 'sab': кадры едут буферами",
    );
  }
}
