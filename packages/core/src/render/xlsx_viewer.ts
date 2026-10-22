import type {
  FrameTransport,
  HyperlinkInfo,
  InMsg,
  OutMsg,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  ReadyInfo,
  RenderConfig,
  RenderMode,
  SheetInfo,
  Viewport,
} from '../protocol.js';
import { armAbort } from './abort.js';
import { exportPng } from './export_png.js';
import { createMainFrameChain, type MainFrameChain } from './main_frame_chain.js';
import {
  disposeMainPainter,
  dropMainBitmap,
  initMainPainter,
  registerMainBitmap,
  resizeMainCanvas,
} from './main_painter.js';
import { resolveRenderMode } from './offscreen.js';
import { pumpStream, type PumpOptions } from './open_stream.js';
import { computeDpr } from './resize_observer.js';
import { endSpan, setSpanAttribute, startSpan, type SpanAttributes } from '../telemetry.js';

/** Что умеет открытая книга. */
export interface XlsxViewerHandle {
  /**
   * Каким путём воркер возит кадры: SAB, если страница отдана с COOP/COEP,
   * иначе буферы. Функциональность от этого не зависит — поле для диагностики.
   */
  readonly transport: FrameTransport;
  /** Где рисуется кадр: в воркере или на main-потоке. */
  readonly renderMode: 'worker' | 'main';
  /** Имена и размеры листов, как их видит рендер. */
  readonly sheets: SheetInfo[];
  /** Номер показанного листа. */
  readonly sheetIndex: number;
  /** Зум: 1 — сто процентов. */
  readonly zoom: number;

