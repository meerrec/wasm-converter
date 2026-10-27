// Main-сторонний адаптер рендера: единственная точка, где просмотрщик
// разговаривает с путём кадра.
//
// Режим выбирается один раз на `init` и дальше не меняется:
// - `worker` — холст уехал в воркер, painter и шрифты живут там; на main
//   остаётся только `resize`, который надо переслать воркеру;
// - `main` — painter поднимается здесь (`main_painter`), кадры приходят
//   сообщением `frame`, рисуются раз в кадр экрана и возвращаются `frame-ack`.
//
// `viewer_types` этот модуль не импортирует: контракт описан здесь, чтобы не
// заводить цикл `viewer_types → render_adapter → viewer_types`.

import type { FrameMsg, InMsg, PaintStats } from '../protocol.js';
import { exportPng } from './export_png.js';
import { createMainFrameChain, type MainFrameChain } from './main_frame_chain.js';
import {
  disposeMainPainter,
  dropMainBitmap,
  initMainPainter,
  registerMainBitmap,
  resizeMainCanvas,
} from './main_painter.js';

/**
 * Рендер-адаптер на стороне страницы.
 *
 * Методы, которые в выбранном режиме нечего делать, — намеренные no-op:
 * хост не должен знать, куда поехал кадр, и не должен ветвиться по режиму.
 */
export interface MainRenderAdapter {
  /** Где рисуется кадр: этот же режим выбран в `execution/plan`. */
  readonly mode: 'worker' | 'main';
  /**
   * Подготовить рендер. В воркер-режиме делать нечего; в main-режиме здесь
   * поднимается painter и цикл отрисовки.
   *
   * @throws Error, если у холста нет 2D-контекста или painter уже занят
   */
  init(): Promise<void>;
  /** Сменить размеры холста: воркеру — сообщением, main — локально. */
  resize(cssW: number, cssH: number, dpr: number): void;
  /** Принять готовый кадр воркера (main-режим). */
  handleFrame(frame: FrameMsg): void;
  /** Вернуть `ImageBitmap` (main-режим; в воркере картинки живут на месте). */
  registerBitmap(id: number, bitmap: ImageBitmap): void;
  /** Снять один битмап с painter'а. */
  dropBitmap(id: number): void;
  /** Снять весь набор: воркер при смене документа шлёт один `clear-bitmaps`. */
  clearBitmaps(): void;
  /**
   * Снимок холста в PNG, если он делается локально, иначе `null` — тогда
   * снимок просит воркер (`export-png`).
   */
  capturePng(): Promise<Uint8Array> | null;
  /** Остановить рендер и освободить painter'а (main-режим). */
  destroy(): void;
}

/** Вход адаптера: всё, что нужно обоим режимам. */
export interface MainRenderAdapterOptions {
  /** Решение плана: где рисуется кадр. */
  mode: 'worker' | 'main';
  /** Холст просмотрщика; в main-режиме painter рисует в него. */
  canvas: HTMLCanvasElement;
  /** Во что уходят сообщения: `init` не здесь, а у ядра просмотрщика. */
  worker: Pick<Worker, 'postMessage'>;
  /** URL wasm-бинаря для main-режима; без него wasm-bindgen ищет рядом. */
  wasmUrl?: string;
  /** `tick` кадра; в воркер-режиме их присылает воркер. */
  onTick: (stats: PaintStats) => void;
}

/**
 * Создать адаптер под выбранный режим.
 *
 * @param opts — режим, холст, воркер и обработчик `tick`
 * @returns адаптер; `destroy` обязателен, иначе singleton painter'а в
 *   main-режиме не освободится и следующий вьюер не поднимется
 */
export function createMainRenderAdapter(opts: MainRenderAdapterOptions): MainRenderAdapter {
  return opts.mode === 'worker' ? createWorkerAdapter(opts) : createMainAdapter(opts);
}

/**
 * Воркер-режим: кадр и painter уехали в воркер.
 *
 * Всё, что связано с отрисовкой и картинками, здесь — no-op: этим занимается
 * воркер, а main только пересылает `resize` (холст у него уже offscreen).
 */
function createWorkerAdapter(opts: MainRenderAdapterOptions): MainRenderAdapter {
  return {
    mode: 'worker',
    async init() {
      // Инициализацию воркера ведёт `viewer_core` своим `init`; painter
      // поднимается внутри воркера, на странице готовить нечего.
    },
    resize(cssW, cssH, dpr) {
      // Размеры ставит воркер: только он знает про DPR и свой painter.
      opts.worker.postMessage({ type: 'resize', cssW, cssH, dpr } satisfies InMsg);
    },
    handleFrame() {
      // Кадры воркер рисует сам, на main они не приходят.
    },
    registerBitmap() {
      // Битмапы регистрируются в painter'е воркера.
    },
    dropBitmap() {
      // См. registerBitmap.
    },
    clearBitmaps() {
      // См. registerBitmap.
    },
    capturePng() {
      // Снимок делает воркер по `export-png`: холст у него.
      return null;
    },
    destroy() {
      // Освобождать нечего: painter и его ресурсы живут в воркере.
    },
  };
}

