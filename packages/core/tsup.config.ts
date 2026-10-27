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
    xlsb_worker: 'src/worker/xlsb_worker.ts',
    ods_worker: 'src/worker/ods_worker.ts',
    odp_worker: 'src/worker/odp_worker.ts',
    odt_worker: 'src/worker/odt_worker.ts',
  },
  format: ['esm', 'cjs'],
  dts: true,
  sourcemap: true,
  clean: true,
  target: 'es2022',
  splitting: false,
  treeshake: true,
});
