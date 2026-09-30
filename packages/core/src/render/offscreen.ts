import { createRpc, type RpcHandle } from '../rpc.js';
import type { HitResult, PdfOptions, RenderConfig, Viewport, WorkerResponse } from '../protocol.js';

export interface ViewerHandle {
  render(req: { sheet?: number; page?: number; viewport: Viewport; config: RenderConfig }): Promise<void>;
  exportPdf(opts: PdfOptions): Promise<Uint8Array>;
  exportPng(opts: { dpi: number }): Promise<Uint8Array>;
  hitTest(x: number, y: number): Promise<HitResult | null>;
  onTick(handler: (msg: Extract<WorkerResponse, { type: 'tick' }>) => void): () => void;
  dispose(): void;
}

export interface ViewerOptions {
  workerUrl?: string | URL;
  /** Если true — не создавать ResizeObserver (для SSR / тестов). */
  manualResize?: boolean;
}

export function supportsOffscreen(): boolean {
  return (
    typeof OffscreenCanvas !== 'undefined' &&
    typeof HTMLCanvasElement !== 'undefined' &&
    'transferControlToOffscreen' in HTMLCanvasElement.prototype
  );
}

export async function createViewer(
  canvas: HTMLCanvasElement,
  opts: ViewerOptions = {},
): Promise<ViewerHandle> {
  if (!supportsOffscreen()) {
    throw new Error('OffscreenCanvas + transferControlToOffscreen not supported');
  }
  const offscreen = canvas.transferControlToOffscreen();
  const worker = new Worker(
    opts.workerUrl ?? new URL('../worker/worker.js', import.meta.url),
    { type: 'module', name: 'doc-converter-renderer' },
  );
  const rpc: RpcHandle = createRpc(worker);
  await rpc.call('init', { canvas: offscreen }, [offscreen]);

  let ro: ResizeObserver | undefined;
  if (!opts.manualResize) {
    ro = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const dpr = window.devicePixelRatio || 1;
      rpc.notify('resize', {
        width: entry.contentRect.width,
        height: entry.contentRect.height,
        dpr,
      });
    });
    ro.observe(canvas);
  }

  return {
    render: (req) => rpc.call('render', req),
    exportPdf: (o) => rpc.call<Uint8Array>('exportPdf', o),
    exportPng: (o) => rpc.call<Uint8Array>('exportPng', o),
    hitTest: (x, y) => rpc.call<HitResult | null>('hitTest', { x, y }),
    onTick: (h) => rpc.on('tick', (m) => h(m as Extract<WorkerResponse, { type: 'tick' }>)) as () => void,
    dispose: () => { ro?.disconnect(); rpc.dispose(); worker.terminate(); },
  };
}

/** Fallback: рендер на main-thread, парсинг/PDF всё равно в воркере. */
export function assertFallbackAvailable(): void {
  if (typeof HTMLCanvasElement === 'undefined') {
    throw new Error('doc-converter requires a browser-like environment');
  }
}
