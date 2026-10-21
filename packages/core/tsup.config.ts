import { defineConfig } from 'tsup';

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    // У каждого формата свой воркер (направление «воркер на формат»,
    // ADR-0030 §е): общий диспетчер по `DocFormat` удалён, чужой формат
    // воркер получает ошибкой `open`, а не переключением состояния.
    xlsx_worker: 'src/worker/xlsx_worker.ts',
    docx_worker: 'src/worker/docx_worker.ts',
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
