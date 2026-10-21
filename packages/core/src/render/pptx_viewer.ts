import type {
  FrameTransport,
  InMsg,
  OutMsg,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  PptxModuleUrls,
  PptxTextFormat,
  ReadyInfo,
  RenderConfig,
  RenderMode,
  SlideInfo,
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

/** Что умеет открытая презентация. */
export interface PptxViewerHandle {
  /**
   * Каким путём воркер возит кадры: SAB, если страница отдана с COOP/COEP,
   * иначе буферы. Функциональность от этого не зависит — поле для диагностики.
   */
  readonly transport: FrameTransport;
  /** Где рисуется кадр: в воркере или на main-потоке. */
  readonly renderMode: 'worker' | 'main';
  /** Слайды презентации: индексы, имена и размеры в пикселях раскладки. */
  readonly slides: SlideInfo[];
  /** Сколько слайдов в презентации. */
  readonly slideCount: number;
  /** Номер видимого слайда. */
  readonly slideIndex: number;
  /** Зум: 1 — сто процентов. */
  readonly zoom: number;

  /**
   * Открыть презентацию. Байты передаются воркеру переводом владения; поток
   * (`ReadableStream<Uint8Array>`) уезжает чанками — прогресс и отмена
   * загрузки (ADR-0031). `signal` — отмена тяжёлой раскладки (ADR-0032):
   * воркер остановит её между слайдами.
   */
  open(
    source: ArrayBuffer | ReadableStream<Uint8Array>,
    opts?: { size?: number; signal?: AbortSignal },
  ): Promise<SlideInfo[]>;
  /** Показать слайд по номеру. */
  showSlide(index: number): void;
  /** Задать зум и перерисовать. */
  setZoom(zoom: number): void;
  /** Прокрутить так, чтобы точка раскладки оказалась в левом верхнем углу. */
  scrollTo(x: number, y: number): void;
  /** Текущий кадр картинкой PNG — им проверяют, что нарисовалось. */
  exportPng(opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /**
   * Экспорт презентации в PDF: слайд = страница, размер страницы из `p:sldSz`.
   *
   * `options` — тот же `PdfOptions`, что у XLSX-пути, но поля страницы не
   * читаются: размер, поля, ориентация и масштаб задаёт сама презентация.
   */
  exportPdf(options?: PdfOptions, opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /** Экспорт конспекта в Markdown: слайды заголовками, маркеры списков. */
  exportMarkdown(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Экспорт конспекта в HTML — самостоятельную страницу со встроенными стилями. */
  exportHtml(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Экспорт конспекта в обычный текст: текстовые рамки без разметки. */
  exportText(opts?: { signal?: AbortSignal }): Promise<string>;
  /**
   * Hit-test намеренно отсутствует: у презентации нет ячеек и позиций текста,
   * по которым вёл бы переход клик (ADR-0033 §1). Поиск по тексту слайдов —
   * отдельный срез, не попавший в скоуп.
   */
  onTick(handler: (stats: PaintStats) => void): () => void;
  /** Остановить воркер и убрать за собой разметку. */
  destroy(): void;
}

export interface PptxViewerOptions {
  /** Готовый воркер: свой URL или уже собранный бандл. */
  workerUrl?: string | URL;
  /** URL wasm-модуля; по умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /**
   * Где лежит ленивый PPTX-модуль. Воркер грузит его при открытии презентации;
   * не задан — открытие получит внятную ошибку вместо слайдов.
   */
  pptxWasm?: PptxModuleUrls;
  /**
   * Где лежит ленивый PDF-модуль. Воркер грузит его при первом экспорте в PDF;
   * не задан — кнопка экспорта получит внятную ошибку вместо файла.
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
  /**
   * Прогресс стримового открытия (ADR-0031): вызывается на каждый
   * отправленный чанк. `total` — размер из `open(source, { size })`, если
   * вызывающий его знает; путь байтов колбэк не трогает.
   */
  onProgress?: (loaded: number, total?: number) => void;
}

const DEFAULT_SLOT_CAPACITY = 1 << 20;

/**
 * Вьюер PPTX: холст над областью прокрутки, слайд — одна страница рендера.
 *
 * Разметка та же, что у DOCX: прокручиваемая область, распорка по размеру
 * слайда с зумом и холст, приклеенный к видимой части трансформом, — холст
 * остаётся на месте, пока под ним едет распорка, и браузер не перерисовывает
 * его при скролле.
 *
 * В кадр уходит один слайд целиком: пагинации у презентации нет (ADR-0033 §4),
 * размер страницы задан `p:sldSz`, а прокрутка нужна только при зуме больше
 * вписывающего слайд в окно.
 *
 * Слайд у́же области прокрутки — он стоит по центру: слева и справа от него
 * одинаковые поля, а ширина распорки — слайд плюс оба поля. Прокрутка от этого
 * остаётся симметричной, и центрированное положение — это `scrollLeft = 0`.
 */
export async function createPptxViewer(
  host: HTMLElement,
  options: PptxViewerOptions = {},
): Promise<PptxViewerHandle> {
  const scroller = document.createElement('div');
  scroller.style.cssText = 'position:relative;overflow:auto;width:100%;height:100%';
  const spacer = document.createElement('div');
  spacer.style.cssText = 'position:absolute;top:0;left:0;width:0;height:0';
  const canvas = document.createElement('canvas');
  canvas.style.cssText =
    'position:absolute;top:0;left:0;display:block;transform:translate(0px,0px)';
  scroller.append(spacer, canvas);
  host.append(scroller);

  // Режим решает, уедет ли холст в воркер. Проверяем ровно ту возможность,
  // которой пользуемся: глобального `OffscreenCanvas` может не быть и там, где
  // холст отдаётся воркеру, а main-режиму он не нужен вовсе.
  const renderMode = resolveRenderMode(canvas, options.renderMode, options.transport);

  const worker = new Worker(
    options.workerUrl ?? new URL('../worker/pptx_worker.js', import.meta.url),
    { type: 'module', name: 'doc-converter-pptx' },
  );

  const slotCapacity = options.slotCapacity ?? DEFAULT_SLOT_CAPACITY;
  /** Битмапы main-режима: main перерегистрирует их после resize. */
  const mainBitmaps = new Map<number, ImageBitmap>();
  let mainChain: MainFrameChain | null = null;
  const tickHandlers = new Set<(stats: PaintStats) => void>();
  let slides: SlideInfo[] = [];
  let slideIndex = 0;
  let slideWidth = 0;
  let slideHeight = 0;
  let zoom = options.zoom ?? 1;
  /** Поле слева от слайда на экране: им он центрируется по горизонтали. */
  let padLeft = 0;
  let dpr = computeDpr(scroller.clientWidth, scroller.clientHeight, devicePixelRatio);
  let destroyed = false;
  let frameScheduled = false;
  const pendingOpen = new Map<
    number,
    { resolve: (slides: SlideInfo[]) => void; reject: (e: Error) => void }
  >();
  const pendingPng = new Map<
    number,
    { resolve: (bytes: Uint8Array) => void; reject: (e: Error) => void }
  >();
  const pendingPdf = new Map<
    number,
    { resolve: (bytes: Uint8Array) => void; reject: (e: Error) => void }
  >();
  const pendingText = new Map<
    number,
    { resolve: (text: string) => void; reject: (e: Error) => void }
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
      case 'pptx-opened': {
        slides = msg.slides;
        slideIndex = 0;
        applySlideSize();
        // Ответ адресный: разрешаем ровно тот промис, чей id эхом вернул воркер.
        const pending = pendingOpen.get(msg.id);
        if (pending) {
          pendingOpen.delete(msg.id);
          pending.resolve(msg.slides);
        }
        schedule();
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
      case 'pptx-text': {
        pendingText.get(msg.id)?.resolve(msg.text);
        pendingText.delete(msg.id);
        break;
      }
      case 'error': {
        const error = new Error(msg.message);
        console.error('[pptx]', msg.code ? `${msg.code}: ${msg.message}` : msg.message);
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
          const text = pendingText.get(msg.id);
          if (text) {
            pendingText.delete(msg.id);
            text.reject(error);
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
        for (const p of pendingText.values()) p.reject(error);
        pendingText.clear();
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
        pptx: options.pptxWasm,
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

  /**
   * Размер слайда на экране: зум растягивает раскладку, поля центрируют её.
   *
   * Поле считается по ширине области прокрутки, а не холста: разметка
   * прокручивается, и ориентироваться ей есть на что только здесь. Слайд шире
   * области — поля нет вовсе, прокрутка идёт по самому слайду; уже — распорка
   * получает слайд плюс оба поля, так что прокручивать нечего (`scrollLeft`
   * остаётся нулём), а слайд стоит посередине.
   */
  function applySlideSize(): void {
    const slide = slides[slideIndex];
    slideWidth = slide?.width ?? 0;
    slideHeight = slide?.height ?? 0;
    padLeft = Math.max(0, (scroller.clientWidth - slideWidth * zoom) / 2);
    spacer.style.width = `${slideWidth * zoom + 2 * padLeft}px`;
    spacer.style.height = `${slideHeight * zoom}px`;
  }

  /**
   * Приклеить холст к видимой части области: холст лежит в начале распорки, и
   * прокрутка сдвигает его ровно настолько, чтобы он остался на месте экрана.
   * Поля распорки в этот сдвиг не входят — холст показывает то же окно, что
   * видно, а где оно в раскладке, говорит `viewport().x`.
   */
  function placeCanvas(): void {
    canvas.style.transform = `translate(${scroller.scrollLeft}px, ${scroller.scrollTop}px)`;
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
   * Окно в единицах рендера: `x`/`y` — пиксели раскладки, `w`/`h` — физические
   * пиксели canvas, `scale` — зум, умноженный на DPR (как у XLSX).
   *
   * `x`/`y` отсчитываются от левого верхнего угла слайда: кадр несёт один
   * слайд, и его начало — нуль раскладки в Rust. Поле слева, которого при
   * центрировании на холст не видно, уводит `x` в минус.
   */
  function viewport(): Viewport {
    return {
      x: (scroller.scrollLeft - padLeft) / zoom,
      y: scroller.scrollTop / zoom,
      w: scroller.clientWidth * dpr,
      h: scroller.clientHeight * dpr,
      scale: zoom * dpr,
    };
  }

  /**
   * Настройки кадра. Кадру PPTX они не нужны — воркер в этой ветке их не
   * читает, — но поле обязательно по типу `RenderRequest`.
   */
  function frameConfig(): RenderConfig {
    return { showGrid: false, showHeaders: false, theme: 'light', dpr };
  }

  function render(): void {
    if (slides.length === 0) return;
    worker.postMessage({
      type: 'render',
      req: { slide: slideIndex, viewport: viewport(), config: frameConfig() },
    } satisfies InMsg);
  }

  function schedule(): void {
    if (frameScheduled || destroyed) return;
    frameScheduled = true;
    requestAnimationFrame(() => {
      frameScheduled = false;
      render();
    });
  }

  const onScroll = () => {
    placeCanvas();
    schedule();
  };
  scroller.addEventListener('scroll', onScroll, { passive: true });

  const observer = new ResizeObserver(() => {
    resizeCanvas();
    // Ширина области прокрутки — она же база поля слева: после изменения
    // размера слайд надо переставить, даже если прокрутка не сдвинулась.
    applySlideSize();
    placeCanvas();
    schedule();
  });
  observer.observe(scroller);

  resizeCanvas();

  /** Выгрузка в текст: форматов три, а путь один — какой из них, решает воркер. */
  function exportText(
    format: PptxTextFormat,
    opts: { signal?: AbortSignal } = {},
  ): Promise<string> {
    const id = nextRequestId++;
    return new Promise<string>((resolve, reject) => {
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
      pendingText.set(id, guard);
      worker.postMessage({ type: 'export-pptx-text', id, format } satisfies InMsg);
    });
  }

  return {
    transport: ready.transport,
    renderMode,

    get slides() {
      return slides;
    },
    get slideCount() {
      return slides.length;
    },
    get slideIndex() {
      return slideIndex;
    },
    get zoom() {
      return zoom;
    },

    open(
      source: ArrayBuffer | ReadableStream<Uint8Array>,
      opts: { size?: number; signal?: AbortSignal } = {},
    ): Promise<SlideInfo[]> {
      const id = nextRequestId++;
      return new Promise<SlideInfo[]>((resolve, reject) => {
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
          worker.postMessage({ type: 'open', id, format: 'pptx', bytes: source } satisfies InMsg, [
            source,
          ]);
          return;
        }
        // Поток: чанки уезжают переводом владения, main не держит файл.
        const pump: PumpOptions = {};
        if (opts.size !== undefined) pump.size = opts.size;
        if (options.onProgress) pump.onProgress = options.onProgress;
        void pumpStream(
          (msg, transfer) => worker.postMessage(msg, transfer ?? []),
          id,
          'pptx',
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
    },

    showSlide(index: number): void {
      if (index < 0 || index >= slides.length) return;
      slideIndex = index;
      scroller.scrollLeft = 0;
      scroller.scrollTop = 0;
      applySlideSize();
      placeCanvas();
      schedule();
    },

    setZoom(next: number): void {
      const clamped = Math.min(Math.max(next, 0.1), 4);
      // Прокрутка задана в пикселях экрана: чтобы после зума показывалось то
      // же место слайда, её нужно пересчитать.
      const ratio = clamped / zoom;
      zoom = clamped;
      scroller.scrollLeft *= ratio;
      scroller.scrollTop *= ratio;
      applySlideSize();
      onScroll();
    },

    scrollTo(x: number, y: number): void {
      // Точка раскладки встаёт в левый верхний угол видимой области, а её
      // левый край отстоит от начала раскладки на поле; вертикально поля нет.
      scroller.scrollLeft = padLeft + x * zoom;
      scroller.scrollTop = y * zoom;
      onScroll();
    },

    exportPng(opts: { signal?: AbortSignal } = {}): Promise<Uint8Array> {
      // В main-режиме холст и painter здесь: снимок делается локально, без
      // лишнего круга через воркер.
      if (renderMode === 'main') return exportPng(canvas);
      const id = nextRequestId++;
      return new Promise<Uint8Array>((resolve, reject) => {
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
    },

    exportPdf(
      options: PdfOptions = {},
      opts: { signal?: AbortSignal } = {},
    ): Promise<Uint8Array> {
      const id = nextRequestId++;
      return new Promise<Uint8Array>((resolve, reject) => {
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
        worker.postMessage({ type: 'export-pptx-pdf', id, options } satisfies InMsg);
      });
    },

    exportMarkdown: (opts?: { signal?: AbortSignal }) => exportText('markdown', opts),
    exportHtml: (opts?: { signal?: AbortSignal }) => exportText('html', opts),
    exportText: (opts?: { signal?: AbortSignal }) => exportText('text', opts),

    onTick(handler: (stats: PaintStats) => void): () => void {
      tickHandlers.add(handler);
      return () => tickHandlers.delete(handler);
    },

    destroy(): void {
      destroyed = true;
      observer.disconnect();
      scroller.removeEventListener('scroll', onScroll);
      // Отмена адресуется воркеру до его остановки (ADR-0032). Ответа на
      // запросы уже не будет — их промисы реджектятся здесь же.
      for (const id of pendingOpen.keys()) {
        worker.postMessage({ type: 'cancel', id } satisfies InMsg);
      }
      // Воркер остановлен: ответа на эти запросы уже не будет.
      const error = new Error('viewer destroyed');
      for (const p of pendingOpen.values()) p.reject(error);
      pendingOpen.clear();
      for (const p of pendingPng.values()) p.reject(error);
      pendingPng.clear();
      for (const p of pendingPdf.values()) p.reject(error);
      pendingPdf.clear();
      for (const p of pendingText.values()) p.reject(error);
      pendingText.clear();
      worker.postMessage({ type: 'close' } satisfies InMsg);
      worker.terminate();
      mainChain?.destroy();
      if (renderMode === 'main') disposeMainPainter();
      scroller.remove();
    },
  };
}
