# Рецепты: интеграция

Сборка под Vite, запуск в Node, проверка окружения и типовые ошибки
инициализации.

---

### <a id="in-01"></a>Рецепт 1. Vite: воркер через `?worker&url`

**Задача.** Собрать воркер просмотрщика отдельным чанком.

```ts
// xlsx_worker.ts — точка входа воркера
import '@meerrec/core/xlsx-worker';

// main.ts
import { createXlsxViewer } from '@meerrec/core';
import workerUrl from './xlsx_worker.ts?worker&url';

const viewer = await createXlsxViewer(host, { workerUrl });
```

```ts
// vite.config.ts
export default defineConfig({
  plugins: [react()],
  worker: { format: 'es' },
});
```

**Под капотом.** Суффикс `?worker&url` — единственный способ получить
отдельный чанк: голый `new URL('./xlsx_worker.ts', import.meta.url)` Vite
считает обычным ассетом и инлайнит исходник в data-URL, где импорт
`@meerrec/core/xlsx-worker` уже не разрешается, и воркер молча не стартует.

---

### <a id="in-02"></a>Рецепт 2. Vite: COOP/COEP на dev-сервере

**Задача.** Включить SAB-путь локально и в preview.

```ts
// vite.config.ts
export default defineConfig({
  server: {
    // SharedArrayBuffer доступен только в cross-origin isolated окружении.
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
  preview: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
});
```

**Под капотом.** С заголовками воркер выберет `sab`; без них тот же код
работает на `bytes` — dev-сервер с заголовками нужен, чтобы тестировать
ровно тот путь, которым страница поедет в проде.

---

### <a id="in-03"></a>Рецепт 3. WorkerPool в Node

**Задача.** Конвертировать файлы из своей Node-программы параллельно.

```ts
import { WorkerPool, type ConvertJob, type ConvertResult } from '@meerrec/cli';

const pool = new WorkerPool(4); // ровно столько воркеров, очередь общая
const job: ConvertJob = {
  input: 'in/report.docx',
  output: 'out/report.pdf',
  format: 'pdf',
  pdf: {}, // PdfOptions (@meerrec/core): pageSize/orientation/scale/margins/sheetIndex — необязательны
};

const results = await Promise.all([
  pool.run(job),
  pool.run({ ...job, input: 'in/other.docx', output: 'out/other.pdf' }),
]);
for (const r of results) console.log(r.output, r.bytes, r.ms);
await pool.close(); // дожидается всех задач и гасит воркеры
```

**Под капотом.** Воркер живёт долго и держит wasm-модуль в памяти между
файлами, поэтому за файл не платят загрузкой; упавший воркер не роняет
прогон — задача помечается неудачной, а место занимает новый.

---

### <a id="in-04"></a>Рецепт 4. convertFile в Node без пула

**Задача.** Одна конвертация из Node — без воркеров и очередей.

```ts
import { convertFile, CliError } from '@meerrec/cli';

try {
  const { bytes, ms } = await convertFile({
    input: 'report.docx',
    output: 'report.md',
    format: 'md',
    pdf: {},
  });
  console.log(`${bytes} байт за ${ms} мс`);
} catch (error) {
  // CliError — расширение Error; exitCode 2 означает ошибку вызова.
  console.error(error instanceof CliError ? error.message : error);
}
```

**Под капотом.** `convertFile` исполняет ровно то, что делает команда
`convert` без пула: читает файл, зовёт нужную wasm-функцию
(`export_docx_pdf`/`export_pdf`/`export_pdf_book`/`docx_export_*`) и пишет
выход; wasm в Node грузится через `module_or_path` байтами, потому что
`fetch` не умеет `file:`.

---

### <a id="in-05"></a>Рецепт 5. Явный `sab` без заголовков: ловим ошибку и объясняем пользователю

**Задача.** Превратить ошибку инициализации в понятное сообщение и
предложение исправиться.

```ts
import { createXlsxViewer } from '@meerrec/core';

async function mount(): Promise<void> {
  try {
    const viewer = await createXlsxViewer(host, {
      workerUrl,
      transport: 'sab', // осознанный выбор: страница обещала COOP/COEP
    });
    await viewer.open(bytes);
  } catch (error) {
    // Явный 'sab' без заголовков отклоняет промис на init — это ошибка
    // конфигурации, а не поломка: предлагаем отдать заголовки или снять флаг.
    status.textContent =
      'Не удалось включить SharedArrayBuffer: добавьте заголовки ' +
      'COOP/COEP или используйте transport: "auto" (кадры поедут буферами).';
  }
}
```

