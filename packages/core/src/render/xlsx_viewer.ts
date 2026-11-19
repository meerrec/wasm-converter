import type {
  CellAddress,
  CellRange,
  CellRect,
  FrameTransport,
  HyperlinkInfo,
  InMsg,
  OutMsg,
  PaintStats,
  PdfModuleUrls,
  PdfOptions,
  RenderConfig,
  RenderMode,
  SheetInfo,
  Viewport,
  XlsxSelection,
} from '../protocol.js';
import { createLogicalScroll, type LogicalScroll } from './logical_scroll.js';
import { createSelectionController, type SelectionController } from './selection_controller.js';
import {
  createSelection,
  publicSelection,
  selectionName,
  sheetSelection,
  type SelectionState,
} from './selection.js';
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
  /**
   * Ячейка под точкой события мыши, в координатах страницы.
   *
   * Промис отклоняется, если вьюер уничтожен или воркер упал: ответа в этом
   * случае не будет.
   */
  hitTest(clientX: number, clientY: number): Promise<[number, number] | null>;
  /**
   * Гиперссылка под точкой события мыши, в координатах страницы.
   *
   * Промис отклоняется, если вьюер уничтожен или воркер упал.
   */
  hyperlinkAt(clientX: number, clientY: number): Promise<HyperlinkInfo | null>;
  /**
   * Заменить текст ячейки сырым вводом. Правка живёт до следующего открытия,
   * видна в кадре и попадает в PDF-экспорт; кадр перерисовывается сразу.
   */
  setCellText(sheet: number, row: number, col: number, text: string): void;
  /** Текущее выделение активного листа либо `null` после `clearSelection`. */
  getSelection(): XlsxSelection | null;
  /** Выбрать адрес/диапазон A1, полные строки или столбцы и показать его. */
  setSelection(
    reference: string | CellRange,
    options?: { add?: boolean; scrollIntoView?: boolean },
  ): Promise<void>;
  /** Восстановить снимок после повторного открытия той же книги. */
  restoreSelection(selection: XlsxSelection): Promise<void>;
  /** Убрать выделение без изменения книги. */
  clearSelection(): void;
  /** Прокрутить минимально, чтобы ячейка или её объединение стало видимым. */
  scrollCellIntoView(address: CellAddress): Promise<void>;
  /** Изменения активной ячейки или диапазонов. */
  onSelectionChange(handler: (selection: XlsxSelection) => void): () => void;
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
interface XlsxRequest {
  responseType: OutMsg['type'];
  resolve: (msg: OutMsg) => void;
  reject: (error: Error) => void;
}

interface XlsxState {
  sheets: SheetInfo[];
  sheetIndex: number;
  scroll: LogicalScroll;
  showGrid: boolean;
  showHeaders: boolean;
  theme: 'light' | 'dark';
  selections: Map<number, SelectionState>;
  selectionHandlers: Set<(selection: XlsxSelection) => void>;
  requests: Map<number, XlsxRequest>;
  controller: SelectionController | null;
  liveRegion: HTMLElement | null;
  activeCell: HTMLElement | null;
}

