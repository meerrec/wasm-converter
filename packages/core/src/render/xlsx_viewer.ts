import type {
  HyperlinkInfo,
  InMsg,
  OutMsg,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  RenderConfig,
  SheetInfo,
  Viewport,
} from '../protocol.js';
import { computeDpr } from './resize_observer.js';

/** Что умеет открытая книга. */
export interface XlsxViewerHandle {
  /** Имена и размеры листов, как их видит рендер. */
  readonly sheets: SheetInfo[];
  /** Номер показанного листа. */
  readonly sheetIndex: number;
  /** Зум: 1 — сто процентов. */
  readonly zoom: number;

  /** Открыть книгу. Байты передаются воркеру переводом владения. */
  open(bytes: ArrayBuffer): Promise<SheetInfo[]>;
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
  exportPng(): Promise<Uint8Array>;
  /**
   * Экспорт листа в PDF. Без настроек — весь текущий лист на A4.
   * `options.sheetIndex` перекрывает показанный лист.
   */
  exportPdf(options?: PdfOptions): Promise<Uint8Array>;
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
    { type: 'module', name: 'doc-converter-xlsx' },
  );

  const slotCapacity = options.slotCapacity ?? DEFAULT_SLOT_CAPACITY;
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
  const pendingPng = new Map<number, (bytes: Uint8Array) => void>();
  const pendingPdf = new Map<
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
      case 'opened':
        sheets = msg.sheets;
        for (const p of pendingOpen.values()) p.resolve(msg.sheets);
        pendingOpen.clear();
        applySheetSize();
        schedule();
        break;
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
        pendingPng.get(msg.id)?.(msg.bytes);
        pendingPng.delete(msg.id);
        break;
      }
      case 'pdf': {
        pendingPdf.get(msg.id)?.resolve(msg.bytes);
        pendingPdf.delete(msg.id);
        break;
      }
      case 'error':
        for (const p of pendingOpen.values()) p.reject(new Error(msg.message));
        pendingOpen.clear();
        // У ошибки нет id: отклоняем все экспорты — незавершённый повис бы навсегда.
        for (const p of pendingPdf.values()) p.reject(new Error(msg.message));
        pendingPdf.clear();
        console.error('[xlsx]', msg.message);
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
      {
        type: 'init',
        canvas: off,
        wasmUrl: options.wasmUrl,
        pdf: options.pdfWasm,
        slotCapacity,
      } satisfies InMsg,
      [off],
    );
  });

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
    get sheets() {
      return sheets;
    },
    get sheetIndex() {
      return sheetIndex;
    },
    get zoom() {
      return zoom;
    },

    open(bytes: ArrayBuffer): Promise<SheetInfo[]> {
      const id = nextRequestId++;
      return new Promise<SheetInfo[]>((resolve, reject) => {
        pendingOpen.set(id, { resolve, reject });
        worker.postMessage({ type: 'open', format: 'xlsx', bytes } satisfies InMsg, [bytes]);
      });
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

    exportPng(): Promise<Uint8Array> {
      const id = nextRequestId++;
      return new Promise<Uint8Array>((resolve) => {
        pendingPng.set(id, resolve);
        worker.postMessage({ type: 'export-png', id } satisfies InMsg);
      });
    },

    exportPdf(options: PdfOptions = {}): Promise<Uint8Array> {
      const id = nextRequestId++;
      const sheet = options.sheetIndex ?? sheetIndex;
      return new Promise<Uint8Array>((resolve, reject) => {
        pendingPdf.set(id, { resolve, reject });
        worker.postMessage({
          type: 'export-pdf',
          id,
          sheet,
          options,
        } satisfies InMsg);
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
      canvas.removeEventListener('click', onCanvasClick);
      worker.postMessage({ type: 'close' } satisfies InMsg);
      worker.terminate();
      scroller.remove();
    },
  };
}
