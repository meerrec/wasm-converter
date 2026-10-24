import type {
  FrameTransport,
  HyperlinkInfo,
  InMsg,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  RenderConfig,
  RenderMode,
  SheetInfo,
  Viewport,
} from '../protocol.js';
import {
  createViewerBase,
  type ViewerSpec,
  type ViewerState,
} from './viewer_core.js';

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
   * вся книга одним документом (видимые листы подряд, закладка на каждый;
   * скрытые не печатаются, как в Excel и LibreOffice).
   *
   * `fitToPage` укладывает лист на одну страницу (1×1),
   * `fitToWidth`/`fitToHeight` задают число страниц по осям (`0` — без
   * ограничения) и сильнее флага; fit-режим перебивает ручной `scale`.
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

/** Форматное состояние книги: что отличает её от DOCX/PPTX. */
interface XlsxState {
  sheets: SheetInfo[];
  sheetIndex: number;
  showGrid: boolean;
  showHeaders: boolean;
  theme: 'light' | 'dark';
}

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
  /** Окно в единицах рендера: прокрутка браузера — в пикселях экрана, а раскладка — до зума. */
  function viewport(st: ViewerState<XlsxState>): Viewport {
    return {
      x: st.scroller.scrollLeft / st.zoom,
      y: st.scroller.scrollTop / st.zoom,
      w: st.scroller.clientWidth * st.dpr,
      h: st.scroller.clientHeight * st.dpr,
      scale: st.zoom * st.dpr,
    };
  }

  /** Настройки рисования, общие для кадра и запросов по точке. */
  function renderConfig(st: ViewerState<XlsxState>): RenderConfig {
    const d = st.data;
    return { showGrid: d.showGrid, showHeaders: d.showHeaders, theme: d.theme, dpr: st.dpr };
  }

  /** Точка события мыши в физических пикселях холста. */
  function canvasPoint(st: ViewerState<XlsxState>, clientX: number, clientY: number): [number, number] {
    const rect = st.canvas.getBoundingClientRect();
    return [(clientX - rect.left) * st.dpr, (clientY - rect.top) * st.dpr];
  }

  function hitTest(
    st: ViewerState<XlsxState>,
    clientX: number,
    clientY: number,
  ): Promise<[number, number] | null> {
    const [x, y] = canvasPoint(st, clientX, clientY);
    const id = st.nextRequestId++;
    return new Promise<[number, number] | null>((resolve) => {
      st.pendingHit.set(id, { resolve });
      st.worker.postMessage({
        type: 'hit-test',
        id,
        sheet: st.data.sheetIndex,
        x,
        y,
        viewport: viewport(st),
        config: renderConfig(st),
      } satisfies InMsg);
    });
  }

  function hyperlinkAt(
    st: ViewerState<XlsxState>,
    clientX: number,
    clientY: number,
  ): Promise<HyperlinkInfo | null> {
    const [x, y] = canvasPoint(st, clientX, clientY);
    const id = st.nextRequestId++;
    return new Promise<HyperlinkInfo | null>((resolve) => {
      st.pendingHyperlink.set(id, { resolve });
      st.worker.postMessage({
        type: 'hyperlink-at',
        id,
        sheet: st.data.sheetIndex,
        x,
        y,
        viewport: viewport(st),
        config: renderConfig(st),
      } satisfies InMsg);
    });
  }

  const spec: ViewerSpec<XlsxState, XlsxViewerOptions> = {
    workerName: 'doc-converter-xlsx',
    workerUrl: ({ workerUrl }) => workerUrl ?? new URL('../worker/xlsx_worker.js', import.meta.url),
    telemetry: 'xlsx',
    formatTag: 'xlsx',
    openedResponse: 'opened',
    onOpened(st, msg) {
      if (msg.type !== 'opened') return;
      st.data.sheets = msg.sheets;
      st.applyLayout();
      st.schedule();
    },
    applyLayout(st) {
      const sheet = st.data.sheets[st.data.sheetIndex];
      if (!sheet) return;
      st.spacer.style.width = `${sheet.width * st.zoom}px`;
      st.spacer.style.height = `${sheet.height * st.zoom}px`;
    },
    viewport,
    openMsg(_st, id, bytes) {
      return { type: 'open', id, format: 'xlsx', bytes } satisfies InMsg;
    },
    createState(opts) {
      return {
        sheets: [],
        sheetIndex: 0,
        showGrid: opts.showGrid ?? true,
        showHeaders: opts.showHeaders ?? true,
        theme: opts.theme ?? 'light',
      };
    },
    initOptions: (opts) => ({ pdf: opts.pdfWasm }),
    renderMsg(st) {
      if (st.data.sheets.length === 0) return null;
      return {
        type: 'render',
        req: { sheet: st.data.sheetIndex, viewport: viewport(st), config: renderConfig(st) },
      } satisfies InMsg;
    },
    exportPdfMsg(st, id, options) {
      return {
        type: 'export-pdf',
        id,
        sheet: options.sheetIndex ?? st.data.sheetIndex,
        options,
      } satisfies InMsg;
    },
    extraInit(st, canvas) {
      // Доступная книга: область прокрутки отвечает скринридеру как документ.
      if (options.a11y === true) st.scroller.setAttribute('role', 'document');
      // Клик по холсту: ссылку ищем там же, где ячейку для подсказки адреса.
      // Книги может не быть вовсе — тогда и спрашивать нечего.
      const onCanvasClick = (ev: MouseEvent): void => {
        const callback = options.onHyperlink;
        if (!callback || st.destroyed || st.data.sheets.length === 0) return;
        void hyperlinkAt(st, ev.clientX, ev.clientY).then((link) => {
          // Клик по ячейке без ссылки — обычный клик, колбэк молчит.
          if (link) callback(link);
        });
      };
      canvas.addEventListener('click', onCanvasClick);
      st.disposers.add(() => canvas.removeEventListener('click', onCanvasClick));
    },
  };

  const base = await createViewerBase<XlsxState, XlsxViewerOptions>(host, spec, options);
  const st = base.state;

  return {
    transport: base.transport,
    renderMode: base.renderMode,

    get sheets() {
      return st.data.sheets;
    },
    get sheetIndex() {
      return st.data.sheetIndex;
    },
    get zoom() {
      return st.zoom;
    },

    async open(
      source: ArrayBuffer | ReadableStream<Uint8Array>,
      opts?: { size?: number; signal?: AbortSignal },
    ): Promise<SheetInfo[]> {
      await base.open(source, opts);
      return st.data.sheets;
    },

    showSheet(index: number): void {
      if (index < 0 || index >= st.data.sheets.length) return;
      st.data.sheetIndex = index;
      st.scroller.scrollTop = 0;
      st.scroller.scrollLeft = 0;
      st.applyLayout();
      st.refresh();
    },

    setZoom: base.setZoom,
    scrollTo: base.scrollTo,

    setView(next: { showGrid?: boolean; showHeaders?: boolean; theme?: 'light' | 'dark' }): void {
      if (next.showGrid !== undefined) st.data.showGrid = next.showGrid;
      if (next.showHeaders !== undefined) st.data.showHeaders = next.showHeaders;
      if (next.theme !== undefined) st.data.theme = next.theme;
      st.schedule();
    },

    hitTest: (clientX: number, clientY: number) => hitTest(st, clientX, clientY),
    hyperlinkAt: (clientX: number, clientY: number) => hyperlinkAt(st, clientX, clientY),

    exportPng: base.exportPng,
    exportPdf: base.exportPdf,

    onTick: base.onTick,
    destroy: base.destroy,
  };
}
