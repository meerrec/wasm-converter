import type {
  FrameTransport,
  InMsg,
  OdtModuleUrls,
  PageInfo,
  PaintStats,
  RenderConfig,
  RenderMode,
  TextLayer,
  Viewport,
} from '../protocol.js';
import { createTextLayer } from './text_layer.js';
import {
  createViewerBase,
  type ViewerSpec,
  type ViewerState,
} from './viewer_core.js';

/** Что умеет открытый документ ODT. */
export interface OdtViewerHandle {
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
   * загрузки (ADR-0031). `signal` — отмена тяжёлой раскладки (ADR-0032).
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

export interface OdtViewerOptions {
  /** Готовый воркер: свой URL или уже собранный бандл. */
  workerUrl?: string | URL;
  /** URL wasm-модуля; по умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /**
   * Где лежит ленивый ODT-модуль. Воркер грузит его при открытии документа;
   * не задан — открытие получит внятную ошибку вместо страниц.
   */
  odtWasm?: OdtModuleUrls;
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
  /**
   * Доступный документ: прозрачный DOM-слой поверх canvas с выделением текста
   * и ARIA-деревом. По умолчанию выключен.
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

const DEFAULT_PAGE_GAP = 16;
/**
 * Допуск на округление прокрутки браузером, экранные пиксели. Округление вниз
 * теряет меньше пикселя, так что этот допуск покрывает и его, и округление к
 * ближайшему.
 */
const SCROLL_EPSILON_PX = 1;

/** Форматное состояние документа: стопка страниц и её геометрия. */
interface OdtState {
  pages: PageInfo[];
  /** Верх каждой страницы в пикселях раскладки — считается при открытии. */
  pageTops: number[];
  stackWidth: number;
  stackHeight: number;
  pageIndex: number;
  pageGap: number;
}

/**
 * Вьюер ODT: холст над областью прокрутки, страницы — стопкой по вертикали.
 *
 * Разметка та же, что у DOCX: прокручиваемая область, распорка по высоте стопки
 * и холст, приклеенный к видимой части трансформом. PDF-экспорт и правка текста
 * в этом срезе не реализованы.
 */
export async function createOdtViewer(
  host: HTMLElement,
  options: OdtViewerOptions = {},
): Promise<OdtViewerHandle> {
  /** Разложить страницы в стопку: верх каждой и общий размер в пикселях раскладки. */
  function layoutStack(st: ViewerState<OdtState>): void {
    const d = st.data;
    d.pageTops = [];
    d.stackWidth = 0;
    let y = 0;
    for (const page of d.pages) {
      d.pageTops.push(y);
      y += page.height + d.pageGap;
      d.stackWidth = Math.max(d.stackWidth, page.width);
    }
    // Зазор нужен только между страницами — за последней его нет.
    d.stackHeight = d.pages.length === 0 ? 0 : y - d.pageGap;
  }

  /** Верх страницы в пикселях раскладки; у номера вне документа — нуль. */
  function pageTop(st: ViewerState<OdtState>, index: number): number {
    return st.data.pageTops[index] ?? 0;
  }

  /**
   * Принять новые размеры страниц: перестроить стопку, переустановить распорку
   * и слой, вернуть индекс видимой страницы.
   */
  function applyPages(st: ViewerState<OdtState>, pages: PageInfo[]): void {
    st.data.pages = pages;
    layoutStack(st);
    st.applyLayout();
    st.textLayer?.configure(pages, st.data.pageTops);
    st.data.pageIndex = pageAt(st, st.scroller.scrollTop);
    st.schedule();
  }

  /**
   * Страница, занимающая верх видимой области: последняя, чей верх не ниже
   * прокрутки. В зазоре между страницами это страница выше.
   */
  function pageAt(st: ViewerState<OdtState>, scrollPx: number): number {
    const d = st.data;
    let index = 0;
    for (let i = 1; i < d.pageTops.length; i++) {
      const top = d.pageTops[i];
      if (top === undefined || top * st.zoom > scrollPx + SCROLL_EPSILON_PX) break;
      index = i;
    }
    return index;
  }

  /**
   * Окно в единицах рендера: `x`/`y` — пиксели раскладки, `w`/`h` — физические
   * пиксели canvas, `scale` — зум, умноженный на DPR (как у XLSX).
   */
  function viewport(st: ViewerState<OdtState>): Viewport {
    return {
      x: (st.scroller.scrollLeft - st.padLeft) / st.zoom,
      y: st.scroller.scrollTop / st.zoom,
      w: st.scroller.clientWidth * st.dpr,
      h: st.scroller.clientHeight * st.dpr,
      scale: st.zoom * st.dpr,
    };
  }

  /**
   * Настройки кадра. Кадру ODT они не нужны — воркер в этой ветке их не
   * читает, — но поле обязательно по типу `RenderRequest`.
   */
  function frameConfig(st: ViewerState<OdtState>): RenderConfig {
    return { showGrid: false, showHeaders: false, theme: 'light', dpr: st.dpr };
  }

  const spec: ViewerSpec<OdtState, OdtViewerOptions> = {
    workerName: 'doc-converter-odt',
    workerUrl: ({ workerUrl }) => workerUrl ?? new URL('../worker/odt_worker.js', import.meta.url),
    telemetry: 'odt',
    formatTag: 'odt',
    openedResponse: 'docx-opened',
    onOpened(st, msg) {
      if (msg.type !== 'docx-opened') return;
      applyPages(st, msg.pages);
    },
    applyLayout(st) {
      const d = st.data;
      // Поле считается по ширине области прокрутки, а не холста: разметка
      // прокручивается, и ориентироваться ей есть на что только здесь. Стопка
      // шире области — поля нет вовсе; уже — распорка получает стопку плюс оба
      // поля, и стопка стоит посередине.
      st.padLeft = Math.max(0, (st.scroller.clientWidth - d.stackWidth * st.zoom) / 2);
      st.spacer.style.width = `${d.stackWidth * st.zoom + 2 * st.padLeft}px`;
      st.spacer.style.height = `${d.stackHeight * st.zoom}px`;
      // Слой в layout-координатах: зум и поле — только transform, spans не
      // пересобираются и не отстают от canvas на кадр.
      st.textLayer?.setTransform(st.padLeft, st.zoom, d.stackWidth, d.stackHeight);
    },
    viewport,
    openMsg(_st, id, bytes) {
      return { type: 'open', id, format: 'odt', bytes } satisfies InMsg;
    },
    requestTextLayer(st, page) {
      if (page === null) {
        st.textLayer?.updateWindow(st.scroller.scrollTop, st.scroller.clientHeight, st.zoom);
        return;
      }
      void st.requestTextLayer(page);
    },
    extraInit(st, _canvas, scroller) {
      // Доступный слой создаётся до первого кадра: он живёт в layout-координатах
      // и наполняется после `docx-opened`.
      if (options.a11y === true) {
        st.textLayer = createTextLayer({
          fetchPage: (page) => st.requestTextLayer(page),
          fetchAll: () => st.requestTextLayer(null),
        });
        scroller.append(st.textLayer.element);
      }
    },
    extraMsg(st, msg) {
      // Ответ слоя адресный, но у ODT он один на оба запроса (страница/весь
      // документ) — формат сам находит свой ожидающий промис.
      if (msg.type !== 'text-layer') return false;
      const pending = st.pendingTextLayer.get(msg.id);
      if (pending) {
        st.pendingTextLayer.delete(msg.id);
        pending.resolve(msg.layer);
      }
      return true;
    },
    createState(opts) {
      return {
        pages: [],
        pageTops: [],
        stackWidth: 0,
        stackHeight: 0,
        pageIndex: 0,
        pageGap: opts.pageGap ?? DEFAULT_PAGE_GAP,
      };
    },
    initOptions: (opts) => ({ odt: opts.odtWasm }),
    renderMsg(st) {
      if (st.data.pages.length === 0) return null;
      st.data.pageIndex = pageAt(st, st.scroller.scrollTop);
      return {
        type: 'render',
        req: { gap: st.data.pageGap, viewport: viewport(st), config: frameConfig(st) },
      } satisfies InMsg;
    },
    exportPdfMsg(_st, id, options) {
      return { type: 'export-docx-pdf', id, options } satisfies InMsg;
    },
  };

  const base = await createViewerBase<OdtState, OdtViewerOptions>(host, spec, options);
  const st = base.state;

  return {
    transport: base.transport,
    renderMode: base.renderMode,

    get pages() {
      return st.data.pages;
    },
    get pageCount() {
      return st.data.pages.length;
    },
    get pageIndex() {
      return st.data.pageIndex;
    },
    get zoom() {
      return st.zoom;
    },

    async open(
      source: ArrayBuffer | ReadableStream<Uint8Array>,
      opts?: { size?: number; signal?: AbortSignal },
    ): Promise<PageInfo[]> {
      await base.open(source, opts);
      return st.data.pages;
    },

    showPage(index: number): void {
      if (index < 0 || index >= st.data.pages.length) return;
      st.data.pageIndex = index;
      st.scroller.scrollLeft = 0;
      st.scroller.scrollTop = pageTop(st, index) * st.zoom;
      st.refresh();
    },

    setZoom: base.setZoom,
    scrollTo: base.scrollTo,

    exportPng: base.exportPng,

    exportMarkdown: (opts?: { signal?: AbortSignal }) => base.exportText('markdown', opts),
    exportHtml: (opts?: { signal?: AbortSignal }) => base.exportText('html', opts),
    exportText: (opts?: { signal?: AbortSignal }) => base.exportText('text', opts),
    getTextLayer: (page: number) => base.getTextLayer(page),
    getFullTextLayer: () => base.getFullTextLayer(),

    onTick: base.onTick,
    destroy: base.destroy,
  };
}
