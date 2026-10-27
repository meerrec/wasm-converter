// SAB-транспорт: кадр собирается прямо в слот ring'а и там же рисуется.
//
// Ring живёт в `SharedArrayBuffer`, то есть в памяти, общей с main, но кадры
// через границу потока не едут: painter тоже в воркере. Поэтому у транспорта
// нет ни отправки, ни подтверждения — только сборка и отрисовка. Такой путь
// дешевле буферного: нет ни аллокаций, ни перевода владения, ни `frame-ack`.
//
// `SharedArrayBuffer` требует cross-origin isolation (COOP/COEP); недоступность
// ring'а — забота выбора плана (`resolveExecutionPlan`), а не этого класса.

import type { PaintStatsWire } from '../generated/wire.js';
import type { RenderRequest } from '../protocol.js';
import type { RelayBuildOutcome } from '../worker/relay_loop.js';
import { paintSab } from '../worker/wasm_typed.js';
import type { LocalBuildResult, TransportAdapter, TransportDeps } from './transport.js';

/**
 * Транспорт кадра через кольцо слотов в `SharedArrayBuffer`.
 *
 * Кадр пишется сразу в слот ring'а и читается painter'ом из того же ring'а:
 * копирования между сторонами нет. Если кадр не влез в слот, ring растёт, а
 * текущий кадр пропускается — догонять прокрутку устаревшим кадром незачем.
 */
export class SharedArrayBufferTransport implements TransportAdapter {
  readonly kind = 'sab' as const;

  /**
   * Текущий ring. Меняется на `grow`: старый массив остаётся валидным для
   * уже начатой отрисовки, а новые сборки идут в новый.
   */
  sab: SharedArrayBuffer;

  /** Ёмкость одного слота ring'а; растёт вместе с `sab`. */
  capacity: number;

  /** Зависимости сборки и выделения ring'а. */
  readonly deps: TransportDeps;

  /**
   * @param sab — ring, выделенный под `deps.slotCapacity`; его владение
   *   переходит транспорту
   * @param deps — сборка в слот и выделение нового ring'а при росте
   */
  constructor(sab: SharedArrayBuffer, deps: TransportDeps) {
    this.sab = sab;
    this.capacity = deps.slotCapacity;
    this.deps = deps;
  }

  /** Размер слота: кадр длиннее него не влезет и будет пропущен. */
  get slotCapacity(): number {
    return this.capacity;
  }

  /**
   * Собрать кадр в слот ring'а.
   *
   * `stats === null` — собирать нечего (нового запроса нет), это не пропуск.
   * `written: false` — кадр не уложился: ring растёт под его длину, а сам
   * кадр пропускается, потому что перерисовывать его после роста значит
   * показывать устаревшее состояние.
   */
  buildLocal(req: RenderRequest): LocalBuildResult {
    const stats = this.deps.buildSab(this.sab, this.capacity, req);
    if (stats === null) return { outcome: 'idle', stats: null };
    if (stats.written) return { outcome: 'built', stats };
    if (stats.bytes > this.capacity) this.grow(stats.bytes);
    return { outcome: 'dropped', stats };
  }

  /** Нарисовать самый свежий слот ring'а: painter читает его на месте. */
  paintLocal(): PaintStatsWire {
    return paintSab(this.sab, this.capacity);
  }

  /**
   * Отправки на main у SAB-пути нет: offscreen-painter живёт в воркере, и
   * кадру незачем пересекать границу потока.
   *
   * @throws Error всегда: вызов означает, что план и транспорт разошлись
   */
  buildForward(): RelayBuildOutcome {
    throw new Error(
      'SAB-транспорт не отправляет кадры на main: в offscreen-режиме painter живёт в воркере',
    );
  }

  /** Подтверждать нечего: буфер ring'а не покидал воркер. */
  ack(_slot: number, _buffer: ArrayBuffer): void {
    // Намеренно пусто: у SAB-пути нет буферов в полёте.
  }

  /**
   * Вырасти под кадр длиной `bytes`.
   *
   * Новый ring выделяется до записи кадра. Если памяти не нашлось, остаёмся на
   * старом ring'е: лучше пропустить кадр, чем уронить просмотрщик — следующий
   * кадр может оказаться меньше и влезть.
   */
  grow(bytes: number): void {
    const capacity = Math.max(bytes, this.capacity * 2);
    const grown = this.deps.allocSab(capacity);
    // Памяти не нашлось — остаёмся на старом ring'е: лучше пропустить кадр,
    // чем уронить просмотрщик, а следующий кадр может оказаться меньше.
    if (grown === null) return;
    this.capacity = capacity;
    this.sab = grown;
  }

  /** Закрывать нечего: ring принадлежит воркеру и живёт до его завершения. */
  close(): void {
    // Намеренно пусто: освобождать нечего, GC заберёт ring с транспортом.
  }
}
