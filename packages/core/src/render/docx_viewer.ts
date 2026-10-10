import type {
  InMsg,
  OutMsg,
  PageInfo,
  PaintStats,
  RenderConfig,
  Viewport,
} from '../protocol.js';
import { computeDpr } from './resize_observer.js';

/** Что умеет открытый документ. */
export interface DocxViewerHandle {
  /** Размеры страниц в пикселях раскладки, как их видит рендер. */
  readonly pages: PageInfo[];
  /** Сколько страниц в документе. */
  readonly pageCount: number;
  /** Номер видимой страницы — той, для которой собирается кадр. */
  readonly pageIndex: number;
  /** Зум: 1 — сто процентов. */
  readonly zoom: number;

  /** Открыть документ. Байты передаются воркеру переводом владения. */
  open(bytes: ArrayBuffer): Promise<PageInfo[]>;
  /** Показать страницу по номеру: прокрутка встаёт на её верх. */
  showPage(index: number): void;
  /** Задать зум и перерисовать. */
  setZoom(zoom: number): void;
  /** Прокрутить так, чтобы точка раскладки оказалась в левом верхнем углу. */
  scrollTo(x: number, y: number): void;
  /** Текущий кадр картинкой PNG — им проверяют, что нарисовалось. */
  exportPng(): Promise<Uint8Array>;
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
  /** Размер слота ring'а в байтах. */
  slotCapacity?: number;
  /** Начальный зум. */
  zoom?: number;
  /** Зазор между страницами в пикселях раскладки. */
  pageGap?: number;
}

const DEFAULT_SLOT_CAPACITY = 1 << 20;
const DEFAULT_PAGE_GAP = 16;

