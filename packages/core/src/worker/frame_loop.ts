import type { PaintStats } from '../protocol.js';

export type { PaintStats };

/** Что вышло из попытки собрать кадр. */
export type BuildOutcome =
  /** Кадр собран и ждёт отрисовки. */
  | 'built'
  /** Собрать не удалось: не было свободного буфера (слот мал — кадр больше него). */
  | 'dropped'
  /** Собирать нечего: нового запроса на кадр не было. */
  | 'idle';

export interface FrameLoopCallbacks {
  /** Сборщик: кладёт кадр в свободный буфер. */
  build(): BuildOutcome;
  /** Painter: рисует самый свежий готовый кадр, остальные пропускает. */
  paint(): { cmds: number; dropped: boolean; paintMs: number };
  onTick(stats: PaintStats): void;
}

/** Управление циклом кадров: запрос перерисовки и остановка. */
export interface FrameLoop {
  /** Запросить кадр: сборка случится одна на микрозадачу. */
  request(): void;
  /** Остановить цикл. */
  stop(): void;
}

/**
 * Конвейер кадров: сборка и отрисовка — разные шаги, а не один.
 *
 * Сборщик работает по факту запроса (микрозадача), painter — по кадру экрана
 * (`requestAnimationFrame`). Поэтому кадр обычно бывает собран ещё до
 * вертикальной синхронизации, и тайминг отрисовки не платит за сборку
 * DisplayList. Пока painter рисует один кадр, сборщик успевает положить
 * второй — это и есть двойная буферизация: у транспорта два слота, и второй
 * занят следующей сборкой, а не простаивает.
 *
 * Запросы коалесцируются: сколько бы раз ни позвали `request()`, сборка
 * случится одна на микрозадачу, отрисовка — одна на кадр экрана.
 */
export function startFrameLoop(cb: FrameLoopCallbacks): FrameLoop {
  let stopped = false;
  /** Есть состояние, которого нет ни в одном готовом кадре. */
  let dirty = false;
  let buildScheduled = false;
  let paintScheduled = false;
  let frameId = 0;

  const runBuild = () => {
    buildScheduled = false;
    if (stopped || !dirty) return;
    let outcome: BuildOutcome;
    try {
      outcome = cb.build();
    } catch (e) {
      console.error('[frame_loop] build', e);
      return;
    }
    // Кадр собрать не удалось (буферы заняты) — состояние остаётся «грязным»:
    // его дособерут после отрисовки, когда буфер освободится.
    if (outcome === 'built') dirty = false;
    if (outcome === 'idle') {
      dirty = false;
      return;
    }
    schedulePaint();
  };

  const scheduleBuild = () => {
    if (buildScheduled || stopped) return;
    buildScheduled = true;
    queueMicrotask(runBuild);
  };

  const runPaint = () => {
    paintScheduled = false;
    if (stopped) return;
    try {
      const stats = cb.paint();
      cb.onTick({ ...stats, frameId: ++frameId });
    } catch (e) {
      // Ошибку логируем, loop не роняем.
      console.error('[frame_loop]', e);
    }
    // Отрисовка освободила буфер: если сборка ждала его, самое время собрать.
    if (dirty) scheduleBuild();
  };

  const schedulePaint = () => {
    if (paintScheduled || stopped) return;
    paintScheduled = true;
    requestAnimationFrame(runPaint);
  };

  return {
    request() {
      if (stopped) return;
      dirty = true;
      scheduleBuild();
    },
    stop() {
      stopped = true;
    },
  };
}
