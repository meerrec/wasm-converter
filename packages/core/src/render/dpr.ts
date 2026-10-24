const DEFAULT_MAX_DPR = 3;
/** На Retina 4K canvas.width*height > 16M — OffscreenCanvas падает. */
const MAX_PIXELS = 16_000_000;

/**
 * Клампит DPR по размеру canvas. Вынесено отдельно, чтобы правило проверялось
 * без браузера.
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