**Под капотом.** Контракт «ошибка, а не тихая подмена» существует именно
ради этого сценария: неверная конфигурация видна сразу, на инициализации,
и диагностируется по тексту, а не по упавшему FPS.

---

### <a id="in-06"></a>Рецепт 6. Режим рендера и проверка окружения

**Задача.** Узнать, будет ли доступен worker-путь, и предупредить о переходе
на main-режим.

```ts
import { createXlsxViewer } from '@meerrec/core';

async function mount(): Promise<void> {
  try {
    // Явный 'worker' без OffscreenCanvas — ошибка инициализации, а не тихая
    // подмена пути: так окружение проверяется честно, без отдельной функции.
    const viewer = await createXlsxViewer(host, { renderMode: 'worker' });
    await viewer.open(bytes);
  } catch (error) {
    // Chrome 90+, Firefox 105+, Safari 16.4+ (README «Браузеры»).
    showBanner('OffscreenCanvas нет: используйте renderMode: "auto" — рендер на main-потоке');
    // ...и поднять вьюер с renderMode: 'auto'
  }
}
```

**Под капотом.** Отдельных `supportsOffscreen`/`assertFallbackAvailable`
больше нет — решение о месте рендера принимает `resolveRenderMode`
(пакетный модуль `render/offscreen`). С `renderMode: 'auto'` (по умолчанию)
вьюер сам переходит в main-режим: парсинг и сборка DisplayList остаются в
воркере, а рисует main тем же Rust-painter'ом, поэтому на странице может
жить только один main-вьюер. Явный `renderMode: 'worker'` без возможности
даёт внятную ошибку инициализации — годится для предварительной проверки;
подробнее — [transports.md](transports.md#tr-08).

---

### <a id="in-07"></a>Рецепт 7. Отдельный воркер на каждый вьюер

**Задача.** Две страницы демо (XLSX и DOCX) не мешают друг другу.

```ts
// У каждого вьюера свой воркер-файл и свой чанк:
//   xlsx_worker.ts  → import '@meerrec/core/xlsx-worker';
//   docx_worker.ts  → import '@meerrec/core/docx-worker';
import docxWorkerUrl from '../workers/docx_worker.ts?worker&url';
import xlsxWorkerUrl from '../workers/xlsx_worker.ts?worker&url';

const docx = await createDocxViewer(docxHost, { workerUrl: docxWorkerUrl, docxWasm });
const xlsx = await createXlsxViewer(xlsxHost, { workerUrl: xlsxWorkerUrl });

// Закрытие страницы DOCX гасит только её воркер (destroy терминует его),
// книга XLSX остаётся жить в своём.
docx.destroy();
```

**Под капотом.** Файлы разделены намеренно (паттерн демо
`examples/viewer/src/workers/`): у вьюеров независимые воркеры и ленивые
модули, поэтому закрытие одной страницы не сносит состояние другой.

---

### <a id="in-08"></a>Рецепт 8. Кэш wasm-модулей blob-URL-ами

**Задача.** Прогреть кэш модулей, чтобы открытие документа не ждало сеть.

```ts
// Упрощённый вариант паттерна examples/viewer/src/lib/viewer_urls.ts:
// модуль остаётся ленивым, но его байты читаются заранее в кэш, а воркеру
// отдаются blob-URL'ы.

const JS_MIME = 'text/javascript';
const WASM_MIME = 'application/wasm';

async function cachedUrls(cache: Cache, jsUrl: string, wasmUrl: string) {
  const cached = await cache.match(jsUrl);
  if (cached === undefined) return { module: jsUrl, binary: wasmUrl };
  const module = URL.createObjectURL(await cached.blob());
  const wasm = await cache.match(wasmUrl);
  const binary = wasm === undefined
    ? wasmUrl
    : URL.createObjectURL(await wasm.blob());
  return { module, binary };
}

const { module, binary } = await cachedUrls(cache, pdfJsUrl, pdfWasmUrl);
const viewer = await createXlsxViewer(host, { workerUrl, pdfWasm: { module, binary } });
// После первого успешного экспорта blob-URL'ы можно отозвать:
// URL.revokeObjectURL(module); URL.revokeObjectURL(binary);
```

**Под капотом.** MIME-типы обязательны: по `text/javascript` браузер
разбирает blob-URL как модуль, по `application/wasm` работает стриминговая
инициализация; отзывать адреса можно только после того, как воркер модуль
забрал, — `import` отозванного blob-URL падает.

---

Ссылки: демо-паттерны — `examples/viewer/src/{lib,workers}/`,
транспорты — [transports.md](transports.md), COOP/COEP — `docs/workers.md`.
