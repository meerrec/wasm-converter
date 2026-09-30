import { describe, expect, it, vi } from 'vitest';

import { exportPng } from '../src/render/export_png.js';

const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/** Минимальный двойник OffscreenCanvas: реального кодера в node нет. */
function fakeCanvas(bytes: Uint8Array): OffscreenCanvas {
  return {
    convertToBlob: vi.fn(async () => ({
      arrayBuffer: async () => bytes.buffer.slice(0) as ArrayBuffer,
    })),
  } as unknown as OffscreenCanvas;
}

describe('exportPng', () => {
  it('returns the bytes produced by convertToBlob', async () => {
    const src = new Uint8Array([...PNG_MAGIC, 1, 2, 3]);
    const out = await exportPng(fakeCanvas(src));
    expect(Array.from(out)).toEqual(Array.from(src));
  });

  it('requests image/png', async () => {
    const canvas = fakeCanvas(new Uint8Array(PNG_MAGIC));
    await exportPng(canvas);
    expect(canvas.convertToBlob).toHaveBeenCalledWith({ type: 'image/png' });
  });

  it('returns a standalone buffer that survives transfer', async () => {
    const out = await exportPng(fakeCanvas(new Uint8Array(PNG_MAGIC)));
    expect(out.byteOffset).toBe(0);
    expect(out.buffer.byteLength).toBe(PNG_MAGIC.length);
  });
});
