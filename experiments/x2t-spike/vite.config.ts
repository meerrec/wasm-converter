import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

/**
 * Просмотр документа идёт нашим вьюером из `@meerrec/core`, а харнесс лежит вне
 * pnpm-workspace: по имени пакет не резолвится (та же причина, что у `packages/x2t` в
 * src/worker.ts). Страница поэтому импортирует ядро относительным путём, а здесь остаётся
 * то, что разрешить иначе нечем: сборка `@meerrec/wasm` — её тянет воркер ядра
 * (`dist/worker.js`), и это единственный голый импорт в нём.
 */
const wasmEntry = fileURLToPath(
  new URL('../../packages/wasm/pkg/doc_converter_wasm.js', import.meta.url),
);

/**
 * Cross-origin isolation — условие пути через SharedArrayBuffer: с этими заголовками воркер
 * ядра возит кадры через SAB, без них — буферами (fallback в ядре есть, но заголовки
 * честнее: так харнесс проверяет тот же путь, что и пример-просмотрщик).
 */
const isolationHeaders = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

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
  resolve: {
    alias: { '@meerrec/wasm': wasmEntry },
  },
  server: { port: 5191, strictPort: true, headers: isolationHeaders },
  preview: { headers: isolationHeaders },
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