  /**
   * Открыть книгу. Байты передаются воркеру переводом владения; поток
   * (`ReadableStream<Uint8Array>`) уезжает чанками — прогресс и отмена
   * загрузки (ADR-0031). `signal` — отмена тяжёлой операции (ADR-0032):
   * воркер остановит её в ближайшей точке проверки.
   */
  open(
    source: ArrayBuffer | ReadableStream<Uint8Array>,
    opts?: { size?: number; signal?: AbortSignal },
  ): Promise<SheetInfo[]>;
  /** Показать лист по номеру. */
  showSheet(index: number): void;
  /** Задать зум и перерисовать. */
  setZoom(zoom: number): void;
  /** Прокрутить так, чтобы ячейка оказалась в левом верхнем углу. */
  scrollTo(x: number, y: number): void;
  /** Показать или скрыть сетку, заголовки и тёмное оформление. */
  setView(options: {
    showGrid?: boolean;
    showHeaders?: boolean;
    theme?: 'light' | 'dark';
  }): void;
  /** Ячейка под точкой события мыши, в координатах страницы. */
  hitTest(clientX: number, clientY: number): Promise<[number, number] | null>;
  /** Гиперссылка под точкой события мыши, в координатах страницы. */
  hyperlinkAt(clientX: number, clientY: number): Promise<HyperlinkInfo | null>;
  /** Текущий кадр картинкой PNG — им проверяют, что нарисовалось. */
  exportPng(opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /**
   * Экспорт листа в PDF. Без настроек — весь текущий лист на A4.
   * `options.sheetIndex` перекрывает показанный лист, `options.allSheets` —
   * вся книга одним документом (листы подряд, закладка на каждый).
   */
  exportPdf(options?: PdfOptions, opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /** Подписка на метрики кадров. */
  onTick(handler: (stats: PaintStats) => void): () => void;
  /** Остановить воркер и убрать за собой разметку. */
  destroy(): void;
}

export interface XlsxViewerOptions {
  /** Готовый воркер: свой URL или уже собранный бандл. */
  workerUrl?: string | URL;
  /** URL wasm-модуля; по умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /**
   * Где лежит ленивый PDF-модуль. Воркер грузит его при первом экспорте;
   * не задан — кнопка экспорта получит внятную ошибку вместо PDF.
   */
  pdfWasm?: PdfModuleUrls;
  /** Размер слота ring'а в байтах. */
  slotCapacity?: number;
  /**
   * Транспорт кадра. `auto` (по умолчанию) — SAB, если страница отдана с
   * COOP/COEP, иначе буферы. Задавать явно стоит ради отладки и тестов:
   * `sab` без заголовков — ошибка инициализации, а не тихая подмена.
   */
  transport?: 'auto' | FrameTransport;
  /**
   * Где рисовать кадр. `auto` (по умолчанию) — воркер, если холст можно отдать
   * ему через `transferControlToOffscreen`, иначе main-поток. Явный `worker`
   * без этой возможности — ошибка инициализации; `main` вместе с
   * `transport: 'sab'` — ошибка конфигурации.
   */
  renderMode?: RenderMode;
  /** Начальный зум. */
  zoom?: number;
  showGrid?: boolean;
  showHeaders?: boolean;
  /**
   * Тёмное оформление листа: цвета темы книги и контраст текста считаются
   * под тёмный фон. По умолчанию светлое — как в Excel.
   */
  theme?: 'light' | 'dark';
  /**
   * Клик по ячейке с гиперссылкой. Получает саму ссылку; клик по ячейке без
   * ссылки колбэк не трогает.
   */
  onHyperlink?: (link: HyperlinkInfo) => void;
  /**
   * Прогресс стримового открытия (ADR-0031): вызывается на каждый
   * отправленный чанк. `total` — размер из `open(source, { size })`, если
   * вызывающий его знает; путь байтов колбэк не трогает.
   */
  onProgress?: (loaded: number, total?: number) => void;
  /**
   * Минимальная a11y-обвязка: canvas скрыт от скринридеров, область получает
   * `role=document`. Полноценная DOM-сетка листа — после Sprint 2.
   */
  a11y?: boolean;
}

const DEFAULT_SLOT_CAPACITY = 1 << 20;

/**
 * Вьюер XLSX: холст над областью прокрутки.
 *
 * Разметка собирается целиком здесь: хост получает прокручиваемую область,
 * распорку по размеру листа и холст, приклеенный к видимой части. Холст
 * сдвигается трансформом на величину прокрутки — так он остаётся на месте,
 * пока под ним едет распорка, и браузер не перерисовывает его при скролле.
 */
export async function createXlsxViewer(
  host: HTMLElement,
  options: XlsxViewerOptions = {},
): Promise<XlsxViewerHandle> {
  const scroller = document.createElement('div');
  scroller.style.cssText = 'position:relative;overflow:auto;width:100%;height:100%';
  const spacer = document.createElement('div');
  spacer.style.cssText = 'position:absolute;top:0;left:0;width:0;height:0';
  const canvas = document.createElement('canvas');
  canvas.style.cssText =
    'position:absolute;top:0;left:0;display:block;transform:translate(0px,0px)';
  scroller.append(spacer, canvas);
  if (options.a11y === true) {
    canvas.setAttribute('aria-hidden', 'true');
    scroller.setAttribute('role', 'document');
  }
  host.append(scroller);

  // Режим решает, уедет ли холст в воркер. Проверяем ровно ту возможность,
  // которой пользуемся: глобального `OffscreenCanvas` может не быть и там, где
  // холст отдаётся воркеру, а main-режиму он не нужен вовсе.
  const renderMode = resolveRenderMode(canvas, options.renderMode, options.transport);

  const worker = new Worker(
    options.workerUrl ?? new URL('../worker/xlsx_worker.js', import.meta.url),
    { type: 'module', name: 'doc-converter-xlsx' },
  );

  const slotCapacity = options.slotCapacity ?? DEFAULT_SLOT_CAPACITY;
  /** Битмапы main-режима: main перерегистрирует их после resize. */
  const mainBitmaps = new Map<number, ImageBitmap>();
  let mainChain: MainFrameChain | null = null;
  const tickHandlers = new Set<(stats: PaintStats) => void>();
  let sheets: SheetInfo[] = [];
  let sheetIndex = 0;
  let zoom = options.zoom ?? 1;
  let dpr = computeDpr(scroller.clientWidth, scroller.clientHeight, devicePixelRatio);
  let showGrid = options.showGrid ?? true;
  let showHeaders = options.showHeaders ?? true;
  let theme: 'light' | 'dark' = options.theme ?? 'light';
  let destroyed = false;
  let frameScheduled = false;
  const pendingOpen = new Map<number, { resolve: (s: SheetInfo[]) => void; reject: (e: Error) => void }>();
  const pendingHit = new Map<number, { resolve: (c: [number, number] | null) => void }>();
  const pendingHyperlink = new Map<number, { resolve: (link: HyperlinkInfo | null) => void }>();
  const pendingPng = new Map<
    number,
    { resolve: (bytes: Uint8Array) => void; reject: (e: Error) => void }
  >();
  const pendingPdf = new Map<
    number,
    { resolve: (bytes: Uint8Array) => void; reject: (e: Error) => void }
  >();
  let nextRequestId = 1;

  // main-режим: painter поднимается до воркера и до первого кадра. Если
  // инициализация не удалась, воркер не должен остаться висеть.
  if (renderMode === 'main') {
    const ctx2d = canvas.getContext('2d');
    if (!ctx2d) {
      worker.terminate();
      throw new Error('getContext("2d") returned null');
    }
    try {
      await initMainPainter(ctx2d, options.wasmUrl);
    } catch (e) {
      worker.terminate();
      throw e;
    }
    mainChain = createMainFrameChain(worker, (stats) => {
      for (const handler of tickHandlers) handler(stats);
    });
  }

  worker.addEventListener('message', (ev: MessageEvent<OutMsg>) => {
    const msg = ev.data;
    switch (msg.type) {
      case 'tick':
        for (const handler of tickHandlers) handler(msg.stats);
        break;
      case 'frame':
        // main-режим: воркер собрал DisplayList, рисуем и возвращаем буфер.
        mainChain?.handle(msg);
        break;
      case 'bitmap': {
        registerMainBitmap(msg.id, msg.bitmap);
        mainBitmaps.set(msg.id, msg.bitmap);
        break;
      }
      case 'drop-bitmap': {
        dropMainBitmap(msg.id);
        mainBitmaps.delete(msg.id);
        break;
      }
      case 'clear-bitmaps': {
        for (const id of mainBitmaps.keys()) dropMainBitmap(id);
        mainBitmaps.clear();
        break;
      }
      case 'opened': {
        sheets = msg.sheets;
        // Ответ адресный: разрешаем ровно тот промис, чей id эхом вернул воркер.
        const pending = pendingOpen.get(msg.id);
        if (pending) {
          pendingOpen.delete(msg.id);
          pending.resolve(msg.sheets);
        }
        applySheetSize();
        schedule();
        break;
      }
      case 'hit': {
        pendingHit.get(msg.id)?.resolve(msg.cell);
        pendingHit.delete(msg.id);
        break;
      }
      case 'hyperlink': {
        pendingHyperlink.get(msg.id)?.resolve(msg.link);
        pendingHyperlink.delete(msg.id);
        break;
      }
      case 'png': {
        pendingPng.get(msg.id)?.resolve(msg.bytes);
        pendingPng.delete(msg.id);
        break;
      }
      case 'pdf': {
        pendingPdf.get(msg.id)?.resolve(msg.bytes);
        pendingPdf.delete(msg.id);
        break;
      }
      case 'error': {
        const error = new Error(msg.message);
        console.error('[xlsx]', msg.code ? `${msg.code}: ${msg.message}` : msg.message);
        if (msg.id !== undefined) {
          // Ошибка адресная: отклоняем только свой запрос, соседние ждут
          // своих ответов — их ошибка не касается.
          const open = pendingOpen.get(msg.id);
          if (open) {
            pendingOpen.delete(msg.id);
            open.reject(error);
          }
          const png = pendingPng.get(msg.id);
          if (png) {
            pendingPng.delete(msg.id);
            png.reject(error);
          }
          const pdf = pendingPdf.get(msg.id);
          if (pdf) {
            pendingPdf.delete(msg.id);
            pdf.reject(error);
          }
          break;
        }
        // Ошибки без id — сбой воркера целиком (init): ответа не будет ни на
        // один запрос, поэтому отклоняем всё ожидающее.
        for (const p of pendingOpen.values()) p.reject(error);
        pendingOpen.clear();
        for (const p of pendingPng.values()) p.reject(error);
        pendingPng.clear();
        for (const p of pendingPdf.values()) p.reject(error);
        pendingPdf.clear();
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
        const data = ev.data as { type?: string; message?: string } & Partial<ReadyInfo>;
        if (data?.type === 'ready') {
          worker.removeEventListener('message', onMessage);
          resolve({
            backend: data.backend ?? 'worker',
            transport: data.transport ?? 'bytes',
            slotCapacity: data.slotCapacity ?? slotCapacity,
            totalBytes: data.totalBytes ?? 0,
            crossOriginIsolated: data.crossOriginIsolated ?? false,
          });
        } else if (data?.type === 'error') {
          worker.removeEventListener('message', onMessage);
          reject(new Error(data.message ?? 'worker init error'));
        }
      };
      worker.addEventListener('message', onMessage);
      const init = {
        type: 'init',
        backend: renderMode,
        wasmUrl: options.wasmUrl,
        pdf: options.pdfWasm,
        slotCapacity,
        transport: options.transport,
      } satisfies InMsg;
      if (renderMode === 'worker') {
        // Холст уезжает воркеру ровно один раз; в main-режиме он остаётся здесь.
        const off = canvas.transferControlToOffscreen();
        worker.postMessage({ ...init, canvas: off } satisfies InMsg, [off]);
      } else {
        worker.postMessage(init);
      }
    });
  } catch (e) {
    // Воркер не поднялся — он не нужен, а painter в main-режиме singleton
    // страницы: без освобождения следующий вьюер не смог бы инициализироваться.
    worker.terminate();
    mainChain?.destroy();
    if (renderMode === 'main') disposeMainPainter();
    throw e;
  }

  /** Размер листа в пикселях экрана: зум растягивает раскладку. */
  function applySheetSize(): void {
    const sheet = sheets[sheetIndex];
    if (!sheet) return;
    spacer.style.width = `${sheet.width * zoom}px`;
    spacer.style.height = `${sheet.height * zoom}px`;
  }

  function resizeCanvas(): void {
    const cssW = scroller.clientWidth;
    const cssH = scroller.clientHeight;
    if (cssW === 0 || cssH === 0) return;
    dpr = computeDpr(cssW, cssH, devicePixelRatio);
    canvas.style.width = `${cssW}px`;
    canvas.style.height = `${cssH}px`;
    if (renderMode === 'main') {
      // Размеры ставит main: холст здесь же, воркеру о них знать нечего.
      // Установка width/height сбрасывает 2D-контекст, поэтому painter
      // пересчитывает его сам, а картинки возвращаются в кэш после этого.
      canvas.width = Math.max(1, Math.round(cssW * dpr));
      canvas.height = Math.max(1, Math.round(cssH * dpr));
      resizeMainCanvas(cssW, cssH, dpr);
      for (const [id, bmp] of mainBitmaps) registerMainBitmap(id, bmp);
      return;
    }
    worker.postMessage({ type: 'resize', cssW, cssH, dpr } satisfies InMsg);
  }

  /**
   * Окно в единицах рендера: прокрутка браузера — в пикселях экрана, а
   * раскладка — до зума, поэтому x/y делятся на зум.
   */
  function viewport(): Viewport {
    return {
      x: scroller.scrollLeft / zoom,
      y: scroller.scrollTop / zoom,
      w: scroller.clientWidth * dpr,
      h: scroller.clientHeight * dpr,
      scale: zoom * dpr,
    };
  }

  /** Настройки рисования, общие для кадра и запросов по точке. */
  function renderConfig(): RenderConfig {
    return { showGrid, showHeaders, theme, dpr };
  }

  /** Точка события мыши в физических пикселях холста. */
  function canvasPoint(clientX: number, clientY: number): [number, number] {
    const rect = canvas.getBoundingClientRect();
    return [(clientX - rect.left) * dpr, (clientY - rect.top) * dpr];
  }

  function render(): void {
    if (sheets.length === 0) return;
    worker.postMessage({
      type: 'render',
      req: { sheet: sheetIndex, viewport: viewport(), config: renderConfig() },
    } satisfies InMsg);
  }

  function hitTest(clientX: number, clientY: number): Promise<[number, number] | null> {
    const [x, y] = canvasPoint(clientX, clientY);
    const id = nextRequestId++;
    return new Promise<[number, number] | null>((resolve) => {
      pendingHit.set(id, { resolve });
      worker.postMessage({
        type: 'hit-test',
        id,
        sheet: sheetIndex,
        x,
        y,
        viewport: viewport(),
        config: renderConfig(),
      } satisfies InMsg);
    });
  }

  function hyperlinkAt(clientX: number, clientY: number): Promise<HyperlinkInfo | null> {
    const [x, y] = canvasPoint(clientX, clientY);
    const id = nextRequestId++;
    return new Promise<HyperlinkInfo | null>((resolve) => {
      pendingHyperlink.set(id, { resolve });
      worker.postMessage({
        type: 'hyperlink-at',
        id,
        sheet: sheetIndex,
        x,
        y,
        viewport: viewport(),
        config: renderConfig(),
      } satisfies InMsg);
    });
  }

  /**
   * Клик по холсту: ссылку ищем там же, где ячейку для подсказки адреса.
   * Книги может не быть вовсе — тогда и спрашивать нечего.
   */
  const onCanvasClick = (ev: MouseEvent): void => {
    const callback = options.onHyperlink;
    if (!callback || destroyed || sheets.length === 0) return;
    void hyperlinkAt(ev.clientX, ev.clientY).then((link) => {
      // Клик по ячейке без ссылки — обычный клик, колбэк молчит.
      if (link) callback(link);
    });
  };
  canvas.addEventListener('click', onCanvasClick);

  function schedule(): void {
    if (frameScheduled || destroyed) return;
    frameScheduled = true;
    requestAnimationFrame(() => {
      frameScheduled = false;
      render();
    });
  }

  const onScroll = () => {
    canvas.style.transform = `translate(${scroller.scrollLeft}px, ${scroller.scrollTop}px)`;
    schedule();
  };
  scroller.addEventListener('scroll', onScroll, { passive: true });

  const observer = new ResizeObserver(() => {
    resizeCanvas();
    schedule();
  });
  observer.observe(scroller);

  resizeCanvas();

  return {
    transport: ready.transport,
    renderMode,

    get sheets() {
      return sheets;
    },
    get sheetIndex() {
      return sheetIndex;
    },
    get zoom() {
      return zoom;
    },

    async open(
      source: ArrayBuffer | ReadableStream<Uint8Array>,
      opts: { size?: number; signal?: AbortSignal } = {},
    ): Promise<SheetInfo[]> {
      // Спан один на вызов, не на кадр (ADR-0037); атрибуты — только формат,
      // размер входа и число листов, без байтов документа.
      const attrs: SpanAttributes = { format: 'xlsx' };
      if (source instanceof ArrayBuffer) attrs.bytes = source.byteLength;
      else if (opts.size !== undefined) attrs.bytes = opts.size;
      const span = startSpan('viewer.open', attrs);
      let failed: unknown;
      try {
        const id = nextRequestId++;
        const result = await new Promise<SheetInfo[]>((resolve, reject) => {
          // Новое открытие отменяет предыдущее (ADR-0032): два параллельных open
          // на один воркер не имеют смысла, а ответ старого повис бы в пустоте.
          for (const stale of pendingOpen.keys()) {
            worker.postMessage({ type: 'cancel', id: stale } satisfies InMsg);
          }
          const guard = armAbort(
            opts.signal,
            (msg) => worker.postMessage(msg),
            id,
            resolve,
            reject,
          );
          if (!guard) {
            reject(new Error('operation cancelled'));
            return;
          }
          pendingOpen.set(id, guard);
          if (source instanceof ArrayBuffer) {
            worker.postMessage(
              { type: 'open', id, format: 'xlsx', bytes: source } satisfies InMsg,
              [source],
            );
            return;
          }
          // Поток: чанки уезжают переводом владения, main не держит файл.
          const pump: PumpOptions = {};
          if (opts.size !== undefined) pump.size = opts.size;
          if (options.onProgress) pump.onProgress = options.onProgress;
          void pumpStream(
            (msg, transfer) => worker.postMessage(msg, transfer ?? []),
            id,
            'xlsx',
            source,
            pump,
          ).catch((e) => {
            // Чтение потока упало (сеть, отмена): воркер обычно уже ответил
            // ошибкой «stream aborted», но если ответа не было — реджектим здесь.
            const pending = pendingOpen.get(id);
            if (pending) {
              pendingOpen.delete(id);
              pending.reject(new Error(String(e)));
            }
          });
        });
        setSpanAttribute(span, 'sheetCount', result.length);
        return result;
      } catch (e) {
        failed = e;
        throw e;
      } finally {
        endSpan(span, failed);
      }
    },

    showSheet(index: number): void {
      if (index < 0 || index >= sheets.length) return;
      sheetIndex = index;
      scroller.scrollTop = 0;
      scroller.scrollLeft = 0;
      applySheetSize();
      onScroll();
    },

    setZoom(next: number): void {
      const clamped = Math.min(Math.max(next, 0.1), 4);
      // Прокрутка задана в пикселях экрана: чтобы после зума показывалось то
      // же место листа, её нужно пересчитать.
      const ratio = clamped / zoom;
      zoom = clamped;
      scroller.scrollLeft *= ratio;
      scroller.scrollTop *= ratio;
      applySheetSize();
      onScroll();
    },

    scrollTo(x: number, y: number): void {
      scroller.scrollLeft = x * zoom;
      scroller.scrollTop = y * zoom;
      onScroll();
    },

    setView(next): void {
      if (next.showGrid !== undefined) showGrid = next.showGrid;
      if (next.showHeaders !== undefined) showHeaders = next.showHeaders;
      if (next.theme !== undefined) theme = next.theme;
      schedule();
    },

    hitTest,

    hyperlinkAt,

    async exportPng(opts: { signal?: AbortSignal } = {}): Promise<Uint8Array> {
      // Спан один на вызов (ADR-0037); `bytes` — размер результата, не данных.
      const span = startSpan('viewer.export', { format: 'xlsx', kind: 'png' });
      let failed: unknown;
      try {
        let bytes: Uint8Array;
        if (renderMode === 'main') {
          // В main-режиме холст и painter здесь: снимок делается локально, без
          // лишнего круга через воркер.
          bytes = await exportPng(canvas);
        } else {
          const id = nextRequestId++;
          bytes = await new Promise<Uint8Array>((resolve, reject) => {
            const guard = armAbort(
              opts.signal,
              (msg) => worker.postMessage(msg),
              id,
              resolve,
              reject,
            );
            if (!guard) {
              reject(new Error('operation cancelled'));
              return;
            }
            pendingPng.set(id, guard);
            worker.postMessage({ type: 'export-png', id } satisfies InMsg);
          });
        }
        setSpanAttribute(span, 'bytes', bytes.byteLength);
        return bytes;
      } catch (e) {
        failed = e;
        throw e;
      } finally {
        endSpan(span, failed);
      }
    },

    async exportPdf(
      options: PdfOptions = {},
      opts: { signal?: AbortSignal } = {},
    ): Promise<Uint8Array> {
      // Спан один на вызов (ADR-0037); `bytes` — размер результата, не данных.
      const span = startSpan('viewer.export', { format: 'xlsx', kind: 'pdf' });
      let failed: unknown;
      try {
        const id = nextRequestId++;
        const sheet = options.sheetIndex ?? sheetIndex;
        const bytes = await new Promise<Uint8Array>((resolve, reject) => {
          const guard = armAbort(
            opts.signal,
            (msg) => worker.postMessage(msg),
            id,
            resolve,
            reject,
          );
          if (!guard) {
            reject(new Error('operation cancelled'));
            return;
          }
          pendingPdf.set(id, guard);
          worker.postMessage({
            type: 'export-pdf',
            id,
            sheet,
            options,
          } satisfies InMsg);
        });
        setSpanAttribute(span, 'bytes', bytes.byteLength);
        return bytes;
      } catch (e) {
        failed = e;
        throw e;
      } finally {
        endSpan(span, failed);
      }
    },

    onTick(handler: (stats: PaintStats) => void): () => void {
      tickHandlers.add(handler);
      return () => tickHandlers.delete(handler);
    },

    destroy(): void {
      destroyed = true;
      observer.disconnect();
      scroller.removeEventListener('scroll', onScroll);
      canvas.removeEventListener('click', onCanvasClick);
      // Отмена адресуется воркеру до его остановки (ADR-0032): флаг остановит
      // операцию в ближайшей точке проверки. Ответа на запросы уже не будет —
      // их промисы реджектятся здесь же.
      for (const id of pendingOpen.keys()) {
        worker.postMessage({ type: 'cancel', id } satisfies InMsg);
      }
      const error = new Error('viewer destroyed');
      for (const p of pendingOpen.values()) p.reject(error);
      pendingOpen.clear();
      for (const p of pendingPng.values()) p.reject(error);
      pendingPng.clear();
      for (const p of pendingPdf.values()) p.reject(error);
      pendingPdf.clear();
      // Сначала цикл кадров: он вернёт воркеру вытесненный буфер, пока тот
      // ещё жив. Затем singleton painter'а освобождается для страницы.
      mainChain?.destroy();
      worker.postMessage({ type: 'close' } satisfies InMsg);
      worker.terminate();
      if (renderMode === 'main') {
        for (const id of mainBitmaps.keys()) dropMainBitmap(id);
        mainBitmaps.clear();
        disposeMainPainter();
      }
      scroller.remove();
    },
  };
}