/**
 * Main-режим: painter на странице, кадр приходит из воркера буфером.
 *
 * Битмапы хранятся здесь, потому что смена размеров холста сбрасывает кэш
 * `ImageBitmap` painter'а: после `resize` их надо вернуть в кэш заново, а
 * источник правды о том, какие картинки сейчас в документе, — этот реестр.
 */
function createMainAdapter(opts: MainRenderAdapterOptions): MainRenderAdapter {
  const canvas = opts.canvas;
  /**
   * Контекст — признак живого painter'а: до `init` и после `destroy` его нет.
   * Установка размеров и регистрация битмапов до painter'а упали бы в wasm.
   */
  let ctx: CanvasRenderingContext2D | null = null;
  /** Цепочка кадра: последний кадр, одна отрисовка на кадр экрана, `frame-ack`. */
  let chain: MainFrameChain | null = null;
  const bitmaps = new Map<number, ImageBitmap>();

  const register = (id: number, bitmap: ImageBitmap): void => {
    bitmaps.set(id, bitmap);
    registerMainBitmap(id, bitmap);
  };

  const drop = (id: number): void => {
    bitmaps.delete(id);
    dropMainBitmap(id);
  };

  const clear = (): void => {
    // Сначала снимаем с painter'а, потом чистим реестр: ключи нужны для
    // поштучного `dropMainBitmap`, а `clear()` по живому итератору соврал бы.
    for (const id of bitmaps.keys()) dropMainBitmap(id);
    bitmaps.clear();
  };

  return {
    mode: 'main',
    async init() {
      const context = canvas.getContext('2d');
      if (context === null) throw new Error('getContext("2d") returned null');
      // `exactOptionalPropertyTypes` не даёт передать `undefined` в
      // необязательный параметр, а звать с явным URL нужно только когда он есть.
      if (opts.wasmUrl !== undefined) {
        await initMainPainter(context, opts.wasmUrl);
      } else {
        await initMainPainter(context);
      }
      ctx = context;
      chain = createMainFrameChain(opts.worker, opts.onTick);
    },
    resize(cssW, cssH, dpr) {
      // Установка размеров — до painter'а: это свойство холста, а не wasm, и
      // промах по DPR чинится немедленно. Ниже — только то, что требует
      // поднятого painter'а.
      canvas.width = Math.max(1, Math.round(cssW * dpr));
      canvas.height = Math.max(1, Math.round(cssH * dpr));
      if (ctx === null) return;
      resizeMainCanvas(cssW, cssH, dpr);
      // Смена размеров сбросила кэш `ImageBitmap`: возвращаем картинки в кэш,
      // иначе следующий кадр нарисует их подстановкой.
      for (const [id, bitmap] of bitmaps) registerMainBitmap(id, bitmap);
    },
    handleFrame(frame) {
      // До `init` и после `destroy` цепочки нет: кадр пришёл слишком рано или
      // уже поздно, и подтвердить его некому — воркер остановлен.
      chain?.handle(frame);
    },
    registerBitmap(id, bitmap) {
      if (ctx === null) return;
      register(id, bitmap);
    },
    dropBitmap(id) {
      if (ctx === null) {
        bitmaps.delete(id);
        return;
      }
      drop(id);
    },
    clearBitmaps() {
      if (ctx === null) {
        bitmaps.clear();
        return;
      }
      clear();
    },
    capturePng() {
      // Снимок локальный: холст и painter здесь, гонять PNG через воркер
      // значило бы копировать байты лишний раз.
      return exportPng(canvas);
    },
    destroy() {
      // Сначала цикл: он вернёт воркеру вытесненный буфер, пока тот ещё жив.
      chain?.destroy();
      chain = null;
      if (ctx !== null) {
        clear();
        // Painter — singleton страницы: без этого следующий вьюер не поднимется.
        // Снимаем его, только если подняли сами: неудачный `init` (второй
        // вьюер в main-режиме) не должен гасить painter соседа.
        disposeMainPainter();
      }
      // Без живого painter'а снимать нечего, но реестр не должен удерживать
      // битмапы после destroy.
      bitmaps.clear();
      ctx = null;
    },
  };
}
