import type {
  FrameTransport,
  HitResult,
  InMsg,
  OutMsg,
  PaintStats,
  PdfOptions,
  ReadyInfo,
  RenderRequest,
} from '../protocol.js';
import { attachResize, type RpcLike } from './resize_observer.js';

/**
 * Передаёт OffscreenCanvas воркеру ровно один раз.
 *
 * Готовность воркера — это `ready`, а не наличие в нём `SharedArrayBuffer`:
 * без COOP/COEP кадры поедут буферами, и просмотрщик от этого работать не
 * перестаёт. Воркер сам решает, каким транспортом везти кадры, и говорит об
 * этом в `ready`.
 */
export function initOffscreen(
  canvas: HTMLCanvasElement,
  worker: Worker,
  rpc: RpcLike,
  wasmUrl?: string,
  slotCapacity = 1 << 20,
  transport?: 'auto' | FrameTransport,
): Promise<ReadyInfo> {
  if (!('transferControlToOffscreen' in canvas)) {
    return Promise.reject(new Error('transferControlToOffscreen unsupported'));
  }
  const off = canvas.transferControlToOffscreen();
  return new Promise((resolve, reject) => {
    const onMsg = (ev: MessageEvent) => {
      const data = ev.data as { type?: string; message?: string } & Partial<ReadyInfo>;
      if (data?.type === 'ready' && data.transport) {
        worker.removeEventListener('message', onMsg);
        resolve({
          transport: data.transport,
          slotCapacity: data.slotCapacity ?? slotCapacity,
          totalBytes: data.totalBytes ?? 0,
          crossOriginIsolated: data.crossOriginIsolated ?? false,
        });
      } else if (data?.type === 'error') {
        worker.removeEventListener('message', onMsg);
        reject(new Error(data.message ?? 'worker init error'));
      }
    };
    worker.addEventListener('message', onMsg);
    worker.postMessage(
      { type: 'init', canvas: off, wasmUrl, slotCapacity, transport } satisfies InMsg,
      [off],
    );
  });
}

export interface ViewerHandle {
  /**
   * Каким путём воркер возит кадры. Интерфейсу это знать не нужно — оба пути
   * дают один и тот же кадр, — но для диагностики и тестов полезно.
   */
  readonly transport: FrameTransport;
  render(req: RenderRequest): Promise<void>;
  /** Фаза 4. */
  exportPdf(opts: PdfOptions): Promise<Uint8Array>;
  exportPng(opts: { dpi: number }): Promise<Uint8Array>;
  /** Фаза 3. */
  hitTest(x: number, y: number): Promise<HitResult | null>;
  onTick(handler: (stats: PaintStats) => void): () => void;
  dispose(): void;
}

export interface ViewerOptions {
  workerUrl?: string | URL;
  /** Если true — не создавать ResizeObserver (для SSR / тестов). */
  manualResize?: boolean;
  /** URL wasm-модуля. По умолчанию — рядом с бандлом воркера. */
  wasmUrl?: string;
  /** Размер одного слота ring'а в байтах. */
  slotCapacity?: number;
  /**
   * Транспорт кадра. `auto` (по умолчанию) — SAB, если страница отдана с
   * COOP/COEP, иначе буферы. Задавать явно стоит ради отладки и тестов:
   * `sab` без заголовков — ошибка инициализации, а не тихая подмена.
   */
  transport?: 'auto' | FrameTransport;
}

export function supportsOffscreen(): boolean {
  return (
    typeof OffscreenCanvas !== 'undefined' &&
    typeof HTMLCanvasElement !== 'undefined' &&
    'transferControlToOffscreen' in HTMLCanvasElement.prototype
  );
}

/** Fallback: рендер на main-thread, парсинг/PDF всё равно в воркере. */
export function assertFallbackAvailable(): void {
  if (typeof HTMLCanvasElement === 'undefined') {
    throw new Error('doc-converter requires a browser-like environment');
  }
}

export async function createViewer(
  canvas: HTMLCanvasElement,
  opts: ViewerOptions = {},
): Promise<ViewerHandle> {
  // Внятное сообщение вместо `Worker is not defined` в SSR и в тестах без DOM.
  assertFallbackAvailable();
  if (!supportsOffscreen()) {
    throw new Error('OffscreenCanvas + transferControlToOffscreen not supported');
  }

  const worker = new Worker(
    opts.workerUrl ?? new URL('../worker/worker.js', import.meta.url),
    { type: 'module', name: 'doc-converter-renderer' },
  );

  const tickHandlers = new Set<(stats: PaintStats) => void>();
  const pendingPng = new Map<
    number,
    { resolve: (b: Uint8Array) => void; reject: (e: Error) => void }
  >();
  let nextPngId = 1;

  worker.addEventListener('message', (ev: MessageEvent<OutMsg>) => {
    const msg = ev.data;
    switch (msg.type) {
      case 'tick':
        for (const h of tickHandlers) h(msg.stats);
        break;
      case 'png': {
        pendingPng.get(msg.id)?.resolve(msg.bytes);
        pendingPng.delete(msg.id);
        break;
      }
      case 'error':
        for (const p of pendingPng.values()) p.reject(new Error(msg.message));
        pendingPng.clear();
        break;
      case 'ready':
        break;
    }
  });

  // Мост к прямому протоколу воркера: attachResize работает с RpcLike.
  const rpc: RpcLike = {
    call(method, params) {
      worker.postMessage({ type: method, ...(params as object) } as InMsg);
    },
  };

  let ready: ReadyInfo;
  try {
    ready = await initOffscreen(
      canvas,
      worker,
      rpc,
      opts.wasmUrl,
      opts.slotCapacity,
      opts.transport,
    );
  } catch (e) {
    worker.terminate();
    throw e;
  }

  const detachResize = opts.manualResize ? undefined : attachResize(canvas, rpc);

  return {
    transport: ready.transport,
    render: (req) => {
      worker.postMessage({ type: 'render', req } satisfies InMsg);
      return Promise.resolve();
    },
    exportPdf: () =>
      Promise.reject(new Error('exportPdf: not yet implemented (Фаза 4)')),
    exportPng: () => {
      const id = nextPngId++;
      return new Promise<Uint8Array>((resolve, reject) => {
        pendingPng.set(id, { resolve, reject });
        worker.postMessage({ type: 'export-png', id } satisfies InMsg);
      });
    },
    hitTest: () =>
      Promise.reject(new Error('hitTest: not yet implemented (Фаза 3)')),
    onTick: (handler) => {
      tickHandlers.add(handler);
      return () => tickHandlers.delete(handler);
    },
    dispose: () => {
      detachResize?.();
      tickHandlers.clear();
      pendingPng.clear();
      worker.terminate();
    },
  };
}
