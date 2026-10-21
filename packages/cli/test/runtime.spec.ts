import { availableParallelism } from 'node:os';

import { describe, expect, it } from 'vitest';

import { defaultWorkers } from '../src/args.js';
import { detectRuntime, loadRuntime } from '../src/runtime.js';

/** Граница рантайма: сам тест идёт под Node, поэтому активна Node-ветка. */
describe('runtime', () => {
  it('определяет Node и берёт число ядер у os', async () => {
    expect(detectRuntime()).toBe('node');
    const runtime = await loadRuntime();
    expect(runtime.availableParallelism()).toBe(availableParallelism());
  });

  it('defaultWorkers уважает переданный рантайм и потолок в восемь', () => {
    expect(defaultWorkers({ availableParallelism: () => 2 })).toBe(2);
    expect(defaultWorkers({ availableParallelism: () => 64 })).toBe(8);
  });

  it('грузит байты .wasm рядом с клеем', async () => {
    const runtime = await loadRuntime();
    const bytes = await runtime.loadWasmBytes('@meerrec/wasm-pdf/pkg/doc_converter_pdf_wasm.js');
    expect(Array.from(bytes.subarray(0, 4))).toEqual([0x00, 0x61, 0x73, 0x6d]);
  });
});
