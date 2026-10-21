import type { FrameMsg, InMsg, PaintStats } from '../protocol.js';
import { paintDisplayList } from './main_painter.js';

export interface MainFrameChain {
  /** Принять кадр из воркера. */
  handle(frame: FrameMsg): void;
  /** Остановить цикл и вернуть вытесненный буфер воркеру. */
  destroy(): void;
}

/**
 * Цепочка кадра main-режима: последний `FrameMsg`, одна отрисовка на кадр
 * экрана, `frame-ack` после отрисовки.
 *
 * Пока main рисует кадр, воркер успевает собрать следующий — буферов два.
 * Кадр, вытесненный более свежим до отрисовки, не рисуется вовсе, а его буфер
 * возвращается сразу: воркер ждёт ack, чтобы собрать следующий кадр, и держать
 * его до `requestAnimationFrame` значило бы простаивать.
 */
export function createMainFrameChain(
  worker: Pick<Worker, 'postMessage'>,
  onTick: (stats: PaintStats) => void,
): MainFrameChain {
  /** Самый свежий неотрисованный кадр. */
  let pending: FrameMsg | null = null;
  let raf = 0;
  let destroyed = false;

  const ack = (frame: FrameMsg): void => {
    // Слот называет буфер: его идентичность через `postMessage` не переживает,
    // а по номеру воркер кладёт память обратно в тот же слот релея.
    worker.postMessage(
      { type: 'frame-ack', slot: frame.slot, buffer: frame.bytes } satisfies InMsg,
      [frame.bytes],
    );
  };

  const paint = (): void => {
    raf = 0;
    const frame = pending;
    pending = null;
    if (!frame) return;
    // Ошибка painter'а не роняет цикл: буфер всё равно возвращается, и
    // следующий кадр попробует нарисоваться.
    let stats = { cmds: 0, dropped: true, paintMs: 0 };
    try {
      stats = paintDisplayList(new Uint8Array(frame.bytes, 0, frame.len));
    } catch (e) {
      console.error('[main_frame_chain] paint', e);
    }
    onTick({
      cmds: stats.cmds,
      dropped: stats.dropped || frame.dropped,
      paintMs: stats.paintMs,
      frameId: frame.frameId,
      buildMs: frame.buildMs,
    });
    ack(frame);
  };

  return {
    handle(frame) {
      if (destroyed) {
        // Воркер уже останавливают, но ack безопасен и не даёт ему ждать
        // буфер, который сюда всё же успел прийти.
        ack(frame);
        return;
      }
      if (pending) ack(pending);
      pending = frame;
      if (raf === 0) raf = requestAnimationFrame(paint);
    },
    destroy() {
      destroyed = true;
      if (raf !== 0) cancelAnimationFrame(raf);
      raf = 0;
      if (pending) {
        ack(pending);
        pending = null;
      }
    },
  };
}
