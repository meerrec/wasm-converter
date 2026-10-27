// Контракт транспорта кадра: буферы, сборка и доставка DisplayList.
//
// До рефакторинга `WorkerCore` держал `sab`/`pool`/`relay` и ветвился по
// `transport === 'bytes'`. Теперь различие живёт в двух реализациях
// (`SharedArrayBufferTransport` и `ArrayBufferTransport`), а ядро знает только
// этот интерфейс. Сборка формата остаётся снаружи: транспорт принимает
// `buildInto`/`buildSab` замыканиями и не знает про XLSX/DOCX/PPTX.

import type { BuildStatsWire, PaintStatsWire } from '../generated/wire.js';
import type { FrameTransport, RenderRequest } from '../protocol.js';
import type { RelayBuildOutcome } from '../worker/relay_loop.js';
import type { RenderTarget } from './plan.js';

/** Результат локальной сборки: кадр в буфере/SAB и статистика. */
export interface LocalBuildResult {
  /** Что вышло из попытки собрать кадр. */
  readonly outcome: 'built' | 'dropped' | 'idle';
  /** Статистика сборки; `null` — собирать было нечего. */
  readonly stats: BuildStatsWire | null;
}

/**
 * Транспорт кадра на стороне воркера.
 *
 * `buildLocal`/`paintLocal` обслуживают offscreen-путь (painter в воркере),
 * `buildForward`/`ack` — main-путь (кадр уезжает на main). Метод чужого пути
 * бросает ошибку: неверный план должен падать громко, а не слать кадр не туда.
 */
export interface TransportAdapter {
  /** Каким путём едут кадры. */
  readonly kind: FrameTransport;
  /** Размер слота/буфера: кадр длиннее него не влезет. */
  readonly slotCapacity: number;
  /** Собрать кадр под локальный painter. */
  buildLocal(req: RenderRequest): LocalBuildResult;
  /** Собрать кадр для отправки на main. */
  buildForward(req: RenderRequest): RelayBuildOutcome;
  /** Нарисовать самый свежий локальный кадр. */
  paintLocal(): PaintStatsWire;
  /** Вернуть буфер после отрисовки на main. */
  ack(slot: number, buffer: ArrayBuffer): void;
  /** Вырасти под кадр длиной `bytes`. */
  grow(bytes: number): void;
  /** Освободить ресурсы транспорта. */
  close(): void;
}

/** Зависимости, общие для обеих реализаций. */
export interface TransportDeps {
  /** Начальная ёмкость слота/буфера. */
  readonly slotCapacity: number;
  /** Куда собирается кадр: от этого зависит, какой метод транспорта законен. */
  readonly target: RenderTarget;
  /** Выделенный ring; обязателен для SAB-транспорта. */
  readonly sab: SharedArrayBuffer | null;
  /** Выделить новый ring под выросший кадр; `null` — памяти не нашлось. */
  readonly allocSab: (bytes: number) => SharedArrayBuffer | null;
  /** Сборка в готовый буфер вызывающего. */
  readonly buildInto: (out: Uint8Array, req: RenderRequest) => BuildStatsWire | null;
  /** Сборка прямо в слот ring'а. */
  readonly buildSab: (
    sab: SharedArrayBuffer,
    slotCapacity: number,
    req: RenderRequest,
  ) => BuildStatsWire | null;
}

/** Создать транспорт по выбранному плану. */
export function createTransportAdapter(
  kind: FrameTransport,
  deps: TransportDeps,
): TransportAdapter {
  if (kind === 'sab') {
    if (deps.sab === null) {
      throw new Error('SAB-транспорт без ring: план и выделение разошлись');
    }
    return new SharedArrayBufferTransport(deps.sab, deps);
  }
  return new ArrayBufferTransport(deps);
}

// Импорты вниз: классы типизируются этим модулем, поэтому цикл типов
// разрывается `import type` в их файлах.
import { SharedArrayBufferTransport } from './transport_sab.js';
import { ArrayBufferTransport } from './transport_array_buffer.js';
