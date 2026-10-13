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
  Viewport,
} from '../protocol.js';
import { computeDpr } from './resize_observer.js';

/** Что умеет открытый документ. */
export interface DocxViewerHandle {
  /**
   * Каким путём воркер возит кадры: SAB, если страница отдана с COOP/COEP,
   * иначе буферы. Функциональность от этого не зависит — поле для диагностики.
   */
  readonly transport: FrameTransport;
  /** Размеры страниц в пикселях раскладки, как их видит рендер. */
  readonly pages: PageInfo[];
  /** Сколько страниц в документе. */
  readonly pageCount: number;
  /** Номер видимой страницы — той, что занимает верх области прокрутки. */
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
  /**
   * Экспорт документа в PDF. Без настроек — страницы раскладки со своими
   * размерами и полями из `w:sectPr`.
   *
   * `options` — тот же `PdfOptions`, что у XLSX-пути, но читаются не все поля:
   * `pageSize`, `orientation`, `margins`, `scale` и `sheetIndex` не влияют,
   * страницы берутся из документа.
   */
  exportPdf(options?: PdfOptions): Promise<Uint8Array>;
  /** Экспорт документа в Markdown: заголовки, списки, таблицы, ссылки. */
  exportMarkdown(): Promise<string>;
  /** Экспорт документа в HTML — самостоятельную страницу со встроенными стилями. */
  exportHtml(): Promise<string>;
  /** Экспорт документа в обычный текст: содержимое без разметки. */
  exportText(): Promise<string>;
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
  /** Начальный зум. */
  zoom?: number;
  /**
   * Зазор между страницами в пикселях раскладки. Он же уходит в кадр: по нему
   * Rust ставит страницы стопкой, и стопка в кадре обязана совпасть с распоркой.
   */
  pageGap?: number;
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
  /** Поле слева от стопки на экране: им она центрируется по горизонтали. */
  let padLeft = 0;
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
  const pendingPdf = new Map<
    number,
    { resolve: (bytes: Uint8Array) => void; reject: (e: Error) => void }
  >();
  const pendingText = new Map<
    number,
    { resolve: (text: string) => void; reject: (e: Error) => void }
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
        pageIndex = pageAt(scroller.scrollTop);
        for (const p of pendingOpen) p.resolve(msg.pages);
        pendingOpen.clear();
        schedule();
        break;
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
      case 'error':
        for (const p of pendingOpen) p.reject(new Error(msg.message));
        pendingOpen.clear();
        // У ошибки нет id: отклоняем весь экспорт — незавершённый повис бы навсегда.
        for (const p of pendingPng.values()) p.reject(new Error(msg.message));
        pendingPng.clear();
        for (const p of pendingPdf.values()) p.reject(new Error(msg.message));
        pendingPdf.clear();
        for (const p of pendingText.values()) p.reject(new Error(msg.message));
        pendingText.clear();
        console.error('[docx]', msg.message);
        break;
      default:
        break;
    }
  });

  const ready = await new Promise<ReadyInfo>((resolve, reject) => {
    const onMessage = (ev: MessageEvent) => {
      const data = ev.data as { type?: string; message?: string } & Partial<ReadyInfo>;
      if (data?.type === 'ready') {
        worker.removeEventListener('message', onMessage);
        resolve({
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
    const off = canvas.transferControlToOffscreen();
    worker.postMessage(
      {
        type: 'init',
        canvas: off,
        wasmUrl: options.wasmUrl,
        docx: options.docxWasm,
        pdf: options.pdfWasm,
        slotCapacity,
        transport: options.transport,
      } satisfies InMsg,
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
  function exportText(format: DocxTextFormat): Promise<string> {
    const id = nextRequestId++;
    return new Promise<string>((resolve, reject) => {
      pendingText.set(id, { resolve, reject });
      worker.postMessage({ type: 'export-docx-text', id, format } satisfies InMsg);
    });
  }

  return {
    transport: ready.transport,

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
      // Точка раскладки встаёт в левый верхний угол видимой области, а её
      // левый край отстоит от начала раскладки на поле; вертикально поля нет.
      scroller.scrollLeft = padLeft + x * zoom;
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

    exportPdf(options: PdfOptions = {}): Promise<Uint8Array> {
      const id = nextRequestId++;
      return new Promise<Uint8Array>((resolve, reject) => {
        pendingPdf.set(id, { resolve, reject });
        worker.postMessage({ type: 'export-docx-pdf', id, options } satisfies InMsg);
      });
    },

    exportMarkdown: () => exportText('markdown'),
    exportHtml: () => exportText('html'),
    exportText: () => exportText('text'),

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
      for (const p of pendingPdf.values()) p.reject(new Error('viewer destroyed'));
      pendingPdf.clear();
      for (const p of pendingText.values()) p.reject(new Error('viewer destroyed'));
      pendingText.clear();
      worker.postMessage({ type: 'close' } satisfies InMsg);
      worker.terminate();
      scroller.remove();
    },
  };
}
