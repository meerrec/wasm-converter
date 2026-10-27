import type {
  FrameTransport,
  InMsg,
  OdpModuleUrls,
  PaintStats,
  RenderConfig,
  RenderMode,
  SlideInfo,
  Viewport,
} from '../protocol.js';
import {
  createViewerBase,
  type ViewerSpec,
  type ViewerState,
} from './viewer_core.js';

/** Что умеет открытая презентация ODP. */
export interface OdpViewerHandle {
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
   * загрузки (ADR-0031). `signal` — отмена тяжёлой раскладки (ADR-0032).
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
  /** Экспорт конспекта в Markdown: слайды заголовками, маркеры списков. */
  exportMarkdown(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Экспорт конспекта в HTML — самостоятельную страницу со встроенными стилями. */
  exportHtml(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Экспорт конспекта в обычный текст: текстовые рамки без разметки. */
  exportText(opts?: { signal?: AbortSignal }): Promise<string>;
  /** Подписка на метрики кадров. */
  onTick(handler: (stats: PaintStats) => void): () => void;
  /** Остановить воркер и убрать за собой разметку. */
  destroy(): void;
}

export interface OdpViewerOptions {
  /** Готовый воркер: свой URL или уже собранный бандл. */
  workerUrl?: string | URL;
  /** URL wasm-модуля; по умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /**
   * Где лежит ленивый ODP-модуль. Воркер грузит его при открытии презентации;
   * не задан — открытие получит внятную ошибку вместо слайдов.
   */
  odpWasm?: OdpModuleUrls;
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
  /**
   * Прогресс стримового открытия (ADR-0031): вызывается на каждый
   * отправленный чанк. `total` — размер из `open(source, { size })`, если
   * вызывающий его знает; путь байтов колбэк не трогает.
   */
  onProgress?: (loaded: number, total?: number) => void;
}

/** Форматное состояние презентации: слайд и его размер. */
interface OdpState {
  slides: SlideInfo[];
  slideIndex: number;
  slideWidth: number;
  slideHeight: number;
}

/**
 * Вьюер ODP: холст над областью прокрутки, слайд — одна страница рендера.
 *
 * Разметка та же, что у PPTX: прокручиваемая область, распорка по размеру слайда
 * с зумом и холст, приклеенный к видимой части трансформом. PDF-экспорт в этом
 * срезе не реализован — только PNG и текстовые выгрузки.
 */
export async function createOdpViewer(
  host: HTMLElement,
  options: OdpViewerOptions = {},
): Promise<OdpViewerHandle> {
  /**
   * Окно в единицах рендера: `x`/`y` — пиксели раскладки, `w`/`h` — физические
   * пиксели canvas, `scale` — зум, умноженный на DPR (как у XLSX).
   */
  function viewport(st: ViewerState<OdpState>): Viewport {
    return {
      x: (st.scroller.scrollLeft - st.padLeft) / st.zoom,
      y: st.scroller.scrollTop / st.zoom,
      w: st.scroller.clientWidth * st.dpr,
      h: st.scroller.clientHeight * st.dpr,
      scale: st.zoom * st.dpr,
    };
  }

  /**
   * Настройки кадра. Кадру ODP они не нужны — воркер в этой ветке их не
   * читает, — но поле обязательно по типу `RenderRequest`.
   */
  function frameConfig(st: ViewerState<OdpState>): RenderConfig {
    return { showGrid: false, showHeaders: false, theme: 'light', dpr: st.dpr };
  }

  const spec: ViewerSpec<OdpState, OdpViewerOptions> = {
    workerName: 'doc-converter-odp',
    workerUrl: ({ workerUrl }) => workerUrl ?? new URL('../worker/odp_worker.js', import.meta.url),
    telemetry: 'odp',
    formatTag: 'odp',
    openedResponse: 'pptx-opened',
    onOpened(st, msg) {
      if (msg.type !== 'pptx-opened') return;
      st.data.slides = msg.slides;
      st.data.slideIndex = 0;
      st.applyLayout();
      st.schedule();
    },
    applyLayout(st) {
      const d = st.data;
      const slide = d.slides[d.slideIndex];
      d.slideWidth = slide?.width ?? 0;
      d.slideHeight = slide?.height ?? 0;
      st.padLeft = Math.max(0, (st.scroller.clientWidth - d.slideWidth * st.zoom) / 2);
      st.spacer.style.width = `${d.slideWidth * st.zoom + 2 * st.padLeft}px`;
      st.spacer.style.height = `${d.slideHeight * st.zoom}px`;
    },
    viewport,
    openMsg(_st, id, bytes) {
      return { type: 'open', id, format: 'odp', bytes } satisfies InMsg;
    },
    createState() {
      return { slides: [], slideIndex: 0, slideWidth: 0, slideHeight: 0 };
    },
    initOptions: (opts) => ({ odp: opts.odpWasm }),
    renderMsg(st) {
      if (st.data.slides.length === 0) return null;
      return {
        type: 'render',
        req: { slide: st.data.slideIndex, viewport: viewport(st), config: frameConfig(st) },
      } satisfies InMsg;
    },
    exportPdfMsg(_st, id, options) {
      return { type: 'export-pptx-pdf', id, options } satisfies InMsg;
    },
  };

  const base = await createViewerBase<OdpState, OdpViewerOptions>(host, spec, options);
  const st = base.state;

  return {
    transport: base.transport,
    renderMode: base.renderMode,

    get slides() {
      return st.data.slides;
    },
    get slideCount() {
      return st.data.slides.length;
    },
    get slideIndex() {
      return st.data.slideIndex;
    },
    get zoom() {
      return st.zoom;
    },

    async open(
      source: ArrayBuffer | ReadableStream<Uint8Array>,
      opts?: { size?: number; signal?: AbortSignal },
    ): Promise<SlideInfo[]> {
      await base.open(source, opts);
      return st.data.slides;
    },

    showSlide(index: number): void {
      if (index < 0 || index >= st.data.slides.length) return;
      st.data.slideIndex = index;
      st.scroller.scrollLeft = 0;
      st.scroller.scrollTop = 0;
      st.applyLayout();
      st.refresh();
    },

    setZoom: base.setZoom,
    scrollTo: base.scrollTo,

    exportPng: base.exportPng,

    exportMarkdown: (opts?: { signal?: AbortSignal }) => base.exportText('markdown', opts),
    exportHtml: (opts?: { signal?: AbortSignal }) => base.exportText('html', opts),
    exportText: (opts?: { signal?: AbortSignal }) => base.exportText('text', opts),

    onTick: base.onTick,
    destroy: base.destroy,
  };
}
