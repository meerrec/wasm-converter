import type { InMsg, OutMsg, PaintStats, SheetInfo } from '../protocol.js';
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
  /** Показать или скрыть сетку и заголовки. */
  setView(options: { showGrid?: boolean; showHeaders?: boolean }): void;
  /** Ячейка под точкой события мыши, в координатах страницы. */
  hitTest(clientX: number, clientY: number): Promise<[number, number] | null>;
  /** Текущий кадр картинкой PNG — им проверяют, что нарисовалось. */
  exportPng(): Promise<Uint8Array>;
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
  /** Размер слота ring'а в байтах. */
  slotCapacity?: number;
  /** Начальный зум. */
  zoom?: number;
  showGrid?: boolean;
  showHeaders?: boolean;
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
  let destroyed = false;
  let frameScheduled = false;
  const pendingOpen = new Map<number, { resolve: (s: SheetInfo[]) => void; reject: (e: Error) => void }>();
  const pendingHit = new Map<number, { resolve: (c: [number, number] | null) => void }>();
  const pendingPng = new Map<number, (bytes: Uint8Array) => void>();
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
      case 'png': {
        pendingPng.get(msg.id)?.(msg.bytes);
        pendingPng.delete(msg.id);
        break;
      }
      case 'error':
        for (const p of pendingOpen.values()) p.reject(new Error(msg.message));
        pendingOpen.clear();
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
    worker.postMessage({ type: 'init', canvas: off, wasmUrl: options.wasmUrl, slotCapacity } satisfies InMsg, [
      off,
    ]);
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

  function render(): void {
    if (sheets.length === 0) return;
    worker.postMessage({
      type: 'render',
      req: {
        sheet: sheetIndex,
        viewport: {
          // Прокрутка браузера — в пикселях экрана, раскладка — до зума.
          x: scroller.scrollLeft / zoom,
          y: scroller.scrollTop / zoom,
          w: scroller.clientWidth * dpr,
          h: scroller.clientHeight * dpr,
          scale: zoom * dpr,
        },
        config: { showGrid, showHeaders, theme: 'light', dpr },
      },
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
      schedule();
    },

    hitTest(clientX: number, clientY: number): Promise<[number, number] | null> {
      const id = nextRequestId++;
      const rect = canvas.getBoundingClientRect();
      const x = (clientX - rect.left) * dpr;
      const y = (clientY - rect.top) * dpr;
      return new Promise<[number, number] | null>((resolve) => {
        pendingHit.set(id, { resolve });
        worker.postMessage({
          type: 'hit-test',
          id,
          sheet: sheetIndex,
          x,
          y,
          viewport: {
            x: scroller.scrollLeft / zoom,
            y: scroller.scrollTop / zoom,
            w: scroller.clientWidth * dpr,
            h: scroller.clientHeight * dpr,
            scale: zoom * dpr,
          },
          config: { showGrid, showHeaders, theme: 'light', dpr },
        } satisfies InMsg);
      });
    },

    exportPng(): Promise<Uint8Array> {
      const id = nextRequestId++;
      return new Promise<Uint8Array>((resolve) => {
        pendingPng.set(id, resolve);
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
      worker.postMessage({ type: 'close' } satisfies InMsg);
      worker.terminate();
      scroller.remove();
    },
  };
}
