import type {
  FrameTransport,
  InMsg,
  PaintStats,
  RenderConfig,
  RenderMode,
  SheetInfo,
  Viewport,
  XlsbModuleUrls,
} from '../protocol.js';
import {
  createViewerBase,
  type ViewerSpec,
  type ViewerState,
} from './viewer_core.js';

/** Что умеет открытая книга XLSB. */
export interface XlsbViewerHandle {
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
   * загрузки (ADR-0031). `signal` — отмена тяжёлой операции (ADR-0032).
   */
  open(
    source: ArrayBuffer | ReadableStream<Uint8Array>,
    opts?: { size?: number; signal?: AbortSignal },
  ): Promise<SheetInfo[]>;
  /** Показать лист по номеру. */
  showSheet(index: number): void;
  /** Задать зум и перерисовать. */
  setZoom(zoom: number): void;
  /** Прокрутить так, чтобы точка раскладки оказалась в левом верхнем углу. */
  scrollTo(x: number, y: number): void;
  /** Показать или скрыть сетку, заголовки и тёмное оформление. */
  setView(options: {
    showGrid?: boolean;
    showHeaders?: boolean;
    theme?: 'light' | 'dark';
  }): void;
  /** Текущий кадр картинкой PNG — им проверяют, что нарисовалось. */
  exportPng(opts?: { signal?: AbortSignal }): Promise<Uint8Array>;
  /** Подписка на метрики кадров. */
  onTick(handler: (stats: PaintStats) => void): () => void;
  /** Остановить воркер и убрать за собой разметку. */
  destroy(): void;
}

export interface XlsbViewerOptions {
  /** Готовый воркер: свой URL или уже собранный бандл. */
  workerUrl?: string | URL;
  /** URL wasm-модуля; по умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /**
   * Где лежит ленивый XLSB-модуль. Воркер грузит его при открытии книги; не
   * задан — открытие получит внятную ошибку вместо листов.
   */
  xlsbWasm?: XlsbModuleUrls;
  /** Размер слота ring'а в байтах. */
  slotCapacity?: number;
  /**
   * Транспорт кадра. `auto` (по умолчанию) — SAB, если страница отдана с
   * COOP/COEP, иначе буферы. Задавать явно стоит ради отладки и тестов.
   */
  transport?: 'auto' | FrameTransport;
  /**
   * Где рисовать кадр. `auto` (по умолчанию) — воркер, если холст можно отдать
   * ему через `transferControlToOffscreen`, иначе main-поток.
   */
  renderMode?: RenderMode;
  /** Начальный зум. */
  zoom?: number;
  showGrid?: boolean;
  showHeaders?: boolean;
  /**
   * Тёмное оформление листа: цвета темы книги и контраст текста считаются
   * под тёмный фон. По умолчанию светлое.
   */
  theme?: 'light' | 'dark';
  /**
   * Прогресс стримового открытия (ADR-0031): вызывается на каждый
   * отправленный чанк. `total` — размер из `open(source, { size })`, если
   * вызывающий его знает; путь байтов колбэк не трогает.
   */
  onProgress?: (loaded: number, total?: number) => void;
  /**
   * Минимальная a11y-обвязка: canvas скрыт от скринридеров, область получает
   * `role=document`.
   */
  a11y?: boolean;
}

/** Форматное состояние книги: что отличает её от DOCX/ODT/PPTX/ODP. */
interface XlsbState {
  sheets: SheetInfo[];
  sheetIndex: number;
  showGrid: boolean;
  showHeaders: boolean;
  theme: 'light' | 'dark';
}

/**
 * Вьюер XLSB: холст над областью прокрутки.
 *
 * Разметка та же, что у XLSX: прокручиваемая область, распорка по размеру листа
 * и холст, приклеенный к видимой части трансформом. PDF-экспорт, hit-test,
 * hyperlink-at и правка ячейки в этом срезе не реализованы.
 */
export async function createXlsbViewer(
  host: HTMLElement,
  options: XlsbViewerOptions = {},
): Promise<XlsbViewerHandle> {
  /** Окно в единицах рендера: прокрутка браузера — в пикселях экрана, а раскладка — до зума. */
  function viewport(st: ViewerState<XlsbState>): Viewport {
    return {
      x: st.scroller.scrollLeft / st.zoom,
      y: st.scroller.scrollTop / st.zoom,
      w: st.scroller.clientWidth * st.dpr,
      h: st.scroller.clientHeight * st.dpr,
      scale: st.zoom * st.dpr,
    };
  }

  /** Настройки рисования, общие для кадра. */
  function renderConfig(st: ViewerState<XlsbState>): RenderConfig {
    const d = st.data;
    return { showGrid: d.showGrid, showHeaders: d.showHeaders, theme: d.theme, dpr: st.dpr };
  }

  const spec: ViewerSpec<XlsbState, XlsbViewerOptions> = {
    workerName: 'doc-converter-xlsb',
    workerUrl: ({ workerUrl }) => workerUrl ?? new URL('../worker/xlsb_worker.js', import.meta.url),
    telemetry: 'xlsb',
    formatTag: 'xlsb',
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
      return { type: 'open', id, format: 'xlsb', bytes } satisfies InMsg;
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
    initOptions: (opts) => ({ xlsb: opts.xlsbWasm }),
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
    extraInit(st) {
      // Доступная книга: область прокрутки отвечает скринридеру как документ.
      if (options.a11y === true) st.scroller.setAttribute('role', 'document');
    },
  };

  const base = await createViewerBase<XlsbState, XlsbViewerOptions>(host, spec, options);
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

    exportPng: base.exportPng,

    onTick: base.onTick,
    destroy: base.destroy,
  };
}
