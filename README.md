# doc-converter

Просмотр и конвертация OOXML (DOCX/XLSX) в браузере и Node.js: Rust/WASM,
рендер на `OffscreenCanvas` в Web Worker, экспорт в PDF/Markdown/HTML/текст.

## Что это

- **Просмотрщики XLSX и DOCX** в браузере: разбор, раскладка и отрисовка
  кадра целиком в Web Worker, основной поток получает готовый холст.
- **Экспорт**: PDF (собственный PDF-бэкенд, [ADR-0022](docs/adr/0022-own-pdf-backend.md)),
  PNG, Markdown, HTML, текст.
- **CLI** `@meerrec/cli`: `convert` и `batch` на тех же wasm-модулях, с
  пулом воркеров; работает под Node, Bun и Deno (см.
  [`packages/cli/README.md`](packages/cli/README.md)).
- Монорепозиторий: шесть Rust-крейтов (`core`, `render`, `xlsx`, `docx`,
  `pdf`, три wasm-модуля) и JS/TS-пакеты (`@meerrec/core`,
  `@meerrec/wasm`, `@meerrec/wasm-docx`, `@meerrec/wasm-pdf`,
  `@meerrec/cli`).

## Статус

Спринты 1–11 закрыты (сводка — [ROADMAP.md](ROADMAP.md) §7): разбор и
раскладка обоих форматов, оба транспорта кадра first-class, просмотрщики,
экспорт PDF/Markdown/HTML/текста, CLI с батч-конвертацией, оракул против
LibreOffice, бенчмарки против JS-библиотек; спайк LibreOffice-WASM закрыт
решением no-go. Фазы 1–4 архитектурного аудита реализованы: публикация
десяти пакетов `@meerrec/*@1.2.0` в npmjs.org — основной реестр, установка
без токена ([ADR-0027](docs/adr/0027-npmjs-primary-registry.md); первая
публикация запущена и ждёт валидного `NPM_TOKEN`; GitHub
Packages — зеркало, crates.io — non-goal,
[ADR-0026](docs/adr/0026-rust-crates-distribution.md)), wire-типы Rust → TS
через ts-rs ([ADR-0028](docs/adr/0028-typed-wire-protocol.md)) и рендер на
main-потоке без `OffscreenCanvas` ([ADR-0029](docs/adr/0029-main-thread-render-backend.md)).
Что работает — в разделе «Производительность», что осознанно не входит в v1 — в
разделе «Ограничения v1».

## Демо

Демо просмотрщика живёт на
<https://meerrec.github.io/wasm-converter/viewer/> и это две страницы:
«Слово» на `#/docx` и «ДваИксель» на `#/xlsx` (корень демо ведёт на `#/docx`).
Публикуется workflow `Pages` при пуше в `main`.

