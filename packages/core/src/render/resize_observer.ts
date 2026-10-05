export interface RpcLike {
  call(method: string, params: unknown): unknown;
  attachSab?(sab: SharedArrayBuffer, capacity: number): void;
}

export interface AttachResizeOpts {
  debounceMs?: number;
  maxDpr?: number;
}

const DEFAULT_DEBOUNCE = 100;
const DEFAULT_MAX_DPR = 3;
/** На Retina 4K canvas.width*height > 16M — OffscreenCanvas падает. */
const MAX_PIXELS = 16_000_000;

/**
 * Клампит DPR по размеру canvas. Вынесено отдельно от `attachResize`,
 * чтобы правило проверялось без браузера.
 */
export function computeDpr(
  cssW: number,
  cssH: number,
  devicePixelRatio: number,
  maxDpr: number = DEFAULT_MAX_DPR,
): number {
  let dpr = Math.min(devicePixelRatio || 1, maxDpr);
  if (cssW * cssH * dpr * dpr > MAX_PIXELS) {
    dpr = Math.max(1, Math.sqrt(MAX_PIXELS / (cssW * cssH)));
  }
  return dpr;
}

export function attachResize(
  canvas: HTMLCanvasElement,
  rpc: RpcLike,
  opts: AttachResizeOpts = {},
): () => void {
  const debounceMs = opts.debounceMs ?? DEFAULT_DEBOUNCE;
  const maxDpr = opts.maxDpr ?? DEFAULT_MAX_DPR;

  let timer: number | null = null;

  const flush = () => {
    timer = null;
    const w = canvas.clientWidth || canvas.width;
    const h = canvas.clientHeight || canvas.height;
    const dpr = computeDpr(w, h, window.devicePixelRatio, maxDpr);
    rpc.call('resize', { cssW: w, cssH: h, dpr });
    rpc.call('render', {});
  };

  const onResize = () => {
    if (timer != null) clearTimeout(timer);
    timer = window.setTimeout(flush, debounceMs);
  };

  const ro = new ResizeObserver(onResize);
  ro.observe(canvas);
  window.addEventListener('resize', onResize);
  flush();

  return () => {
    ro.disconnect();
    window.removeEventListener('resize', onResize);
    if (timer != null) clearTimeout(timer);
  };
}
