import { defineConfig } from 'tsup';

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    worker: 'src/worker/worker.ts',
    // Воркер PPTX отдельный (направление «воркер на формат», ADR-0030 §е):
    // обслуживает одну презентацию и не трогает состояние воркера XLSX/DOCX.
    pptx_worker: 'src/worker/pptx_worker.ts',
  },
  format: ['esm', 'cjs'],
  dts: true,
  sourcemap: true,
  clean: true,
  target: 'es2022',
  splitting: false,
  treeshake: true,
});