let nextActiveCellId = 0;

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
  function currentSelection(st: ViewerState<XlsxState>): SelectionState | null {
    return st.data.selections.get(st.data.sheetIndex) ?? null;
  }

  function setSelectionState(st: ViewerState<XlsxState>, next: SelectionState | null): void {
    if (next) st.data.selections.set(st.data.sheetIndex, next);
    else st.data.selections.delete(st.data.sheetIndex);
    const label = next ? `Текущая ячейка ${selectionName(next)}` : 'Выделение очищено';
    st.scroller.setAttribute('aria-label', `Лист Excel. ${label}`);
    if (st.data.liveRegion) st.data.liveRegion.textContent = label;
    if (st.data.activeCell) {
      st.data.activeCell.textContent = next ? selectionName(next) : '—';
      st.data.activeCell.setAttribute('aria-label', label);
    }
    if (next) {
      const snapshot = publicSelection(next);
      for (const handler of st.data.selectionHandlers) handler(snapshot);
    }
    st.schedule();
  }

  function request<T>(
    st: ViewerState<XlsxState>,
    responseType: OutMsg['type'],
    message: (id: number) => InMsg,
    unwrap: (msg: OutMsg) => T,
  ): Promise<T> {
    if (st.destroyed) return Promise.reject(new Error('viewer destroyed'));
    const id = st.nextRequestId++;
    return new Promise<T>((resolve, reject) => {
      st.data.requests.set(id, {
        responseType,
        resolve: (msg) => resolve(unwrap(msg)),
        reject,
      });
      st.worker.postMessage(message(id));
    });
  }

  function rejectRequests(st: ViewerState<XlsxState>, error: Error, id?: number): void {
    if (id !== undefined) {
      const request = st.data.requests.get(id);
      st.data.requests.delete(id);
      request?.reject(error);
      return;
    }
    for (const request of st.data.requests.values()) request.reject(error);
    st.data.requests.clear();
  }

  function extraMsg(st: ViewerState<XlsxState>, msg: OutMsg): boolean {
    if (msg.type === 'error') {
      rejectRequests(st, new Error(msg.message), msg.id);
      return false;
    }
    if (!('id' in msg)) return false;
    const request = st.data.requests.get(msg.id);
    if (!request || request.responseType !== msg.type) return false;
    st.data.requests.delete(msg.id);
    request.resolve(msg);
    return true;
  }

  function scrollCss(st: ViewerState<XlsxState>, x: number, y: number): void {
    const [left, top] = st.data.scroll.scrollTo(x, y);
    st.scroller.scrollLeft = left;
    st.scroller.scrollTop = top;
    st.refresh();
  }

  function cellRect(st: ViewerState<XlsxState>, active: CellAddress): Promise<CellRect> {
    return request(st, 'cell-rect', (id) => ({
      type: 'cell-rect', id, sheet: st.data.sheetIndex, active,
    }), (msg) => {
      if (msg.type !== 'cell-rect') throw new Error('unexpected cell-rect response');
      return msg.rect;
    });
  }

  async function ensureVisible(st: ViewerState<XlsxState>, active: CellAddress): Promise<void> {
    const rect = await cellRect(st, active);
    const sheet = st.data.sheets[st.data.sheetIndex];
    if (!sheet) return;
    const headerX = st.data.showHeaders ? 44 : 0;
    const headerY = st.data.showHeaders ? 20 : 0;
    const availableW = Math.max(1, st.scroller.clientWidth / st.zoom - headerX - sheet.frozenWidth);
    const availableH = Math.max(1, st.scroller.clientHeight / st.zoom - headerY - sheet.frozenHeight);
    let x = st.data.scroll.x;
    let y = st.data.scroll.y;
    const currentX = x / st.zoom;
    const currentY = y / st.zoom;
    const left = sheet.frozenWidth + currentX;
    const top = sheet.frozenHeight + currentY;
    const right = left + availableW;
    const bottom = top + availableH;
    let layoutX = currentX;
    let layoutY = currentY;
    if (rect.x >= sheet.frozenWidth) {
      if (rect.x < left) layoutX = Math.max(0, rect.x - sheet.frozenWidth);
      if (rect.x + rect.width > right) {
        layoutX = Math.max(0, rect.x + rect.width - sheet.frozenWidth - availableW);
      }
    }
    if (rect.y >= sheet.frozenHeight) {
      if (rect.y < top) layoutY = Math.max(0, rect.y - sheet.frozenHeight);
      if (rect.y + rect.height > bottom) {
        layoutY = Math.max(0, rect.y + rect.height - sheet.frozenHeight - availableH);
      }
    }
    if (layoutX !== currentX || layoutY !== currentY) {
      scrollCss(st, layoutX * st.zoom, layoutY * st.zoom);
    }
  }

  /** Окно в единицах рендера: прокрутка браузера — в пикселях экрана, а раскладка — до зума. */
  function viewport(st: ViewerState<XlsxState>): Viewport {
    st.data.scroll.syncFromDom(st.scroller.scrollLeft, st.scroller.scrollTop);
    return {
      x: st.data.scroll.x / st.zoom,
      y: st.data.scroll.y / st.zoom,
      w: st.scroller.clientWidth * st.dpr,
      h: st.scroller.clientHeight * st.dpr,
      scale: st.zoom * st.dpr,
    };
  }

  /** Настройки рисования, общие для кадра и запросов по точке. */
  function renderConfig(st: ViewerState<XlsxState>): RenderConfig {
    const d = st.data;
    const selection = currentSelection(st);
    return {
      showGrid: d.showGrid,
      showHeaders: d.showHeaders,
      theme: d.theme,
      dpr: st.dpr,
      ...(selection ? { selection: sheetSelection(selection) } : {}),
    };
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
    if (st.destroyed) return Promise.reject(new Error('viewer destroyed'));
    const [x, y] = canvasPoint(st, clientX, clientY);
    const id = st.nextRequestId++;
    return new Promise<[number, number] | null>((resolve, reject) => {
      st.pendingHit.set(id, { resolve, reject });
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
    if (st.destroyed) return Promise.reject(new Error('viewer destroyed'));
    const [x, y] = canvasPoint(st, clientX, clientY);
    const id = st.nextRequestId++;
    return new Promise<HyperlinkInfo | null>((resolve, reject) => {
      st.pendingHyperlink.set(id, { resolve, reject });
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
      rejectRequests(st, new Error('workbook reopened'));
      st.data.selections.clear();
      st.data.sheets = msg.sheets;
      setSelectionState(st, createSelection(st.data.sheetIndex));
      st.data.scroll.scrollTo(0, 0);
      st.applyLayout();
      st.schedule();
    },
    applyLayout(st) {
      const sheet = st.data.sheets[st.data.sheetIndex];
      if (!sheet) return;
      st.data.scroll.configure(
        sheet.width,
        sheet.height,
        st.zoom,
        st.scroller.clientWidth,
        st.scroller.clientHeight,
      );
      st.spacer.style.width = `${st.data.scroll.width}px`;
      st.spacer.style.height = `${st.data.scroll.height}px`;
      const [left, top] = st.data.scroll.scrollTo(st.data.scroll.x, st.data.scroll.y);
      if (st.scroller.scrollLeft !== left) st.scroller.scrollLeft = left;
      if (st.scroller.scrollTop !== top) st.scroller.scrollTop = top;
    },
    onScroll(st) {
      st.data.scroll.syncFromDom(st.scroller.scrollLeft, st.scroller.scrollTop);
    },
    extraMsg,
    viewport,
    openMsg(_st, id, bytes) {
      return { type: 'open', id, format: 'xlsx', bytes } satisfies InMsg;
    },
    createState(opts) {
      return {
        sheets: [],
        sheetIndex: 0,
        scroll: createLogicalScroll(),
        showGrid: opts.showGrid ?? true,
        showHeaders: opts.showHeaders ?? true,
        theme: opts.theme ?? 'light',
        selections: new Map(),
        selectionHandlers: new Set(),
        requests: new Map(),
        controller: null,
        liveRegion: null,
        activeCell: null,
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
      st.scroller.tabIndex = 0;
      st.scroller.setAttribute('role', 'grid');
      st.scroller.setAttribute('aria-multiselectable', 'true');
      st.scroller.setAttribute('aria-label', 'Лист Excel');
      canvas.setAttribute('aria-hidden', 'true');
      const activeRow = document.createElement('div');
      activeRow.setAttribute('role', 'row');
      activeRow.style.cssText = 'position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);';
      const activeCell = document.createElement('span');
      activeCell.setAttribute('role', 'gridcell');
      activeCell.setAttribute('aria-selected', 'true');
      activeCell.id = `xlsx-active-cell-${++nextActiveCellId}`;
      activeCell.tabIndex = -1;
      activeRow.append(activeCell);
      st.scroller.append(activeRow);
      st.scroller.setAttribute('aria-activedescendant', activeCell.id);
      st.data.activeCell = activeCell;

      const live = document.createElement('span');
      live.style.cssText = 'position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap;';
      live.setAttribute('role', 'status');
      live.setAttribute('aria-live', 'polite');
      st.host.append(live);
      st.data.liveRegion = live;

      st.data.controller = createSelectionController({
        getSelection: () => currentSelection(st),
        setSelection: (next) => setSelectionState(st, next),
        hitTest: (clientX, clientY) => {
          const [x, y] = canvasPoint(st, clientX, clientY);
          return request(st, 'sheet-hit', (id) => ({
            type: 'sheet-hit-test', id, sheet: st.data.sheetIndex, x, y,
            viewport: viewport(st), config: renderConfig(st),
          }), (msg) => {
            if (msg.type !== 'sheet-hit') throw new Error('unexpected sheet-hit response');
            return msg.hit;
          });
        },
        normalize: (selection) => request(st, 'selection-normalized', (id) => ({
          type: 'normalize-selection', id, sheet: st.data.sheetIndex, selection,
        }), (msg) => {
          if (msg.type !== 'selection-normalized') throw new Error('unexpected normalization response');
          return msg.selection;
        }),
        navigate: (active, command) => request(st, 'selection-navigated', (id) => ({
          type: 'navigate-selection', id, sheet: st.data.sheetIndex, active, command,
        }), (msg) => {
          if (msg.type !== 'selection-navigated') throw new Error('unexpected navigation response');
          return msg.active;
        }),
        currentRegion: (active) => request(st, 'current-region', (id) => ({
          type: 'current-region', id, sheet: st.data.sheetIndex, active,
        }), (msg) => {
          if (msg.type !== 'current-region') throw new Error('unexpected region response');
          return msg.range;
        }),
        pageMove: async (active, horizontal, backward) => {
          const command = horizontal
            ? backward ? 'page-left' : 'page-right'
            : backward ? 'page-up' : 'page-down';
          return request(st, 'selection-navigated', (id) => ({
            type: 'navigate-selection', id, sheet: st.data.sheetIndex, active, command,
            viewport: viewport(st), config: renderConfig(st),
          }), (msg) => {
            if (msg.type !== 'selection-navigated') throw new Error('unexpected page response');
            return msg.active;
          });
        },
        ensureVisible: (active) => ensureVisible(st, active),
        scrollBy: (x, y) => scrollCss(st, st.data.scroll.x + x, st.data.scroll.y + y),
        focus: () => st.scroller.focus({ preventScroll: true }),
      }, canvas, st.scroller);
      st.disposers.add(() => {
        st.data.controller?.destroy();
        st.data.controller = null;
        rejectRequests(st, new Error('viewer destroyed'));
      });

      // Клик по холсту: ссылку ищем там же, где ячейку для подсказки адреса.
      // Книги может не быть вовсе — тогда и спрашивать нечего.
      const onCanvasClick = (ev: MouseEvent): void => {
        const callback = options.onHyperlink;
        if (!callback || st.destroyed || st.data.sheets.length === 0) return;
        void hyperlinkAt(st, ev.clientX, ev.clientY)
          .then((link) => {
            // Клик по ячейке без ссылки — обычный клик, колбэк молчит.
            if (link) callback(link);
          })
          .catch((error: unknown) => {
            // После destroy запрос отклоняется штатно: обрабатывать клик уже
            // некому. Остальные отказы видны в консоли, как и ошибки ядра.
            if (!st.destroyed) console.error('[xlsx] hyperlink-at', error);
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
      const selection = currentSelection(st) ?? createSelection(index);
      setSelectionState(st, selection);
      st.data.scroll.scrollTo(0, 0);
      st.scroller.scrollTop = 0;
      st.scroller.scrollLeft = 0;
      st.applyLayout();
      st.refresh();
    },

    setZoom(next: number): void {
      base.setZoom(next);
      const active = currentSelection(st)?.active;
      if (active) void ensureVisible(st, active).catch(() => undefined);
    },
    scrollTo(x: number, y: number): void {
      const [left, top] = st.data.scroll.scrollTo(x * st.zoom, y * st.zoom);
      st.scroller.scrollLeft = left;
      st.scroller.scrollTop = top;
      st.refresh();
    },

    setView(next: { showGrid?: boolean; showHeaders?: boolean; theme?: 'light' | 'dark' }): void {
      if (next.showGrid !== undefined) st.data.showGrid = next.showGrid;
      if (next.showHeaders !== undefined) st.data.showHeaders = next.showHeaders;
      if (next.theme !== undefined) st.data.theme = next.theme;
      st.schedule();
    },

    hitTest: (clientX: number, clientY: number) => hitTest(st, clientX, clientY),
    hyperlinkAt: (clientX: number, clientY: number) => hyperlinkAt(st, clientX, clientY),

    setCellText(sheet: number, row: number, col: number, text: string): void {
      // Книги нет — править нечего; воркер всё равно ответил бы ошибкой.
      if (st.destroyed || st.data.sheets.length === 0) return;
      st.worker.postMessage({ type: 'edit-cell', sheet, row, col, text } satisfies InMsg);
      // Кадр перерисовывается сразу: сообщения воркер читает по порядку, и
      // правка ляжет в книгу раньше следующего `render`.
      st.schedule();
    },

    getSelection(): XlsxSelection | null {
      const selection = currentSelection(st);
      if (!selection) return null;
      return {
        ...publicSelection(selection),
        active: { ...selection.active },
        ranges: selection.ranges.map((range) => ({ start: { ...range.start }, end: { ...range.end } })),
      };
    },
    setSelection(reference, options) {
      if (!st.data.controller) return Promise.reject(new Error('viewer destroyed'));
      return st.data.controller.setSelection(reference, options?.add ?? false, options?.scrollIntoView ?? true);
    },
    restoreSelection(selection) {
      if (!st.data.controller) return Promise.reject(new Error('viewer destroyed'));
      return st.data.controller.restoreSelection(selection);
    },
    clearSelection(): void {
      st.data.controller?.clearSelection();
    },
    scrollCellIntoView(active: CellAddress): Promise<void> {
      return ensureVisible(st, active);
    },
    onSelectionChange(handler) {
      st.data.selectionHandlers.add(handler);
      return () => st.data.selectionHandlers.delete(handler);
    },

    exportPng: base.exportPng,
    exportPdf: base.exportPdf,

    onTick: base.onTick,
    destroy: base.destroy,
  };
}
