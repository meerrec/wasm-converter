import { defineConfig } from 'vite';

/**
 * publicDir — каталог артефактов внутри пакета, целиком: страница отдаёт `/x2t.js` и
 * `/x2t.wasm` из единственной копии (`packages/x2t/vendor/`, Git LFS), а не из копии,
 * положенной в харнесс. Vite резолвит относительный путь от корня проекта, раздаёт файлы
 * из корня публичного каталога (то есть без префикса `/x2t`) и на сборке копирует каталог
 * в `dist/` как есть. Цена — 46 МБ в `dist/`, зато артефакты в репозитории не дублируются.
 */
export default defineConfig({
  root: import.meta.dirname,
  publicDir: '../../packages/x2t/vendor',
  server: { port: 5191, strictPort: true },
  build: { target: 'es2022' },
  worker: {
    // Воркер — classic, то есть в сборке iife, а он не умеет кодовые чанки: динамический
    // `import()` обвязки пакета иначе просит отдельный чанк, и Rollup отказывает сборке.
    // С инлайном импорт остаётся в бандле воркера — ровно как и задумано в src/worker.ts.
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
