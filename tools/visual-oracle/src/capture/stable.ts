// Ожидание честно стабильного кадра.
//
// В просмотрщике нет события «кадр готов»: `onTick` приходит на каждый
// нарисованный кадр, но сам по себе он не доказывает, что картинка перестала
// меняться (шрифты догружаются, раскладка досчитывается, painter догоняет
// очередь). Поэтому правило двух шагов:
//
// 1. после изменения состояния ждём хотя бы один `onTick` — painter отработал;
// 2. опрашиваем `exportPng()` и требуем два подряд одинаковых SHA-256.
//
// Второй шаг дорогой (полный PNG на каждый опрос), но он единственный, который
// не врёт: одинаковые байты двух последовательных снимков означают, что между
// ними ничего не перерисовалось. Ждать фиксированным `sleep` вместо этого
// запрещено политикой baseline (baseline-policy.md §3.7).

import { sha256Hex } from '../hash.js';
import { deferred } from './deferred.js';
import { CaptureError } from './errors.js';

export { sha256Hex };

/** Минимум, который нужен от вьюера: снимок кадра и подписка на тики. */
export interface StableFrameViewer {
  exportPng(opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  onTick(handler: (stats: unknown) => void): () => void;
}

/** Настройки ожидания стабильного кадра. */
export interface StableFrameOptions {
  /**
   * Действие, меняющее состояние вьюера (`showPage`, `setZoom`, …). Оно
   * выполняется уже после подписки на `onTick`: иначе тик, пришедший между
   * изменением и подпиской, был бы потерян, и ожидание ушло бы в таймаут.
   */
  change?: () => void | Promise<void>;
  /** Сколько ждать двух одинаковых хешей подряд. По умолчанию 20 с. */
  timeoutMs?: number;
  /** Сколько ждать первого `onTick` после изменения. По умолчанию 30 с. */
  tickTimeoutMs?: number;
  /** Пауза между опросами `exportPng`. По умолчанию 50 мс. */
  pollIntervalMs?: number;
  /** Отмена ожидания. */
  signal?: AbortSignal;
}

/** Чем закончилось ожидание: оба хеша, сам кадр и сколько это заняло. */
export interface StableFrameResult {
  /** Два подряд совпавших SHA-256 кадра. */
  hashes: [string, string];
  /**
   * Байты кадра, чей SHA-256 равен `hashes[1]`.
   *
   * Именно этот буфер доказан стабильным, поэтому писать в артефакт нужно его,
   * а не свежий `exportPng`: между подтверждением и повторным снимком может
   * пройти перерисовка, и тогда в файл уедет непроверенная картинка.
   */
  bytes: Uint8Array;
  /** Время от изменения состояния до подтверждённой стабильности. */
  elapsedMs: number;
}

const DEFAULT_TIMEOUT_MS = 20_000;
const DEFAULT_TICK_TIMEOUT_MS = 30_000;
const DEFAULT_POLL_INTERVAL_MS = 50;

/** Пауза, прерываемая сигналом отмены. */
function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  const { promise, resolve, reject } = deferred<void>();
  if (signal?.aborted === true) {
    reject(new CaptureError('unstable-frame', 'stable frame wait aborted'));
    return promise;
  }
  const timer = setTimeout(() => {
    signal?.removeEventListener('abort', onAbort);
    resolve();
  }, ms);
  function onAbort(): void {
    clearTimeout(timer);
    reject(new CaptureError('unstable-frame', 'stable frame wait aborted'));
  }
  signal?.addEventListener('abort', onAbort, { once: true });
  return promise;
}

/**
 * Дождаться стабильного кадра.
 *
 * @throws {CaptureError} код `unstable-frame` — не пришёл тик или кадр не
 * перестал меняться за отведённое время. Последний снимок при этом не
 * возвращается: выдать нестабильный кадр за стабильный — худшее, что может
 * сделать оракул.
 */
export async function waitForStableFrame(
  viewer: StableFrameViewer,
  options: StableFrameOptions = {},
): Promise<StableFrameResult> {
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const tickTimeoutMs = options.tickTimeoutMs ?? DEFAULT_TICK_TIMEOUT_MS;
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  const started = Date.now();

  // Подписка до изменения: тик, пришедший сразу после `showPage`, обязан быть
  // замечен, иначе ожидание честно упадёт по таймауту на быстром документе.
  //
  // Тик, пришедший во время асинхронного изменения (`setSelection` ждёт
  // воркер), тоже считается: painter в этот момент уже рисовал новое
  // состояние. Настоящая гарантия — не тик, а два одинаковых хеша ниже.
  const tickArrived = deferred<void>();
  let armed = false;
  let tickSeen = false;
  // Счётчик тиков, а не флаг: снимок, сделанный до отрисовки уже запрошенного
  // кадра, вернул бы старые байты, и два таких снимка подряд выглядели бы
  // стабильностью. Тик между снимками обнуляет совпадение.
  let ticks = 0;
  const unsubscribe = viewer.onTick(() => {
    ticks += 1;
    tickSeen = true;
    if (armed) tickArrived.resolve();
  });

  try {
    await options.change?.();
    armed = true;
    if (tickSeen) tickArrived.resolve();

    const tickTimer = setTimeout(() => {
      tickArrived.reject(
        new CaptureError(
          'unstable-frame',
          `viewer produced no tick within ${tickTimeoutMs} ms after the state change`,
        ),
      );
    }, tickTimeoutMs);
    try {
      await tickArrived.promise;
    } finally {
      clearTimeout(tickTimer);
    }

    let previous: string | null = null;
    for (;;) {
      if (options.signal?.aborted === true) {
        throw new CaptureError('unstable-frame', 'stable frame wait aborted');
      }
      if (Date.now() - started > timeoutMs) {
        throw new CaptureError(
          'unstable-frame',
          `frame did not stabilise within ${timeoutMs} ms` +
            (previous === null ? '' : ` (last hash ${previous})`),
        );
      }
      const ticksBefore = ticks;
      let bytes: Uint8Array;
      try {
        bytes = await viewer.exportPng(
          options.signal === undefined ? {} : { signal: options.signal },
        );
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        throw new CaptureError('export-failed', `exportPng failed while polling: ${detail}`, {
          cause: error,
        });
      }
      const hash = sha256Hex(bytes);
      if (ticks !== ticksBefore) {
        // Пока снимали, painter нарисовал ещё кадр: снимок мог быть сделан до
        // него, и сравнивать его с предыдущим нельзя.
        previous = null;
        continue;
      }
      if (previous !== null && previous === hash) {
        // `bytes` — ровно тот буфер, чей хеш только что совпал с предыдущим:
        // его и отдаём, чтобы вызывающий писал доказанный кадр, а не новый.
        return { hashes: [previous, hash], bytes, elapsedMs: Date.now() - started };
      }
      previous = hash;
      await sleep(pollIntervalMs, options.signal);
    }
  } finally {
    unsubscribe();
  }
}
