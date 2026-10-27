// Буферный транспорт: кадр едет `ArrayBuffer`'ом, painter — на main.
//
// Этот путь включается там, где `SharedArrayBuffer` недоступен или холст не
// уехал в воркер (main-режим). Он же обслуживает offscreen-путь без SAB: тогда
// буферы остаются в воркере, и владение ими переходит только от сборки к
// painter'у и обратно.
//
// У пути две несовместимые роли, поэтому транспорт один, а методы разные:
// `buildLocal`/`paintLocal` — воркер рисует сам (`FramePool`), `buildForward`/
// `ack` — кадр уезжает на main (`FrameRelay`). Метод чужой роли бросает: план
// и транспорт обязаны совпадать.

import type { PaintStatsWire } from '../generated/wire.js';
import type { RenderRequest } from '../protocol.js';
import { FramePool } from '../worker/frame_pool.js';
import { FrameRelay } from '../worker/frame_relay.js';
import type { RelayBuildOutcome } from '../worker/relay_loop.js';
import { paintBytes } from '../worker/wasm_typed.js';
import type { RenderTarget } from './plan.js';
import type { LocalBuildResult, TransportAdapter, TransportDeps } from './transport.js';

/**
 * Транспорт кадра двумя `ArrayBuffer`'ами.
 *
 * Буферы не аллоцируются на кадр: их ровно два, и они переиспользуются. На
 * offscreen-пути (`target: 'worker'`) это `FramePool`, и владение буфером
 * возвращается после отрисовки в том же потоке. На main-пути
 * (`target: 'main'`) это `FrameRelay`: буфер уходит на main переводом
 * владения и возвращается только `frame-ack`'ом, поэтому оба занятых буфера —
 * это backpressure, а не повод ждать: кадр пропускается, следующий запрос
 * собирается, когда main вернёт буфер.
 */
export class ArrayBufferTransport implements TransportAdapter {
  readonly kind = 'bytes' as const;

  private readonly deps: TransportDeps;
  private readonly target: RenderTarget;

  /** Исходная ёмкость: нужна, только если ни пула, ни релея ещё нет. */
  private readonly capacity: number;

  /** Буферы offscreen-пути; `null` на main-пути. */
  private readonly pool: FramePool | null;

  /** Буферы main-пути; `null` на offscreen-пути. */
  private readonly relay: FrameRelay | null;

  /**
   * @param deps — сборка в готовый буфер и выбранный план; по `target`
   *   решается, какая из двух ролей живёт в этом экземпляре
   */
  constructor(deps: TransportDeps) {
    this.deps = deps;
    this.target = deps.target;
    this.capacity = deps.slotCapacity;
    if (this.target === 'worker') {
      this.pool = new FramePool(this.capacity);
      this.relay = null;
    } else {
      this.relay = new FrameRelay(this.capacity);
      this.pool = null;
    }
  }

  /** Размер буфера: кадр длиннее него не влезет. */
  get slotCapacity(): number {
    return this.pool?.slotCapacity ?? this.relay?.slotCapacity ?? this.capacity;
  }

  /**
   * Собрать кадр в буфер воркерного пула.
   *
   * `written: false` — кадр не влез: буфер возвращается в пул, а пул растёт.
   * Рост идёт после `abortBuild`, чтобы маленький буфер не остался помечен
   * готовым с обрезанным кадром.
   *
   * @throws Error, если план собран на main: там локального painter'а нет
   */
  buildLocal(req: RenderRequest): LocalBuildResult {
    if (this.target !== 'worker' || this.pool === null) {
      throw new Error(
        'локальная сборка недоступна main-транспорту: кадр собирается для отправки на main',
      );
    }
    const pool = this.pool;
    const out = pool.beginBuild();
    const stats = this.deps.buildInto(out, req);
    if (stats === null) {
      pool.abortBuild();
      return { outcome: 'idle', stats: null };
    }
    if (stats.written) {
      pool.commitBuild(stats.bytes);
      return { outcome: 'built', stats };
    }
    pool.abortBuild();
    if (stats.bytes > this.slotCapacity) this.grow(stats.bytes);
    return { outcome: 'dropped', stats };
  }

  /**
   * Собрать кадр в буфер релея и забрать его под отправку на main.
   *
   * `busy` не ошибка: оба буфера в полёте, буфер освободит `frame-ack`.
   * `retry` — кадр не влез и релей уже вырос: цикл повторит сборку сразу,
   * не дожидаясь следующего запроса. В обоих случаях буфер возвращается в
   * пул, иначе после пропуска он остался бы занятым навсегда.
   *
   * @throws Error, если план собран на воркере: отправлять кадр некуда
   */
  buildForward(req: RenderRequest): RelayBuildOutcome {
    if (this.target !== 'main' || this.relay === null) {
      throw new Error(
        'отправка кадра недоступна воркерному транспорту: painter живёт в воркере и читает буфер на месте',
      );
    }
    const relay = this.relay;
    const out = relay.beginBuild();
    if (out === null) return { kind: 'busy' };
    const stats = this.deps.buildInto(out, req);
    if (stats === null) {
      relay.abort();
      return { kind: 'idle' };
    }
    if (!stats.written) {
      relay.abort();
      // Рост — причина повторить сразу: та же сборка в новый буфер влезет.
      if (stats.bytes > relay.slotCapacity) {
        relay.grow(stats.bytes);
        return { kind: 'retry' };
      }
      return { kind: 'busy' };
    }
    relay.commit(stats.bytes);
    const frame = relay.takeFrame();
    if (frame === null) return { kind: 'busy' };
    return {
      kind: 'frame',
      frame: {
        slot: frame.slot,
        bytes: frame.bytes,
        len: frame.len,
        cmds: stats.cmds,
        buildMs: stats.buildMs,
      },
    };
  }

  /**
   * Нарисовать самый свежий кадр пула.
   *
   * Пустой пул — не ошибка: painter мог опередить сборку, тогда кадр просто
   * помечается пропущенным, и `tick` это покажет.
   *
   * @throws Error, если план собран на main: painter'а в воркере нет
   */
  paintLocal(): PaintStatsWire {
    if (this.target !== 'worker' || this.pool === null) {
      throw new Error(
        'локальная отрисовка недоступна main-транспорту: кадр рисует painter на main',
      );
    }
    const frame = this.pool.takeLatest();
    if (frame === null) return { cmds: 0, dropped: true, paintMs: 0 };
    return paintBytes(frame);
  }

  /**
   * Вернуть буфер, пришедший с main, в релей.
   *
   * На воркерном пути подтверждать нечего: буфер не покидал поток.
   */
  ack(slot: number, buffer: ArrayBuffer): void {
    if (this.target !== 'main') return;
    this.relay?.adopt(slot, buffer);
  }

  /** Вырасти под кадр длиной `bytes`: пул и релей растят свои буферы сами. */
  grow(bytes: number): void {
    this.pool?.grow(bytes);
    this.relay?.grow(bytes);
  }

  /**
   * Закрывать нечего: буферы живут до `terminate` воркера.
   *
   * Ссылки на пул и релей остаются намеренно: `close` может прийти между
   * сборкой и отрисовкой, и обнуление ссылок лишь превратило бы штатный путь
   * в гонку. Память освободит завершение воркера.
   */
  close(): void {
    // Намеренно пусто: см. JSDoc.
  }
}
