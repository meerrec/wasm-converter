// Общий жизненный цикл просмотрщиков XLSX/DOCX/PPTX.
//
// Разметка, воркер, транспорт кадров, адресные запросы, зум и прокрутка у всех
// трёх форматов совпадают до строки: отличаются только раскладка (лист, стопка
// страниц, слайд), теги сообщений и форматные операции. Ядро держит общую часть,
// фабрики форматов описывают себя через `ViewerFormatSpec` (см. `viewer_types.ts`).

export type {
  TextExportFormat, ViewerBaseOptions, ViewerFormatSpec, ViewerCoreHooks, ViewerSpec,
  ViewerState, ViewerBaseHandle,
} from './viewer_types.js';

import { DEFAULT_SLOT_CAPACITY } from '../protocol.js';
import type {
  DocFormat,
  InMsg,
  InitMsg,
  OutMsg,
  PaintStats,
  PdfOptions,
  ReadyInfo,
  TextLayer,
} from '../protocol.js';
import { armAbort, type AbortGuard } from './abort.js';
import { toDiagnostic, ViewerError, workerDiagnostic, type Diagnostic } from '../diagnostics.js';
import { createMainRenderAdapter } from './render_adapter.js';
import { resolveRenderMode } from './offscreen.js';
import { pumpStream, type PumpOptions } from './open_stream.js';
import { computeDpr } from './dpr.js';
import { endSpan, setSpanAttribute, startSpan, type SpanAttributes } from '../telemetry.js';
import type {
  TextExportFormat,
  ViewerBaseHandle,
  ViewerBaseOptions,
  ViewerSpec,
  ViewerState,
} from './viewer_types.js';

/** Атрибут спана открытия: число листов, страниц или слайдов. */
function openedCountKey(format: DocFormat): string {
  return format === 'xlsx' ? 'sheetCount' : format === 'docx' ? 'pageCount' : 'slideCount';
}

/** Число единиц документа в ответе на открытие — для атрибута спана. */
function openedCount(msg: OutMsg): number {
  switch (msg.type) {
    case 'opened':
      return msg.sheets.length;
    case 'docx-opened':
      return msg.pages.length;
    case 'pptx-opened':
      return msg.slides.length;
    default:
      return 0;
  }
}

/** Разрешить адресный запрос: чей id вернул воркер, тот и получает ответ. */
function answer<T>(map: Map<number, { resolve: (value: T) => void }>, id: number, value: T): void {
  map.get(id)?.resolve(value);
  map.delete(id);
}

/** Адресно отклонить запрос; остальные ждут своих ответов. */
function rejectPending<T extends { reject: (e: Error) => void }>(map: Map<number, T>, id: number, error: Error): void {
  const pending = map.get(id);
  if (pending) {
    map.delete(id);
    pending.reject(error);
  }
}

/** Отклонить всё ожидающее: ошибка без id — сбой воркера целиком. */
function rejectAll<T extends { reject: (e: Error) => void }>(map: Map<number, T>, error: Error): void {
  for (const pending of map.values()) pending.reject(error);
  map.clear();
}

/**
 * Собрать общий каркас просмотрщика: прокручиваемую область, распорку и холст,
 * приклеенный к видимой части трансформом, затем воркер, `ready`, адресные
 * запросы и раскладку. Форматные отличия приходят через `spec`.
 */