Страницы «Конвертер» (x2t-wasm, конвертер ONLYOFFICE) и «Euro-Office»
(self-host движка Euro-Office, форк ONLYOFFICE, AGPL-3.0-only) вынесены вместе
со всем, что грузит или собирает движок, в отдельный репозиторий
[github.com/meerrec/eurooffice-demo](https://github.com/meerrec/eurooffice-demo)
([ADR-0038](docs/adr/0038-agpl-separation.md)): монорепозиторий — чистый
Apache-2.0, сайт движка собирается и публикуется там.

GitHub Pages не отдаёт заголовки cross-origin isolation (COOP/COEP), поэтому их
ставит единственный service worker сайта — наш `sw.js` в корне; вендоренного
`coi-serviceworker.js` в демо больше нет. COEP он ставит всем ответам (его
требует и воркер-скрипт в своём собственном ответе), COOP — только документам;
кэшей воркер не держит (на `activate` сносит кэши прежних версий сайта).
Регистрирует его страница демо, лишь пока изоляции нет, и только если браузер
умеет встраивать кадр в изолированную страницу — сегодня это Chromium; на
первом заходе демо перезагружается один раз. Firefox и Safari остаются без
изоляции вовсе: вьюер едет на `bytes`-транспорте (ADR-0021) — штатный путь.

## Быстрый старт

### Установка

Десять пакетов `@meerrec/*` версии `1.2.0`: `core`, `wasm`, `wasm-docx`,
`wasm-pdf`, `wasm-pptx`, `wasm-pptx-pdf`, `cli`, `react`, `vue`, `svelte`.
Основной реестр — `registry.npmjs.org` ([ADR-0027](docs/adr/0027-npmjs-primary-registry.md));
установка без токена станет доступна после первой публикации (ждёт валидного
`NPM_TOKEN`); **GitHub Packages** — действующее зеркало (нужна
авторизация и `~/.npmrc`):

```bash
npm install @meerrec/core @meerrec/wasm \
  @meerrec/wasm-docx @meerrec/wasm-pdf \
  @meerrec/wasm-pptx @meerrec/wasm-pptx-pdf
npm install @meerrec/react   # обёртки: react | vue | svelte
npm install -g @meerrec/cli
```

Тарболы — в ассетах
релиза [`v1.2.0`](https://github.com/meerrec/wasm-converter/releases/tag/v1.2.0)
(`meerrec-*.tgz`, ставится как `npm install ./meerrec-core-1.2.0.tgz`).

`wasm-docx`, `wasm-pdf`, `wasm-pptx` и `wasm-pptx-pdf` — ленивые модули:
воркер грузит их по URL при открытии документа и при первом экспорте в PDF.

### Просмотрщик XLSX

Воркер — отдельный бандл с единственной строкой:

```ts
// xlsx_worker.ts
import '@meerrec/core/xlsx-worker';
```

```ts
import { createXlsxViewer } from '@meerrec/core';
import workerUrl from './xlsx_worker.ts?worker&url'; // Vite

const viewer = await createXlsxViewer(host, {
  workerUrl,
  pdfWasm: {
    module: new URL(
      '@meerrec/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
      import.meta.url,
    ).href,
  },
});

const bytes = await file.arrayBuffer();
await viewer.open(bytes);        // листы книги
viewer.showSheet(1);
viewer.setZoom(1.25);
const pdf = await viewer.exportPdf();
const png = await viewer.exportPng();
viewer.destroy();
```

`createXlsxViewer(host, options)` сам собирает разметку: прокручиваемую
область и холст внутри переданного элемента. Байты книги передаются воркеру
переводом владения. Полезные методы: `hitTest`, `hyperlinkAt`, `setView`
(сетка, заголовки, тёмная тема), `onTick` (метрики кадров).

### Просмотрщик DOCX

```ts
import { createDocxViewer } from '@meerrec/core';
import workerUrl from './docx_worker.ts?worker&url'; // Vite

const viewer = await createDocxViewer(host, {
  workerUrl,
  docxWasm: {
    module: new URL(
      '@meerrec/wasm-docx/pkg/doc_converter_docx_wasm.js',
      import.meta.url,
    ).href,
  },
  pdfWasm: {
    module: new URL(
      '@meerrec/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
      import.meta.url,
    ).href,
  },
});

await viewer.open(bytes);        // страницы документа
viewer.showPage(3);
const md   = await viewer.exportMarkdown();
const html = await viewer.exportHtml();
const text = await viewer.exportText();
const pdf  = await viewer.exportPdf();
viewer.destroy();
```

Без `docxWasm` открытие `.docx` вернёт внятную ошибку вместо страниц; без
`pdfWasm` — ошибку вернёт экспорт в PDF.

### CLI

```bash
doc-converter convert report.docx -o report.pdf
doc-converter convert report.docx -o report.md
doc-converter batch ./in/*.xlsx --out ./pdf --workers 4
doc-converter batch ./in --out ./out --format pdf --workers 8 --scale 0.9
```

`convert` принимает один файл (DOCX → pdf/md/html/txt, XLSX → pdf), `batch` —
файлы, каталоги и glob-ы с прогрессом, рекурсивным обходом и пулом воркеров
(`node:worker_threads` в Node/Bun, веб-воркеры в Deno).

## Браузеры

Chrome 90+, Firefox 105+, Safari 16.4+ — если браузер умеет `OffscreenCanvas`
(проверяется экспортом `supportsOffscreen()`), это путь по умолчанию: canvas
уезжает в worker, и кадр рисуется там. Без `transferControlToOffscreen` вьюер
не падает, а автоматически переходит в main-режим: парсинг и сборка
DisplayList остаются в worker, а рисует main тем же Rust-painter'ом. Режим
задаётся опцией `renderMode: 'auto' | 'worker' | 'main'` (по умолчанию
`auto`); явный `'worker'` без возможности — ошибка инициализации. В
main-режиме — один вьюер на страницу. Работа без COOP/COEP — штатный путь,
см. ниже.

## Встраивание: COOP/COEP и SharedArrayBuffer

Кадр собирается и рисуется в Web Worker'е (`OffscreenCanvas`), и у движка два
равноправных транспорта для него:

| Транспорт | Когда | Что это |
|---|---|---|
| `sab` | страница отдана с COOP/COEP | ring из двух слотов в `SharedArrayBuffer` |
| `bytes` | всегда, в том числе без заголовков | два `ArrayBuffer`, которые воркер заполняет по очереди |

Для `SharedArrayBuffer` нужны заголовки cross-origin isolation — их ставит
тот, кто отдаёт **страницу** (библиотека их себе поставить не может):

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

При embed это часто не под контролем потребителя: iframe на чужом сайте,
расширение, Electron/webview, GitHub Pages. **Просмотрщик от этого не
перестаёт работать**: без `crossOriginIsolated` воркер сам выбирает транспорт
`bytes`. API, кадры и экспорт (PNG, PDF, Markdown/HTML/текст) одинаковы на
обоих путях — отличается транспорт, а не поведение; какой путь выбран, видно
в `viewer.transport`. Задать транспорт явно можно опцией
`transport: 'sab' | 'bytes' | 'auto'` (по умолчанию `auto`); явный `sab` без
заголовков — ошибка инициализации, а не тихая подмена.

Что даёт SAB: кадр кладётся в разделяемый буфер, ring держит глубину два
(пока painter рисует один кадр, сборщик готовит следующий) и сам отбрасывает
устаревшие кадры при отставании. Что теряется без него: разделяемой памяти
нет, кадр едет обычным `ArrayBuffer` — двойная буферизация при этом та же, а
кадр собирает ровно та же функция Rust и рисует тот же painter. Разница — в
скорости. По замерам Спринта 11 ([ROADMAP.md](ROADMAP.md) §9): RPC
round-trip < 1 мс на SAB против < 3 мс на буферах (RPC-слой с тех пор удалён
как неиспользуемый); сборка кадра без деградации (transferables).

Изоляцию дают двумя способами — на сам транспорт это влияет только тем,
доступен ли он:

- **GitHub Pages + service worker** (текущее демо): заголовки ставит `sw.js`,
  изоляцию получает Chromium, Firefox и Safari остаются на `bytes`. Патч
  зависим от вложенных кадров: воркер должен контролировать родителя.
- **Native COOP/COEP** (рекомендуется для продакшена): заголовки отдаёт сам
  хостинг, service worker для изоляции не нужен, изоляция есть во всех
  браузерах с поддержкой COOP/COEP. Пример конфига для Cloudflare Pages —
  [`demo/cloudflare/_headers`](demo/cloudflare/_headers), порядок подключения
  и проверки — [`demo/cloudflare/README.md`](demo/cloudflare/README.md).

## Производительность

Замеры Спринта 12, после миграции PDF-бэкенда
([docs/sprint-12/budgets.md](docs/sprint-12/budgets.md),
[docs/sprint-12/oracle.md](docs/sprint-12/oracle.md)); машина — Apple Silicon,
релизная сборка.

| Метрика | Факт | Бюджет/цель |
|---|---|---|
| Разбор DOCX 50 МБ (`openDocx`, нативно) | 356 мс | < 1,5 с |
| Пагинация DOCX, 150 страниц | 32,47 мс | < 500 мс на 100 стр. |
| Экспорт PDF, 500 страниц XLSX | 159,4 мс | < 3 с |
| Пик памяти PDF-экспорта `scale-500-pages.xlsx` | 28,12 МиБ | ≤ 40 МиБ |
| Пик памяти разбора `memory_50mib.docx` (57 МБ, 75 картинок) | 155,50 МиБ | ≤ 200 МиБ |
| `batch` по `test-fixtures/xlsx`, 116 файлов, 4 воркера | 0,20 с | 100 файлов < 30 с |
| DOCX → HTML против mammoth, медиана | ×5,3 | — |

Оракул против LibreOffice (постраничный SSIM):

| Метрика | Факт |
|---|---|
| DOCX: число страниц ±1 | 99 из 99 фикстур |
| DOCX: SSIM первой страницы, mean | 0,9925 |
| XLSX: 12 фикстур умолчаний, mean | 0,9824 |
| XLSX: 104 однолистовые книги, mean | 0,9844 |

Размер ассетов (gzip, гейт `pnpm size-limit`):

| Ассет | Факт | Бюджет |
|---|---|---|
| `@meerrec/core` xlsx-worker.js / docx-worker.js | 9,27 / 9,05 КБ | 550 / 550 КБ |
| `@meerrec/core` pptx-worker.js / index.js | 8,36 / 11,22 КБ | 550 / 200 КБ |
| `wasm (viewer)` — XLSX-просмотрщик | 464,2 КБ | 480 КБ |
| `wasm (docx)` — ленивый модуль DOCX | 384,4 КБ | 400 КБ |
| `wasm (pdf)` — ленивый модуль PDF | 1,01 МБ | 1,05 МБ |

Модуль просмотра DOCX и модуль PDF грузятся по требованию; PDF после ухода с
printpdf на собственный бэкенд похудел на 31 % gzip. Бюджет основного
модуля поднят в Спринте 12 из-за вшитых поднаборов Carlito: превью рисует
той же гарнитурой, что и PDF. Источники остальных чисел — [ROADMAP.md](ROADMAP.md)
§9 и [docs/bench-js.md](docs/bench-js.md).

## Ограничения v1

- **Редактирование — нет.** Только чтение и конвертация
  ([ADR-0018](docs/adr/0018-editing-boundaries.md),
  [ADR-0020](docs/adr/0020-parser-writer-boundary.md)).
- **Сноски и комментарии** парсятся в модели, но в раскладку не попадают.
- **Обтекание текстом вокруг плавающих рисунков — нет**: рисунок
  вынимается из потока.
- **VML-фолбэки** (`mc:Fallback`) игнорируются — рисуется только `mc:Choice`
  ([ADR-0014](docs/adr/0014-alternate-content.md)).
- **PPTX поддерживается**: просмотр слайдов, экспорт PDF (слайд = страница,
  размер из `p:sldSz`), PNG и текстового конспекта (Markdown/HTML/text) —
  [ADR-0033](docs/adr/0033-pptx-plugin.md).
- **Strict OOXML (DOCX и XLSX) — поддерживается** наравне с Transitional
  ([ADR-0034](docs/adr/0034-strict-ooxml.md)).
- **Макрофайлы `.docm` поддерживаются** как обычный DOCX, макросы не
  читаются ([ADR-0039](docs/adr/0039-cancel-v1-non-goals.md)).
- **ODF (`.odt`/`.ods`/`.odp`)** — в плане
  ([ADR-0039](docs/adr/0039-cancel-v1-non-goals.md)).
- **XLSB, зашифрованные документы и бинарный `.doc`** — статус non-goal
  снят ([ADR-0039](docs/adr/0039-cancel-v1-non-goals.md)), реализаций нет;
  открытие зашифрованных DOCX и `.doc` — фатальная ошибка
  ([ADR-0016](docs/adr/0016-docx-error-policy.md)).

## Лицензия

Apache-2.0 — файл [LICENSE](LICENSE) в корне репозитория.

## Ссылки

- [docs/adr/](docs/adr/) — принятые архитектурные решения.
- [ROADMAP.md](ROADMAP.md) — план, состояние по спринтам, бюджеты (§9),
  DoD v1.0 (§12).
- [docs/cookbook/](docs/cookbook/) — рецепты: просмотр, экспорт, CLI,
  embed — готовится к релизу (Спринт 12).
- [docs/migration-from-mammoth.md](docs/migration-from-mammoth.md) и
  [docs/migration-from-exceljs.md](docs/migration-from-exceljs.md) —
  миграционные гайды, готовятся к релизу (Спринт 12).
- [docs/bench-js.md](docs/bench-js.md) — сравнение с mammoth, exceljs,
  SheetJS.
- [examples/viewer](examples/viewer) — демо-приложение и e2e-тесты.