/**
 * Вьюер DOCX: холст над областью прокрутки, страницы — стопкой по вертикали.
 *
 * Разметка та же, что у XLSX: прокручиваемая область, распорка по высоте стопки
 * и холст, приклеенный к видимой части трансформом, — холст остаётся на месте,
 * пока под ним едет распорка, и браузер не перерисовывает его при скролле.
 *
 * В кадр уходит одна страница: та, что занимает верх видимой области.
 *
 * Ограничение первого слайса: `docx_build_display_list_sab` собирает одну
 * страницу, поэтому на стыке двух страниц нижняя часть холста остаётся пустой,
 * пока следующая страница не станет текущей. Показать несколько страниц в одном
 * кадре — отдельная работа: команды надо собирать в общий `DisplayList` со
 * сдвигом на высоту предыдущих страниц.
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

  // Проверяем ровно ту возможность, которой пользуемся: глобального
  // `OffscreenCanvas` может не быть и там, где холст отдаётся воркеру. Без неё
  // сообщение должно быть внятным, а не `transferControlToOffscreen is not a
  // function` из недр разметки.
  if (typeof canvas.transferControlToOffscreen !== 'function') {
    throw new Error(
      'этот браузер не поддерживает OffscreenCanvas (transferControlToOffscreen) — просмотрщик не запустится',
    );
  }

  const worker = new Worker(
    options.workerUrl ?? new URL('../worker/worker.js', import.meta.url),
    { type: 'module', name: 'doc-converter-docx' },
  );

  const slotCapacity = options.slotCapacity ?? DEFAULT_SLOT_CAPACITY;
  const pageGap = options.pageGap ?? DEFAULT_PAGE_GAP;
  const tickHandlers = new Set<(stats: PaintStats) => void>();
  let pages: PageInfo[] = [];
  /** Верх каждой страницы в пикселях раскладки — считается при открытии. */
  let pageTops: number[] = [];
  let stackWidth = 0;
  let stackHeight = 0;
  let pageIndex = 0;
  let zoom = options.zoom ?? 1;
  let dpr = computeDpr(scroller.clientWidth, scroller.clientHeight, devicePixelRatio);
  let destroyed = false;
  let frameScheduled = false;
  const pendingOpen = new Set<{
    resolve: (pages: PageInfo[]) => void;
    reject: (e: Error) => void;
  }>();
  const pendingPng = new Map<
    number,
    { resolve: (bytes: Uint8Array) => void; reject: (e: Error) => void }
  >();
  let nextRequestId = 1;

  worker.addEventListener('message', (ev: MessageEvent<OutMsg>) => {
    const msg = ev.data;
    switch (msg.type) {
      case 'tick':
        for (const handler of tickHandlers) handler(msg.stats);
        break;
      case 'docx-opened':
        pages = msg.pages;
        layoutStack();
        applyStackSize();
        pageIndex = pageAt(scroller.scrollTop / zoom);
        for (const p of pendingOpen) p.resolve(msg.pages);
        pendingOpen.clear();
        schedule();
        break;
      case 'png': {
        pendingPng.get(msg.id)?.resolve(msg.bytes);
        pendingPng.delete(msg.id);
        break;
      }
      case 'error':
        for (const p of pendingOpen) p.reject(new Error(msg.message));
        pendingOpen.clear();
        // У ошибки нет id: отклоняем весь экспорт — незавершённый повис бы навсегда.
        for (const p of pendingPng.values()) p.reject(new Error(msg.message));
        pendingPng.clear();
        console.error('[docx]', msg.message);
        break;
      default:
        break;
    }
  });

  await new Promise<void>((resolve, reject) => {
    const onMessage = (ev: MessageEvent) => {
      const data = ev.data as { type?: string; message?: string };
      if (data?.type === 'ready') {
        worker.removeEventListener('message', onMessage);
        resolve();
      } else if (data?.type === 'error') {
        worker.removeEventListener('message', onMessage);
        reject(new Error(data.message ?? 'worker init error'));
      }
    };
    worker.addEventListener('message', onMessage);
    const off = canvas.transferControlToOffscreen();
    worker.postMessage(
      { type: 'init', canvas: off, wasmUrl: options.wasmUrl, slotCapacity } satisfies InMsg,
      [off],
    );
  });

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

  /** Размер стопки на экране: зум растягивает раскладку. */
  function applyStackSize(): void {
    spacer.style.width = `${stackWidth * zoom}px`;
    spacer.style.height = `${stackHeight * zoom}px`;
  }

  function resizeCanvas(): void {
    const cssW = scroller.clientWidth;
    const cssH = scroller.clientHeight;
    if (cssW === 0 || cssH === 0) return;
    dpr = computeDpr(cssW, cssH, devicePixelRatio);
    canvas.style.width = `${cssW}px`;
    canvas.style.height = `${cssH}px`;
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
   */
  function pageAt(scrollY: number): number {
    let index = 0;
    for (let i = 1; i < pageTops.length; i++) {
      const top = pageTops[i];
      if (top === undefined || top > scrollY) break;
      index = i;
    }
    return index;
  }

  /**
   * Окно в единицах рендера: `x`/`y` — пиксели раскладки, `w`/`h` — физические
   * пиксели canvas, `scale` — зум, умноженный на DPR (как у XLSX).
   *
   * `y` отсчитывается от верха текущей страницы, а не от верха стопки: кадр
   * собирается для одной страницы и её начало — нуль раскладки в Rust.
   */
  function viewport(): Viewport {
    return {
      x: scroller.scrollLeft / zoom,
      y: scroller.scrollTop / zoom - pageTop(pageIndex),
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
    pageIndex = pageAt(scroller.scrollTop / zoom);
    worker.postMessage({
      type: 'render',
      req: { page: pageIndex, viewport: viewport(), config: frameConfig() },
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

    open(bytes: ArrayBuffer): Promise<PageInfo[]> {
      return new Promise<PageInfo[]>((resolve, reject) => {
        pendingOpen.add({ resolve, reject });
        worker.postMessage({ type: 'open', format: 'docx', bytes } satisfies InMsg, [bytes]);
      });
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
      scroller.scrollLeft = x * zoom;
      scroller.scrollTop = y * zoom;
      onScroll();
    },

    exportPng(): Promise<Uint8Array> {
      const id = nextRequestId++;
      return new Promise<Uint8Array>((resolve, reject) => {
        pendingPng.set(id, { resolve, reject });
        worker.postMessage({ type: 'export-png', id } satisfies InMsg);
      });
    },

    onTick(handler: (stats: PaintStats) => void): () => void {
      tickHandlers.add(handler);
      return () => tickHandlers.delete(handler);
    },

    destroy(): void {
      destroyed = true;
      observer.disconnect();
      scroller.removeEventListener('scroll', onScroll);
      // Воркер остановлен: ответа на эти запросы уже не будет.
      for (const p of pendingOpen) p.reject(new Error('viewer destroyed'));
      pendingOpen.clear();
      for (const p of pendingPng.values()) p.reject(new Error('viewer destroyed'));
      pendingPng.clear();
      worker.postMessage({ type: 'close' } satisfies InMsg);
      worker.terminate();
      scroller.remove();
    },
  };
}
