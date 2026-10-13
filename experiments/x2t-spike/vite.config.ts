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
    // Формат воркера должен совпадать у dev и сборки: в dev Vite отдаёт воркеры без сборки
    // как ES-модули, и classic-воркер на странице падал бы на первом же ESM-импорте.
    // `es` в сборке даёт модульный воркер — тот же режим, что и в dev.
    format: 'es',
    // Динамический `import()` обвязки инлайнится: воркер остаётся одним файлом,
    // как и описано в src/worker.ts.
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
