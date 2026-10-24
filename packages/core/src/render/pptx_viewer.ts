import type {
  FrameTransport,
  InMsg,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  PptxModuleUrls,
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
   * Где лежит ленивый PPTX-PDF-модуль. Воркер грузит его при первом экспорте в
   * PDF; не задан — кнопка экспорта получит внятную ошибку вместо файла.
   * Просмотрщик и экспортёр PPTX разнесены по модулям, как у DOCX (ADR-0033).
   */
  pptxPdfWasm?: PdfModuleUrls;
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

/** Форматное состояние презентации: слайд и его размер. */
interface PptxState {
  slides: SlideInfo[];
  slideIndex: number;
  slideWidth: number;
  slideHeight: number;
}

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
  /**
   * Окно в единицах рендера: `x`/`y` — пиксели раскладки, `w`/`h` — физические
   * пиксели canvas, `scale` — зум, умноженный на DPR (как у XLSX).
   *
   * `x`/`y` отсчитываются от левого верхнего угла слайда: кадр несёт один
   * слайд, и его начало — нуль раскладки в Rust. Поле слева, которого при
   * центрировании на холст не видно, уводит `x` в минус.
   */
  function viewport(st: ViewerState<PptxState>): Viewport {
    return {
      x: (st.scroller.scrollLeft - st.padLeft) / st.zoom,
      y: st.scroller.scrollTop / st.zoom,
      w: st.scroller.clientWidth * st.dpr,
      h: st.scroller.clientHeight * st.dpr,
      scale: st.zoom * st.dpr,
    };
  }

  /**
   * Настройки кадра. Кадру PPTX они не нужны — воркер в этой ветке их не
   * читает, — но поле обязательно по типу `RenderRequest`.
   */
  function frameConfig(st: ViewerState<PptxState>): RenderConfig {
    return { showGrid: false, showHeaders: false, theme: 'light', dpr: st.dpr };
  }

  const spec: ViewerSpec<PptxState, PptxViewerOptions> = {
    workerName: 'doc-converter-pptx',
    workerUrl: ({ workerUrl }) => workerUrl ?? new URL('../worker/pptx_worker.js', import.meta.url),
    telemetry: 'pptx',
    formatTag: 'pptx',
    openedResponse: 'pptx-opened',
    onOpened(st, msg) {
      if (msg.type !== 'pptx-opened') return;
      st.data.slides = msg.slides;
      st.data.slideIndex = 0;
      st.applyLayout();
      st.schedule();
    },
    applyLayout(st) {
      // Поле считается по ширине области прокрутки, а не холста: разметка
      // прокручивается, и ориентироваться ей есть на что только здесь. Слайд
      // шире области — поля нет вовсе; уже — распорка получает слайд плюс оба
      // поля, и слайд стоит посередине.
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
      return { type: 'open', id, format: 'pptx', bytes } satisfies InMsg;
    },
    createState() {
      return { slides: [], slideIndex: 0, slideWidth: 0, slideHeight: 0 };
    },
    initOptions: (opts) => ({ pptx: opts.pptxWasm, pptxPdf: opts.pptxPdfWasm }),
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

  const base = await createViewerBase<PptxState, PptxViewerOptions>(host, spec, options);
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
    exportPdf: base.exportPdf,

    exportMarkdown: (opts?: { signal?: AbortSignal }) => base.exportText('markdown', opts),
    exportHtml: (opts?: { signal?: AbortSignal }) => base.exportText('html', opts),
    exportText: (opts?: { signal?: AbortSignal }) => base.exportText('text', opts),

    onTick: base.onTick,
    destroy: base.destroy,
  };
}
