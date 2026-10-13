import { defineConfig } from 'vite';

/**
 * Сборка оболочки примера одним файлом в `dist/main.js` — его и ждёт
 * `viewer.html` (страница уезжает в `site/` через `scripts/assemble.mjs`).
 *
 * `@doc-converter/eurooffice` НЕ выносится в externals: `site/` — статика без
 * пакетного менеджера, голый спецификатор в ней не разрешится, поэтому клиент
 * Embed API зашивается в бандл.
 */
export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // Браузеры, под которые собран редактор Euro-Office, — современные; опускать
    // синтаксис до ES5 незачем.
    target: 'es2022',
    sourcemap: true,
    lib: {
      entry: 'src/viewer/main.ts',
      formats: ['es'],
      fileName: 'main',
    },
  },
});
