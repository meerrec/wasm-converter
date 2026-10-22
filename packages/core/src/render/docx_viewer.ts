import type {
  DocxModuleUrls,
  DocxTextFormat,
  FrameTransport,
  InMsg,
  OutMsg,
  PageInfo,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  ReadyInfo,
  RenderConfig,
  RenderMode,
  TextLayer,
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
import { createTextLayer, type TextLayerController } from './text_layer.js';
import { endSpan, setSpanAttribute, startSpan, type SpanAttributes } from '../telemetry.js';

/** Что умеет открытый документ. */
export interface DocxViewerHandle {
  /**
   * Каким путём воркер возит кадры: SAB, если страница отдана с COOP/COEP,
   * иначе буферы. Функциональность от этого не зависит — поле для диагностики.
   */
  readonly transport: FrameTransport;
  /** Где рисуется кадр: в воркере или на main-потоке. */
  readonly renderMode: 'worker' | 'main';
  /** Размеры страниц в пикселях раскладки, как их видит рендер. */
  readonly pages: PageInfo[];
  /** Сколько страниц в документе. */
  readonly pageCount: number;
  /** Номер видимой страницы — той, что занимает верх области прокрутки. */
  readonly pageIndex: number;
  /** Зум: 1 — сто процентов. */
  readonly zoom: number;

  /**
   * Открыть документ. Байты передаются воркеру переводом владения; поток
   * (`ReadableStream<Uint8Array>`) уезжает чанками — прогресс и отмена
   * загрузки (ADR-0031). `signal` — отмена тяжёлой раскладки (ADR-0032):
   * воркер остановит её между блоками тела.
   */
  open(
    source: ArrayBuffer | ReadableStream<Uint8Array>,
    opts?: { size?: number; signal?: AbortSignal },
  ): Promise<PageInfo[]>;
  /** Показать страницу по номеру: прокрутка встаёт на её верх. */
  showPage(index: number): void;
  /** Задать зум и перерисовать. */
  setZoom(zoom: number): void;
  /** Прокрутить так, чтобы точка раскладки оказалась в левом верхнем углу. */
  scrollTo(x: number, y: number): void;
  /** Текущий кадр картинкой PNG — им проверяют, что нарисовалось. */
  exportPng(opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /**
   * Экспорт документа в PDF. Без настроек — страницы раскладки со своими
   * размерами и полями из `w:sectPr`.
   *
   * `options` — тот же `PdfOptions`, что у XLSX-пути, но читаются не все поля:
   * `pageSize`, `orientation`, `margins`, `scale` и `sheetIndex` не влияют,
   * страницы берутся из документа.
   */
  exportPdf(options?: PdfOptions, opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /** Экспорт документа в Markdown: заголовки, списки, таблицы, ссылки. */
  exportMarkdown(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Экспорт документа в HTML — самостоятельную страницу со встроенными стилями. */
  exportHtml(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Экспорт документа в обычный текст: содержимое без разметки. */
  exportText(opts?: { signal?: AbortSignal }): Promise<string>;
  /**
   * TextLayer одной страницы (0-based): runs с bbox в пикселях раскладки.
   *
   * Не зависит от флага `a11y`: это данные, а не DOM-слой.
   */
  getTextLayer(page: number): Promise<TextLayer>;
  /** TextLayer всего документа — для семантического дерева и Ctrl+A. */
  getFullTextLayer(): Promise<TextLayer>;
  /** Подписка на метрики кадров. */
  onTick(handler: (stats: PaintStats) => void): () => void;
  /** Остановить воркер и убрать за собой разметку. */
  destroy(): void;
}

export interface DocxViewerOptions {
  /** Готовый воркер: свой URL или уже собранный бандл. */
  workerUrl?: string | URL;
  /** URL wasm-модуля; по умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /**
   * Где лежит ленивый DOCX-модуль. Воркер грузит его при открытии документа;
   * не задан — открытие получит внятную ошибку вместо страниц.
   */
  docxWasm?: DocxModuleUrls;
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
  /**
   * Доступный документ: прозрачный DOM-слой поверх canvas с выделением текста
   * и ARIA-деревом. По умолчанию выключен — включаем после стабилизации (v2).
   *
   * Флаг задаётся при создании вьюера; в React смена объекта опций пересоздаёт
   * вьюер, в Vue/Svelte опция читается только при монтировании.
   */
  a11y?: boolean;
  /** Начальный зум. */
  zoom?: number;
  /**
   * Зазор между страницами в пикселях раскладки. Он же уходит в кадр: по нему
   * Rust ставит страницы стопкой, и стопка в кадре обязана совпасть с распоркой.
   */
  pageGap?: number;
  /**
   * Прогресс стримового открытия (ADR-0031): вызывается на каждый
   * отправленный чанк. `total` — размер из `open(source, { size })`, если
   * вызывающий его знает; путь байтов колбэк не трогает.
   */
  onProgress?: (loaded: number, total?: number) => void;
}

const DEFAULT_SLOT_CAPACITY = 1 << 20;
const DEFAULT_PAGE_GAP = 16;
/**
 * Допуск на округление прокрутки браузером, экранные пиксели. Округление вниз
 * теряет меньше пикселя, так что этот допуск покрывает и его, и округление к
 * ближайшему.
 */
const SCROLL_EPSILON_PX = 1;

/**
 * Вьюер DOCX: холст над областью прокрутки, страницы — стопкой по вертикали.
 *
 * Разметка та же, что у XLSX: прокручиваемая область, распорка по высоте стопки
 * и холст, приклеенный к видимой части трансформом, — холст остаётся на месте,
 * пока под ним едет распорка, и браузер не перерисовывает его при скролле.
 *
 * В кадр уходит стопка страниц: те, что пересекают окно, плюс по одной соседней
 * сверху и снизу, — поэтому на стыке двух страниц холст не пустует. Зазор между
 * страницами в кадре и в распорке — один и тот же (`pageGap`): разойдись они,
 * кадр поехал бы относительно прокрутки.
 *
 * Стопка у́же области прокрутки — она стоит по центру: слева и справа от неё
 * одинаковые поля, а ширина распорки — стопка плюс оба поля. Прокрутка от этого
 * остаётся симметричной, и центрированное положение — это `scrollLeft = 0`.
 * По вертикали центрирования нет: стопка длиннее окна и прокручивается.
 */
export async function createDocxViewer(
  host: HTMLElement,
  options: DocxViewerOptions = {},
): Promise<DocxViewerHandle> {
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
    options.workerUrl ?? new URL('../worker/docx_worker.js', import.meta.url),
    { type: 'module', name: 'doc-converter-docx' },
  );

  const slotCapacity = options.slotCapacity ?? DEFAULT_SLOT_CAPACITY;
  /** Битмапы main-режима: main перерегистрирует их после resize. */
  const mainBitmaps = new Map<number, ImageBitmap>();
  let mainChain: MainFrameChain | null = null;
  const pageGap = options.pageGap ?? DEFAULT_PAGE_GAP;
  const tickHandlers = new Set<(stats: PaintStats) => void>();
  let pages: PageInfo[] = [];
  /** Верх каждой страницы в пикселях раскладки — считается при открытии. */
  let pageTops: number[] = [];
  let stackWidth = 0;
  let stackHeight = 0;
  let pageIndex = 0;
  let zoom = options.zoom ?? 1;
  /** Поле слева от стопки на экране: им она центрируется по горизонтали. */
  let padLeft = 0;
  let dpr = computeDpr(scroller.clientWidth, scroller.clientHeight, devicePixelRatio);
  let destroyed = false;
  let frameScheduled = false;
  const pendingOpen = new Map<
    number,
    { resolve: (pages: PageInfo[]) => void; reject: (e: Error) => void }
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
  const pendingTextLayer = new Map<
    number,
    { resolve: (layer: TextLayer) => void; reject: (e: Error) => void }
  >();
  let nextRequestId = 1;

  // Доступный слой создаётся до первого кадра: он живёт в layout-координатах и
  // наполняется после `docx-opened`.
  let textLayer: TextLayerController | null = null;
  if (options.a11y === true) {
    canvas.setAttribute('aria-hidden', 'true');
    textLayer = createTextLayer({
      fetchPage: (page) => requestTextLayer(page),
      fetchAll: () => requestTextLayer(null),
    });
    scroller.append(textLayer.element);
  }

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
      case 'docx-opened': {
        pages = msg.pages;
        layoutStack();
        applyStackSize();
        textLayer?.configure(pages, pageTops);
        pageIndex = pageAt(scroller.scrollTop);
        // Ответ адресный: разрешаем ровно тот промис, чей id эхом вернул воркер.
        const pending = pendingOpen.get(msg.id);
        if (pending) {
          pendingOpen.delete(msg.id);
          pending.resolve(msg.pages);
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
      case 'docx-text': {
        pendingText.get(msg.id)?.resolve(msg.text);
        pendingText.delete(msg.id);
        break;
      }
      case 'text-layer': {
        pendingTextLayer.get(msg.id)?.resolve(msg.layer);
        pendingTextLayer.delete(msg.id);
        break;
      }
      case 'error': {
        const error = new Error(msg.message);
        console.error('[docx]', msg.code ? `${msg.code}: ${msg.message}` : msg.message);
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
          const layer = pendingTextLayer.get(msg.id);
          if (layer) {
            pendingTextLayer.delete(msg.id);
            layer.reject(error);
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
        for (const p of pendingTextLayer.values()) p.reject(error);
        pendingTextLayer.clear();
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
        docx: options.docxWasm,
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

  /** Разложить страницы в стопку: верх каждой и общий размер в пикселях раскладки. */
  function layoutStack(): void {
    pageTops = [];
    stackWidth = 0;
    let y = 0;
    for (const page of pages) {
      pageTops.push(y);
      y += page.height + pageGap;
      stackWidth = Math.max(stackWidth, page.width);
    }
    // Зазор нужен только между страницами — за последней его нет.
    stackHeight = pages.length === 0 ? 0 : y - pageGap;
  }

  /**
   * Размер стопки на экране: зум растягивает раскладку, поля центрируют её.
   *
   * Поле считается по ширине области прокрутки, а не холста: разметка
   * прокручивается, и ориентироваться ей есть на что только здесь. Стопка шире
   * области — поля нет вовсе, прокрутка идёт по самой стопке; уже — распорка
   * получает стопку плюс оба поля, так что прокручивать нечего (`scrollLeft`
   * остаётся нулём), а стопка стоит посередине.
   */
  function applyStackSize(): void {
    padLeft = Math.max(0, (scroller.clientWidth - stackWidth * zoom) / 2);
    spacer.style.width = `${stackWidth * zoom + 2 * padLeft}px`;
    spacer.style.height = `${stackHeight * zoom}px`;
    // Слой в layout-координатах: зум и поле — только transform, spans не
    // пересобираются и не отстают от canvas на кадр.
    textLayer?.setTransform(padLeft, zoom, stackWidth, stackHeight);
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

  /** Верх страницы в пикселях раскладки; у номера вне документа — нуль. */
  function pageTop(index: number): number {
    return pageTops[index] ?? 0;
  }

  /**
   * Страница, занимающая верх видимой области: последняя, чей верх не ниже
   * прокрутки. В зазоре между страницами это страница выше — верх области
   * показывает её конец, а не пустоту.
   *
   * `scrollPx` — `scrollTop`, как его отдаёт браузер, в экранных пикселях.
   * WebKit хранит прокрутку целым, округляя вниз: `showPage` ставит её на
   * верх страницы, а обратно приходит на доли пикселя меньше (у страницы
   * высотой 1122.5333 верх второй — 1138.5333, а прокрутка — 1138). Строгое
   * сравнение сочло бы такую страницу ещё не начавшейся и нарисовало
   * предыдущую. Дробная высота страниц — норма, ошибка округления вниз — до
   * целого пикселя, поэтому попадание в его пределах относится к странице
   * ниже. Сравнение идёт в экранных пикселях: квантует браузер именно их.
   */
  function pageAt(scrollPx: number): number {
    let index = 0;
    for (let i = 1; i < pageTops.length; i++) {
      const top = pageTops[i];
      if (top === undefined || top * zoom > scrollPx + SCROLL_EPSILON_PX) break;
      index = i;
    }
    return index;
  }

  /**
   * Окно в единицах рендера: `x`/`y` — пиксели раскладки, `w`/`h` — физические
   * пиксели canvas, `scale` — зум, умноженный на DPR (как у XLSX).
   *
   * `y` отсчитывается от верха стопки, а не от верха текущей страницы: кадр
   * собирается для всей видимой стопки, и её начало — нуль раскладки в Rust.
   * `x` — тоже от начала раскладки, поэтому поле слева, которого при
   * центрировании на холст не видно, уводит его в минус.
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
   * Настройки кадра. Кадру DOCX они не нужны — воркер в этой ветке их не
   * читает, — но поле обязательно по типу `RenderRequest`.
   */
  function frameConfig(): RenderConfig {
    return { showGrid: false, showHeaders: false, theme: 'light', dpr };
  }

  function render(): void {
    if (pages.length === 0) return;
    pageIndex = pageAt(scroller.scrollTop);
    worker.postMessage({
      type: 'render',
      req: { gap: pageGap, viewport: viewport(), config: frameConfig() },
    } satisfies InMsg);
  }

  function schedule(): void {
    if (frameScheduled || destroyed) return;
    frameScheduled = true;
    requestAnimationFrame(() => {
      frameScheduled = false;
      render();
      // Второй слой обновляется в том же rAF, что и кадр: окно виртуализации
      // не должно отставать от canvas.
      textLayer?.updateWindow(scroller.scrollTop, scroller.clientHeight, zoom);
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
    // размера стопку надо переставить, даже если прокрутка не сдвинулась.
    applyStackSize();
    placeCanvas();
    schedule();
  });
  observer.observe(scroller);

  resizeCanvas();

  /** Выгрузка в текст: форматов три, а путь один — какой из них, решает воркер. */
  function exportText(
    format: DocxTextFormat,
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
      worker.postMessage({ type: 'export-docx-text', id, format } satisfies InMsg);
    });
  }

  /** TextLayer страницы (`page`) или всего документа (`null`). */
  function requestTextLayer(page: number | null): Promise<TextLayer> {
    const id = nextRequestId++;
    return new Promise<TextLayer>((resolve, reject) => {
      pendingTextLayer.set(id, { resolve, reject });
      const message: InMsg =
        page === null
          ? { type: 'text-layer-all', id }
          : { type: 'text-layer', id, page };
      worker.postMessage(message);
    });
  }

  return {
    transport: ready.transport,
    renderMode,

    get pages() {
      return pages;
    },
    get pageCount() {
      return pages.length;
    },
    get pageIndex() {
      return pageIndex;
    },
    get zoom() {
      return zoom;
    },

    async open(
      source: ArrayBuffer | ReadableStream<Uint8Array>,
      opts: { size?: number; signal?: AbortSignal } = {},
    ): Promise<PageInfo[]> {
      // Спан один на вызов, не на кадр (ADR-0037); атрибуты — только формат,
      // размер входа и число страниц, без байтов документа.
      const attrs: SpanAttributes = { format: 'docx' };
      if (source instanceof ArrayBuffer) attrs.bytes = source.byteLength;
      else if (opts.size !== undefined) attrs.bytes = opts.size;
      const span = startSpan('viewer.open', attrs);
      let failed: unknown;
      try {
        const id = nextRequestId++;
        const result = await new Promise<PageInfo[]>((resolve, reject) => {
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
              { type: 'open', id, format: 'docx', bytes: source } satisfies InMsg,
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
            'docx',
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
        setSpanAttribute(span, 'pageCount', result.length);
        return result;
      } catch (e) {
        failed = e;
        throw e;
      } finally {
        endSpan(span, failed);
      }
    },

    showPage(index: number): void {
      if (index < 0 || index >= pages.length) return;
      pageIndex = index;
      scroller.scrollLeft = 0;
      scroller.scrollTop = pageTop(index) * zoom;
      onScroll();
    },

    setZoom(next: number): void {
      const clamped = Math.min(Math.max(next, 0.1), 4);
      // Прокрутка задана в пикселях экрана: чтобы после зума показывалось то
      // же место стопки, её нужно пересчитать.
      const ratio = clamped / zoom;
      zoom = clamped;
      scroller.scrollLeft *= ratio;
      scroller.scrollTop *= ratio;
      applyStackSize();
      onScroll();
    },

    scrollTo(x: number, y: number): void {
      // Точка раскладки встаёт в левый верхний угол видимой области, а её
      // левый край отстоит от начала раскладки на поле; вертикально поля нет.
      scroller.scrollLeft = padLeft + x * zoom;
      scroller.scrollTop = y * zoom;
      onScroll();
    },

    async exportPng(opts: { signal?: AbortSignal } = {}): Promise<Uint8Array> {
      // Спан один на вызов (ADR-0037); `bytes` — размер результата, не данных.
      const span = startSpan('viewer.export', { format: 'docx', kind: 'png' });
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
      const span = startSpan('viewer.export', { format: 'docx', kind: 'pdf' });
      let failed: unknown;
      try {
        const id = nextRequestId++;
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
          worker.postMessage({ type: 'export-docx-pdf', id, options } satisfies InMsg);
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

    exportMarkdown: (opts?: { signal?: AbortSignal }) => exportText('markdown', opts),
    exportHtml: (opts?: { signal?: AbortSignal }) => exportText('html', opts),
    exportText: (opts?: { signal?: AbortSignal }) => exportText('text', opts),
    getTextLayer: (page: number) => requestTextLayer(page),
    getFullTextLayer: () => requestTextLayer(null),

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
      for (const p of pendingTextLayer.values()) p.reject(error);
      pendingTextLayer.clear();
      textLayer?.destroy();
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
