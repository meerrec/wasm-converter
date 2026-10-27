// Воркерный рендер-адаптер: painter, шрифты и цикл кадра живут здесь.
//
// Это путь, на котором `OffscreenCanvas` уехал в воркер: сборка DisplayList и
// отрисовка идут в одном потоке, транспорт отдаёт painter'у готовый кадр
// (`paintLocal`), а на main уезжает только `tick` со статистикой.

import { font_bytes, init_painter, resize_canvas } from '@meerrec/wasm';
import type { LocalBuildResult, TransportAdapter } from '../../execution/transport.js';
import type { PaintStats } from '../../protocol.js';
import { registerPaintFonts } from '../fonts.js';
import { startFrameLoop, type BuildOutcome, type FrameLoop } from '../frame_loop.js';
import type { RenderAdapter } from './render.js';

type Ctx = OffscreenCanvasRenderingContext2D;

/** Зависимости адаптера: всё, что зависит от формата, приходит замыканиями. */
export interface OffscreenRenderAdapterOptions {
  /** Транспорт кадра; на этом пути у него законны `buildLocal`/`paintLocal`. */
  transport: TransportAdapter;
  /**
   * Собрать кадр в буфер воркерного пула. Замыкание `WorkerCore`: оно знает
   * формат, последний запрос кадра и растёт под невлезший кадр.
   */
  build: () => LocalBuildResult;
  /** Куда отдать `tick`; `buildMs` адаптер добавляет сам. */
  onTick: (stats: PaintStats) => void;
  /**
   * Перерегистрировать битмапы после смены размеров холста. Установка
   * `canvas.width/height` сбрасывает 2D-контекст и кэш `ImageBitmap` painter'а,
   * поэтому картинки приходится возвращать в кэш.
   */
  onAfterResize: () => void;
}

/**
 * Рендер в воркере: `OffscreenCanvas`, локальный painter, двойной буфер.
 *
 * Цикл кадра (`frame_loop`) отделяет сборку от отрисовки: сборка идёт по
 * микрозадаче, отрисовка — по кадру экрана, и тайминг отрисовки не платит за
 * сборку DisplayList.
 */
export class OffscreenRenderAdapter implements RenderAdapter {
  readonly target = 'worker' as const;

  /** Контекст переданного холста; `null` до `start`. */
  private ctx: Ctx | null = null;
  /** Цикл кадра; `null` до `start` и после `stop`. */
  private loop: FrameLoop | null = null;

  /**
   * Сколько заняла сборка последнего кадра. Живёт здесь, а не в статистике
   * painter'а: `frame_loop` меряет только отрисовку, а сборку знает сборщик.
   */
  private buildMs = 0;

  constructor(private readonly opts: OffscreenRenderAdapterOptions) {}

  /** Холст, на котором рисует painter; `null`, пока `start` не прошёл. */
  get canvas(): OffscreenCanvas | null {
    return this.ctx?.canvas ?? null;
  }

  /**
   * Поднять painter и цикл кадра.
   *
   * Шрифты регистрируются до создания цикла и до первого кадра намеренно:
   * Chromium запоминает промах по конкретной строке `ctx.font` (семейство и
   * кегль) на всю сессию контекста. Если первый кадр успеет нарисоваться
   * подставленным семейством, текст останется на его метриках и после
   * регистрации сабсетов — предпросмотр разойдётся с PDF, куда встроены те же
   * сабсеты Carlito. Поэтому `await registerPaintFonts` стоит раньше
   * `init_painter`, а не «когда-нибудь при первом кадре».
   *
   * @param canvas — холст, уехавший из main через `transferControlToOffscreen`;
   *   без него воркерному пути нечем рисовать
   * @throws Error, если холста нет или у него нет 2D-контекста
   */
  async start(canvas: OffscreenCanvas | null): Promise<void> {
    if (canvas === null) {
      throw new Error('init: worker backend requires canvas');
    }
    await registerPaintFonts({ font_bytes });
    const ctx = canvas.getContext('2d', {
      alpha: true,
      desynchronized: true,
    }) as Ctx | null;
    if (ctx === null) throw new Error('getContext("2d") returned null');
    this.ctx = ctx;
    init_painter(ctx);
    this.loop = startFrameLoop({
      build: () => this.buildFrame(),
      paint: () => this.opts.transport.paintLocal(),
      // `frameId` назначает цикл, `buildMs` — адаптер: в `paint()` его взять
      // неоткуда, а хост ждёт обе цифры в одном `tick`.
      onTick: (stats) => this.opts.onTick({ ...stats, buildMs: this.buildMs }),
    });
  }

  /** Запросить кадр: сборка случится одна на микрозадачу. */
  request(): void {
    this.loop?.request();
  }

  /**
   * Пересчитать холст под DPR. Смена размеров сбрасывает состояние painter'а,
   * поэтому сразу зовём перерегистрацию битмапов.
   */
  resize(cssW: number, cssH: number, dpr: number): void {
    // До `start` painter'а нет: `resize` приходит из main раньше, чем воркер
    // успел поднять рендер, и трогать нечего.
    if (this.ctx === null) return;
    resize_canvas(cssW, cssH, dpr);
    this.opts.onAfterResize();
  }

  /**
   * Подтверждать нечего: на этом пути буфер не покидал воркер, `frame-ack`
   * сюда не приходит. Метод есть только ради общего контракта.
   */
  ack(_slot: number, _buffer: ArrayBuffer): void {
    // Намеренно пусто: буфер вернул painter в том же потоке.
  }

  /** Остановить цикл кадра; painter снимает владелец воркера. */
  stop(): void {
    this.loop?.stop();
    this.loop = null;
  }

  /**
   * Собрать кадр и запомнить время сборки.
   *
   * `stats === null` — собирать было нечего (документа ещё нет): это не ошибка,
   * а пустой ход, и `buildMs` тогда не трогаем.
   */
  private buildFrame(): BuildOutcome {
    const result = this.opts.build();
    if (result.stats !== null) this.buildMs = result.stats.buildMs;
    return result.outcome;
  }
}
