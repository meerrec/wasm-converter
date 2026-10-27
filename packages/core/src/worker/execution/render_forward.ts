// Рендер-адаптер main-режима: воркер только собирает кадр и отправляет его.
//
// Здесь нет ни painter'а, ни `requestAnimationFrame`: painter живёт на main,
// которому кадр уезжает переводом владения. Обратный путь — `frame-ack`: он
// возвращает буфер в релей, иначе оба слота остались бы занятыми навсегда.

import type { TransportAdapter } from '../../execution/transport.js';
import type { FrameMsg } from '../../protocol.js';
import { startRelayLoop, type RelayBuildOutcome, type RelayLoop } from '../relay_loop.js';
import type { RenderAdapter } from './render.js';

/** Зависимости адаптера: сборка и отправка приходят замыканиями `WorkerCore`. */
export interface ForwardingRenderAdapterOptions {
  /** Транспорт кадра; на этом пути у него законны `buildForward`/`ack`. */
  transport: TransportAdapter;
  /**
   * Собрать кадр в свободный буфер релея. `busy` — оба буфера в полёте, `retry`
   * — кадр не влез и ёмкость уже выросла.
   */
  build: () => RelayBuildOutcome;
  /** Отправить кадр на main переводом владения буфером. */
  send: (frame: FrameMsg) => void;
}

/**
 * Main-режим: кадр собирается в воркере, а рисуется на main.
 *
 * Сборка идёт по микрозадаче, отправка — сразу после неё, а повторный запрос
 * ждёт `frame-ack` от main: пока оба буфера в полёте, новые кадры пропускаются.
 * Отдельный цикл (`relay_loop`), а не `frame_loop`, потому что в браузерах без
 * `OffscreenCanvas` worker-`requestAnimationFrame` может отсутствовать вовсе.
 */
export class ForwardingRenderAdapter implements RenderAdapter {
  readonly target = 'main' as const;

  /** Цикл отправки; `null` до `start` и после `stop`. */
  private loop: RelayLoop | null = null;

  constructor(private readonly opts: ForwardingRenderAdapterOptions) {}

  /**
   * Холста у воркера нет: он остался на main, и painter тоже там.
   */
  get canvas(): OffscreenCanvas | null {
    return null;
  }

  /**
   * Запустить цикл отправки.
   *
   * @param _canvas — не используется: в main-режиме холст не покидает страницу
   */
  async start(_canvas: OffscreenCanvas | null): Promise<void> {
    this.loop = startRelayLoop({
      build: this.opts.build,
      send: this.opts.send,
    });
  }

  /** Запросить кадр: сборка случится одна на микрозадачу. */
  request(): void {
    this.loop?.request();
  }

  /** Размеры холста ставит main, воркеру о них знать нечего. */
  resize(_cssW: number, _cssH: number, _dpr: number): void {
    // Намеренно пусто: смена размеров не меняет ни сборку, ни отправку.
    // Кадр при этом пересоберётся сам — запрос на него пришлёт main.
  }

  /**
   * Main вернул буфер кадра: слот снова свободен.
   *
   * Сначала транспорт кладёт буфер в свой слот по номеру, потом цикл пробует
   * собрать кадр, который ждал свободного буфера. Порядок важен: `ack` цикла,
   * вызванный раньше `transport.ack`, упёрся бы в занятый слот.
   */
  ack(slot: number, buffer: ArrayBuffer): void {
    this.opts.transport.ack(slot, buffer);
    this.loop?.ack();
  }

  /** Остановить цикл: новых кадров не будет. */
  stop(): void {
    this.loop?.stop();
    this.loop = null;
  }
}
