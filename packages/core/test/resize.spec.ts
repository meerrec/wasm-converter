import { describe, expect, it } from 'vitest';

import { computeDpr } from '../src/render/resize_observer.js';

/** На Retina 4K canvas.width*height > 16M — OffscreenCanvas падает. */
const MAX_PIXELS = 16_000_000;

describe('computeDpr', () => {
  it('clamps devicePixelRatio to maxDpr', () => {
    expect(computeDpr(800, 600, 3, 3)).toBe(3);
    expect(computeDpr(800, 600, 4, 3)).toBe(3);
    expect(computeDpr(800, 600, 1.5, 3)).toBe(1.5);
  });

  it('treats missing devicePixelRatio as 1', () => {
    expect(computeDpr(800, 600, 0, 3)).toBe(1);
  });

  it('keeps width * height * dpr^2 within MAX_PIXELS', () => {
    const w = 3840;
    const h = 2160;
    const dpr = computeDpr(w, h, 3, 3);
    expect(w * h * dpr * dpr).toBeLessThanOrEqual(MAX_PIXELS);
  });

  it('never drops below 1 even for huge canvases', () => {
    expect(computeDpr(20_000, 20_000, 3, 3)).toBe(1);
  });

  it('leaves small canvases alone', () => {
    expect(computeDpr(320, 240, 2, 3)).toBe(2);
  });
});
