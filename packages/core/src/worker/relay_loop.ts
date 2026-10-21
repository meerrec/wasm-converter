import type { FrameMsg } from '../protocol.js';

/**
 * Кадр без полей конверта: `type`, `frameId` и `dropped` назначает цикл.
 * `slot` — слот релея: по нему `frame-ack` вернёт буфер на место.
 */
export interface RelayFrame {
  slot: number;
  bytes: ArrayBuffer;
  len: number;
  cmds: number;
  buildMs: number;
}

/** Что вышло из попытки собрать кадр. */
export type RelayBuildOutcome =
  /** Кадр собран и ждёт отправки. */
  | { kind: 'frame'; frame: RelayFrame }
  /** Свободных буферов нет: ждём `frame-ack`, запрос остаётся грязным. */
  | { kind: 'busy' }
  /** Буфер вырос под кадр — пробуем ещё раз той же микрозадачей. */
  | { kind: 'retry' }
  /** Собирать нечего: нового запроса на кадр нет. */
  | { kind: 'idle' };

export interface RelayLoopCallbacks {
  /** Сборщик: кладёт кадр в свободный буфер релея. */
  build(): RelayBuildOutcome;
  /** Отправить кадр на main переводом владения буфером. */
  send(frame: FrameMsg): void;
}

export interface RelayLoop {
  /** Запросить кадр: сборка случится одна на микрозадачу. */
  request(): void;
  /** Пришёл `frame-ack`: буфер снова свободен, можно собирать. */
  ack(): void;
  stop(): void;
}

/**
 * Конвейер кадров main-режима.
 *
 * В отличие от `frame_loop` здесь нет painter'а в воркере и нет
 * `requestAnimationFrame`: сборка идёт по микрозадаче, отправка — сразу после
 * неё, а повторный запрос ждёт `frame-ack` от main. Собственный цикл, а не
 * переиспользование `frame_loop`, потому что в браузерах без OffscreenCanvas
 * worker-rAF может отсутствовать вовсе.
 *
 * Запросы коалесцируются: сколько бы раз ни звали `request()`, сборка случится
 * одна на микрозадачу. Если оба буфера в полёте, сборка откладывается, а
 * следующий отправленный кадр помечается `dropped` — main увидит пропуск в
 * `tick`.
 */
export function startRelayLoop(cb: RelayLoopCallbacks): RelayLoop {
  let stopped = false;
  /** Есть состояние, которого нет ни в одном готовом кадре. */
  let dirty = false;
  let scheduled = false;
  /** Сборка пропущена из-за занятых буферов — попадёт в следующий кадр. */
  let skipped = false;
  let frameId = 0;

  const run = () => {
    scheduled = false;
    if (stopped || !dirty) return;
    let outcome: RelayBuildOutcome;
    try {
      outcome = cb.build();
    } catch (e) {
      console.error('[relay_loop] build', e);
      return;
    }
    switch (outcome.kind) {
      case 'idle':
        dirty = false;
        return;
      case 'busy':
        // Буферы освободит `frame-ack`; до него повторять нечего.
        skipped = true;
        return;
      case 'retry':
        // Ёмкость уже выросла — следующий заход влезет.
        schedule();
        return;
      case 'frame': {
        dirty = false;
        const dropped = skipped;
        skipped = false;
        cb.send({
          type: 'frame',
          frameId: ++frameId,
          slot: outcome.frame.slot,
          bytes: outcome.frame.bytes,
          len: outcome.frame.len,
          cmds: outcome.frame.cmds,
          buildMs: outcome.frame.buildMs,
          dropped,
        });
        return;
      }
    }
  };

  const schedule = () => {
    if (scheduled || stopped) return;
    scheduled = true;
    queueMicrotask(run);
  };

  return {
    request() {
      if (stopped) return;
      dirty = true;
      schedule();
    },
    ack() {
      if (stopped || !dirty) return;
      schedule();
    },
    stop() {
      stopped = true;
    },
  };
}