export async function createViewerBase<S, O extends ViewerBaseOptions>(
  host: HTMLElement,
  spec: ViewerSpec<S, O>,
  options: O,
): Promise<ViewerBaseHandle<S, O>> {
  const scroller = document.createElement('div');
  scroller.style.cssText = 'position:relative;overflow:auto;width:100%;height:100%';
  const spacer = document.createElement('div');
  spacer.style.cssText = 'position:absolute;top:0;left:0;width:0;height:0';
  const canvas = document.createElement('canvas');
  canvas.style.cssText =
    'position:absolute;top:0;left:0;display:block;transform:translate(0px,0px)';
  scroller.append(spacer, canvas);
  if (options.a11y === true) canvas.setAttribute('aria-hidden', 'true');
  host.append(scroller);

  // Режим решает, уедет ли холст в воркер: проверяем ровно ту возможность, которой пользуемся, — `OffscreenCanvas` нужен только воркер-пути.
  const renderMode = resolveRenderMode(canvas, options.renderMode, options.transport);

  // Строка из опций уходит в `Worker` как есть; приведение — лишь потому, что описание формата сузило тип до `URL`.
  const workerUrl = spec.workerUrl(
    options.workerUrl === undefined ? {} : { workerUrl: options.workerUrl as unknown as URL },
  );
  const worker = new Worker(workerUrl, { type: 'module', name: spec.workerName });

  const slotCapacity = options.slotCapacity ?? DEFAULT_SLOT_CAPACITY;
  // main-сторонний путь кадра: в воркер-режиме адаптер тонкий (только resize),
  // в main-режиме он держит painter, цикл кадра и битмапы. Ошибка init не
  // оставляет воркер висеть, а singleton painter'а освобождается.
  const tickHandlers = new Set<(stats: PaintStats) => void>();
  const renderer = createMainRenderAdapter({
    mode: renderMode,
    canvas,
    worker,
    ...(options.wasmUrl === undefined ? {} : { wasmUrl: options.wasmUrl }),
    onTick: (stats) => {
      for (const handler of tickHandlers) handler(stats);
    },
  });
  try {
    await renderer.init();
  } catch (e) {
    worker.terminate();
    renderer.destroy();
    throw e;
  }

  const state: ViewerState<S, O> = {
    host,
    options,
    scroller,
    spacer,
    canvas,
    worker,
    renderMode,
    transport: 'bytes',
    data: spec.createState(options),
    zoom: options.zoom ?? 1,
    dpr: computeDpr(scroller.clientWidth, scroller.clientHeight, devicePixelRatio),
    padLeft: 0,
    destroyed: false,
    frameScheduled: false,
    render: renderer,
    tickHandlers,
    pendingOpen: new Map(),
    pendingHit: new Map(),
    pendingHyperlink: new Map(),
    pendingPng: new Map(),
    pendingPdf: new Map(),
    pendingText: new Map(),
    pendingTextLayer: new Map(),
    nextRequestId: 1,
    textLayer: null,
    openedCount: 0,
    disposers: new Set(),
    applyLayout: () => spec.applyLayout(state),
    schedule,
    placeCanvas,
    refresh: () => { placeCanvas(); schedule(); },
    requestTextLayer,
  };
  spec.extraInit?.(state, canvas, scroller);

  // Карты запросов, которым воркер отвечает ошибкой: адресной по id или общей.
  const pendingRejectables: Array<Map<number, { reject: (e: Error) => void }>> = [
    state.pendingOpen,
    state.pendingPng,
    state.pendingPdf,
    state.pendingText,
    state.pendingTextLayer,
  ];

  worker.addEventListener('message', (ev: MessageEvent<OutMsg>) => {
    const msg = ev.data;
    // Форматное сообщение идёт первым: ядро не трогает то, что формат уже обработал.
    if (spec.extraMsg?.(state, msg) === true) return;
    switch (msg.type) {
      case 'tick':
        for (const handler of state.tickHandlers) handler(msg.stats);
        break;
      case 'frame':
        // main-режим: воркер собрал DisplayList, адаптер рисует и возвращает буфер.
        state.render.handleFrame(msg);
        break;
      case 'bitmap':
        state.render.registerBitmap(msg.id, msg.bitmap);
        break;
      case 'drop-bitmap':
        state.render.dropBitmap(msg.id);
        break;
      case 'clear-bitmaps':
        state.render.clearBitmaps();
        break;
      case 'opened':
      case 'docx-opened':
      case 'pptx-opened':
        if (msg.type !== spec.openedResponse) break;
        state.openedCount = openedCount(msg);
        // Формат запоминает результат и раскладывает документ; разрешаем ровно тот промис, чей id эхом вернул воркер.
        spec.onOpened(state, msg);
        answer(state.pendingOpen, msg.id, undefined);
        break;
      case 'hit':
        answer(state.pendingHit, msg.id, msg.cell);
        break;
      case 'hyperlink':
        answer(state.pendingHyperlink, msg.id, msg.link);
        break;
      case 'png':
        answer(state.pendingPng, msg.id, msg.bytes);
        break;
      case 'pdf':
        answer(state.pendingPdf, msg.id, msg.bytes);
        break;
      case 'docx-text':
      case 'pptx-text':
        answer(state.pendingText, msg.id, msg.text);
        break;
      case 'text-layer':
        answer(state.pendingTextLayer, msg.id, msg.layer);
        break;
      case 'error': {
        // Ошибка воркера — уже диагностика: Rust/воркер могли прислать
        // структурный объект, иначе собираем его из кода стадии и текста.
        const diagnostic = msg.diagnostic ?? workerDiagnostic(msg, spec.formatTag);
        const error = new ViewerError(diagnostic);
        console.error(`[${spec.formatTag}]`, `${diagnostic.layer}/${diagnostic.code}: ${diagnostic.message}`);
        if (msg.id !== undefined) {
          // Ошибка адресная: отклоняем только свой запрос, соседние ждут своих ответов — их ошибка не касается.
          for (const map of pendingRejectables) rejectPending(map, msg.id, error);
          break;
        }
        // Ошибки без id — сбой воркера целиком (init): ответа не будет ни на один запрос — отклоняем всё ожидающее.
        for (const map of pendingRejectables) rejectAll(map, error);
        break;
      }
      default:
        break;
    }
  });

  let ready: ReadyInfo;
  try {
    ready = await new Promise<ReadyInfo>((resolve, reject) => {
      const onMessage = (ev: MessageEvent) => {
        const payload = ev.data as {
          type?: string;
          message?: string;
          diagnostic?: Diagnostic;
        } & Partial<ReadyInfo>;
        if (payload?.type === 'ready') {
          worker.removeEventListener('message', onMessage);
          resolve({
            backend: payload.backend ?? 'worker',
            transport: payload.transport ?? 'bytes',
            slotCapacity: payload.slotCapacity ?? slotCapacity,
            totalBytes: payload.totalBytes ?? 0,
            crossOriginIsolated: payload.crossOriginIsolated ?? false,
          });
        } else if (payload?.type === 'error') {
          worker.removeEventListener('message', onMessage);
          // Ошибка без `id` — воркер не поднялся. Это не отказ одного запроса,
          // поэтому в диагностике слой оркестрации, а не разбора.
          reject(
            new ViewerError(
              payload.diagnostic ??
                toDiagnostic(payload.message ?? 'worker init error', {
                  code: 'internal',
                  layer: 'orchestration',
                  format: spec.formatTag,
                }),
            ),
          );
        }
      };
      worker.addEventListener('message', onMessage);
      const init: InitMsg = {
        type: 'init',
        backend: renderMode,
        wasmUrl: options.wasmUrl,
        ...spec.initOptions?.(options),
        slotCapacity,
        transport: options.transport,
      };
      if (renderMode === 'worker') {
        // Холст уезжает воркеру ровно один раз; в main-режиме он остаётся здесь.
        const off = canvas.transferControlToOffscreen();
        worker.postMessage({ ...init, canvas: off } satisfies InMsg, [off]);
      } else {
        worker.postMessage(init);
      }
    });
  } catch (e) {
    // Воркер не поднялся — он не нужен, а painter в main-режиме singleton страницы: без освобождения следующий вьюер не смог бы инициализироваться.
    worker.terminate();
    state.render.destroy();
    throw e;
  }
  state.transport = ready.transport;

  function resizeCanvas(): void {
    const cssW = scroller.clientWidth;
    const cssH = scroller.clientHeight;
    if (cssW === 0 || cssH === 0) return;
    state.dpr = computeDpr(cssW, cssH, devicePixelRatio);
    canvas.style.width = `${cssW}px`;
    canvas.style.height = `${cssH}px`;
    // Режим решает адаптер: воркеру уходит resize, main сам ставит размеры и
    // возвращает битмапы в кэш painter'а.
    state.render.resize(cssW, cssH, state.dpr);
  }

  function render(): void {
    const message = spec.renderMsg(state);
    if (message) worker.postMessage(message);
  }

  function schedule(): void {
    if (state.frameScheduled || state.destroyed) return;
    state.frameScheduled = true;
    requestAnimationFrame(() => {
      state.frameScheduled = false;
      render();
      // Слой обновляется в том же rAF, что и кадр: окно виртуализации не должно отставать от canvas.
      spec.requestTextLayer?.(state, null);
    });
  }

  function placeCanvas(): void {
    canvas.style.transform = `translate(${scroller.scrollLeft}px, ${scroller.scrollTop}px)`;
  }

  const onScroll = (): void => {
    placeCanvas();
    schedule();
  };
  scroller.addEventListener('scroll', onScroll, { passive: true });

  const observer = new ResizeObserver(() => {
    resizeCanvas();
    // Ширина области прокрутки — база поля слева: после изменения размера документ надо переставить, даже если прокрутка не сдвинулась.
    spec.applyLayout(state);
    placeCanvas();
    schedule();
  });
  observer.observe(scroller);

  resizeCanvas();

  /** Запрос с отменой (ADR-0032): промис ответа плюс отправка сообщения. */
  function request<T>(opts: { signal?: AbortSignal } | undefined, build: (id: number) => InMsg, pending: Map<number, AbortGuard<T>>): Promise<T> {
    const id = state.nextRequestId++;
    return new Promise<T>((resolve, reject) => {
      const guard = armAbort(opts?.signal, (msg) => worker.postMessage(msg), id, resolve, reject);
      if (!guard) {
        reject(new Error('operation cancelled'));
        return;
      }
      pending.set(id, guard);
      worker.postMessage(build(id));
    });
  }

  /** Текстовая выгрузка: формат решает, какой функцией модуля её считать. */
  function exportText(format: TextExportFormat, opts: { signal?: AbortSignal } = {}): Promise<string> {
    if (spec.formatTag === 'xlsx') {
      return Promise.reject(new Error('текстовая выгрузка не поддерживается для xlsx'));
    }
    return request(
      opts,
      (id) =>
        spec.formatTag === 'docx'
          ? { type: 'export-docx-text', id, format }
          : { type: 'export-pptx-text', id, format },
      state.pendingText,
    );
  }

  /** TextLayer страницы (`page`) или всего документа (`null`). */
  function requestTextLayer(page: number | null): Promise<TextLayer> {
    const id = state.nextRequestId++;
    return new Promise<TextLayer>((resolve, reject) => {
      state.pendingTextLayer.set(id, { resolve, reject });
      const message: InMsg =
        page === null ? { type: 'text-layer-all', id } : { type: 'text-layer', id, page };
      worker.postMessage(message);
    });
  }

  async function open(source: ArrayBuffer | ReadableStream<Uint8Array>, opts: { size?: number; signal?: AbortSignal } = {}): Promise<void> {
    // Спан один на вызов, не на кадр (ADR-0037); атрибуты — только формат, размер входа и число единиц документа, без байтов документа.
    const attrs: SpanAttributes = { format: spec.telemetry };
    if (source instanceof ArrayBuffer) attrs.bytes = source.byteLength;
    else if (opts.size !== undefined) attrs.bytes = opts.size;
    const span = startSpan('viewer.open', attrs);
    let failed: unknown;
    try {
      const id = state.nextRequestId++;
      await new Promise<void>((resolve, reject) => {
        // Новое открытие отменяет предыдущее (ADR-0032): два параллельных open на один воркер не имеют смысла, а ответ старого повис бы в пустоте.
        for (const stale of state.pendingOpen.keys()) {
          worker.postMessage({ type: 'cancel', id: stale } satisfies InMsg);
        }
        const guard = armAbort(opts.signal, (msg) => worker.postMessage(msg), id, resolve, reject);
        if (!guard) {
          reject(new Error('operation cancelled'));
          return;
        }
        state.pendingOpen.set(id, guard);
        if (source instanceof ArrayBuffer) {
          worker.postMessage(spec.openMsg(state, id, source), [source]);
          return;
        }
        // Поток: чанки уезжают переводом владения, main не держит файл.
        const pump: PumpOptions = {};
        if (opts.size !== undefined) pump.size = opts.size;
        if (options.onProgress) pump.onProgress = options.onProgress;
        void pumpStream(
          (msg, transfer) => worker.postMessage(msg, transfer ?? []),
          id,
          spec.formatTag,
          source,
          pump,
        ).catch((e) => {
          // Чтение потока упало (сеть, отмена): воркер обычно уже ответил ошибкой «stream aborted», но если ответа не было — реджектим здесь.
          const pending = state.pendingOpen.get(id);
          if (pending) {
            state.pendingOpen.delete(id);
            pending.reject(new Error(String(e)));
          }
        });
      });
      setSpanAttribute(span, openedCountKey(spec.formatTag), state.openedCount);
    } catch (e) {
      failed = e;
      throw e;
    } finally {
      endSpan(span, failed);
    }
  }

  function setZoom(next: number): void {
    const clamped = Math.min(Math.max(next, 0.1), 4);
    // Прокрутка в пикселях экрана: после зума пересчитываем, чтобы в окне осталось то же место документа.
    const ratio = clamped / state.zoom;
    state.zoom = clamped;
    scroller.scrollLeft *= ratio;
    scroller.scrollTop *= ratio;
    spec.applyLayout(state);
    placeCanvas();
    schedule();
  }

  function scrollTo(x: number, y: number): void {
    // Точка раскладки встаёт в левый верхний угол; левый край отстоит от начала раскладки на поле, вертикально поля нет.
    scroller.scrollLeft = state.padLeft + x * state.zoom;
    scroller.scrollTop = y * state.zoom;
    placeCanvas();
    schedule();
  }

  async function exportPngBytes(opts: { signal?: AbortSignal } = {}): Promise<Uint8Array> {
    // Спан один на вызов (ADR-0037); `bytes` — размер результата, не данных.
    const span = startSpan('viewer.export', { format: spec.telemetry, kind: 'png' });
    let failed: unknown;
    try {
      let bytes: Uint8Array;
      const local = state.render.capturePng();
      if (local) {
        // В main-режиме холст и painter здесь: снимок делается локально.
        bytes = await local;
      } else {
        bytes = await request(
          opts,
          (id) => ({ type: 'export-png', id }) satisfies InMsg,
          state.pendingPng,
        );
      }
      setSpanAttribute(span, 'bytes', bytes.byteLength);
      return bytes;
    } catch (e) {
      failed = e;
      throw e;
    } finally {
      endSpan(span, failed);
    }
  }

  async function exportPdfBytes(options: PdfOptions = {}, opts: { signal?: AbortSignal } = {}): Promise<Uint8Array> {
    // Спан один на вызов (ADR-0037); `bytes` — размер результата, не данных.
    const span = startSpan('viewer.export', { format: spec.telemetry, kind: 'pdf' });
    let failed: unknown;
    try {
      const bytes = await request(
        opts,
        (id) => spec.exportPdfMsg(state, id, options),
        state.pendingPdf,
      );
      setSpanAttribute(span, 'bytes', bytes.byteLength);
      return bytes;
    } catch (e) {
      failed = e;
      throw e;
    } finally {
      endSpan(span, failed);
    }
  }

  function onTick(handler: (stats: PaintStats) => void): () => void {
    state.tickHandlers.add(handler);
    return () => state.tickHandlers.delete(handler);
  }

  function destroy(): void {
    state.destroyed = true;
    observer.disconnect();
    scroller.removeEventListener('scroll', onScroll);
    for (const dispose of state.disposers) dispose();
    state.disposers.clear();
    // Отмена адресуется воркеру до его остановки (ADR-0032); ответа на запросы уже не будет — их промисы реджектятся здесь же.
    for (const id of state.pendingOpen.keys()) {
      worker.postMessage({ type: 'cancel', id } satisfies InMsg);
    }
    const error = new Error('viewer destroyed');
    for (const map of pendingRejectables) rejectAll(map, error);
    state.textLayer?.destroy();
    // Сначала цикл кадров: он вернёт воркеру вытесненный буфер, пока тот ещё жив. Затем singleton painter'а освобождается для страницы.
    state.render.destroy();
    worker.postMessage({ type: 'close' } satisfies InMsg);
    worker.terminate();
    scroller.remove();
  }

  return {
    state,
    transport: ready.transport,
    renderMode,
    open,
    setZoom,
    scrollTo,
    exportPng: exportPngBytes,
    exportPdf: exportPdfBytes,
    exportText,
    getTextLayer: requestTextLayer,
    getFullTextLayer: () => requestTextLayer(null),
    onTick,
    destroy,
  };
}
