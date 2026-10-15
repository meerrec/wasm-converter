# doc-converter

Просмотр и конвертация OOXML (DOCX/XLSX) в браузере и Node.js: Rust/WASM,
рендер на `OffscreenCanvas` в Web Worker, экспорт в PDF/Markdown/HTML/текст.

## Что это

- **Просмотрщики XLSX и DOCX** в браузере: разбор, раскладка и отрисовка
  кадра целиком в Web Worker, основной поток получает готовый холст.
- **Экспорт**: PDF (собственный PDF-бэкенд, [ADR-0022](docs/adr/0022-own-pdf-backend.md)),
  PNG, Markdown, HTML, текст.
- **Node-CLI** `@doc-converter/cli`: `convert` и `batch` на тех же
  wasm-модулях, с пулом воркеров.
- Монорепозиторий: шесть Rust-крейтов (`core`, `render`, `xlsx`, `docx`,
  `pdf`, три wasm-модуля) и JS/TS-пакеты (`@doc-converter/core`,
  `@doc-converter/wasm`, `@doc-converter/wasm-docx`, `@doc-converter/wasm-pdf`,
  `@doc-converter/cli`).

## Статус

Спринты 1–11 закрыты (сводка — [ROADMAP.md](ROADMAP.md) §7): разбор и
раскладка обоих форматов, оба транспорта кадра first-class, просмотрщики,
экспорт PDF/Markdown/HTML/текста, CLI с батч-конвертацией, оракул против
LibreOffice, бенчмарки против JS-библиотек. Сейчас идёт **Спринт 12 — релиз
v1.0.0** ([docs/sprint-12/plan.md](docs/sprint-12/plan.md)): релизная
механика, публикация в npm и crates.io, документация. Что работает — в
разделе «Производительность», что осознанно не входит в v1 — в разделе
«Ограничения v1».

## Демо

Корень <https://meerrec.github.io/wasm-converter/> — страница-указатель, а не
приложение: там же лежит статика движка Euro-Office, которую открывает страница
демо `#/eurooffice`. Само демо живёт в подкаталоге
<https://meerrec.github.io/wasm-converter/viewer/> и это четыре страницы:
«Слово» на `#/docx`, «ДваИксель» на `#/xlsx`, «Конвертер» на `#/convert` и
«Euro-Office» на `#/eurooffice` (корень демо ведёт на `#/docx`). Публикуется
workflow `Pages` при пуше в `main`.

Страница «Конвертер» работает на x2t-wasm (конвертер ONLYOFFICE) и переводит
файлы внутри семейства — docx↔odt/rtf/txt, xlsx↔ods/csv; ~39 МБ артефактов
грузятся лениво при первой конвертации. Подробности —
[packages/browser-convert/README.md](packages/browser-convert/README.md).

Страница «Euro-Office» — self-host движка Euro-Office (форк ONLYOFFICE)
целиком в браузере: наша страница встраивает редактор iframe'ом и открывает
файлы через Embed API **только для просмотра**. Файл читает родитель и передаёт
его байтами, так что документ никуда не уезжает; взамен первый заход тянет
~35 МиБ JS/WASM движка. Код проекта — Apache-2.0, движок — AGPL-3.0-only; тексты
лицензий движка лежат в `/LICENSES/` опубликованного сайта.

GitHub Pages не отдаёт заголовки cross-origin isolation (COOP/COEP), поэтому их
ставит единственный service worker сайта — наш `sw.js` в корне; вендоренного
`coi-serviceworker.js` в демо больше нет. В корне он потому, что навигацию
вложенного кадра обслуживает воркер, контролирующий родителя: воркер в области
`/viewer/` навигацию `editor.html` не увидел бы, и кадр редактора остался бы без
COEP — а изолированная страница демо такой кадр не впускает. COEP он ставит всем
ответам (его требует и воркер-скрипт в своём собственном ответе — без этого у
движка не поднимается конвертер x2t), COOP — только документам; кэшей воркер не
держит (на `activate` сносит кэши прежних версий сайта). Регистрируют его
страница демо и корневая страница-указатель, лишь пока изоляции нет, и только
если браузер умеет встраивать кадр в изолированную страницу — сегодня это
Chromium (проверку спрашивают и страницы, и сам воркер: движок регистрирует его
мимо них); на первом заходе демо перезагружается один раз. Firefox и Safari
остаются без изоляции вовсе: вьюер едет на `bytes`-транспорте (ADR-0021), и
`#/eurooffice` там рабочий — неизолированный родитель кадр не проверяет. На
браузере с поддержкой, но без воркера, вьюер тоже уходит на `bytes`, а вот
`#/eurooffice`
не заработает — редактор не встраивается.

## Быстрый старт

### Установка

```bash
npm install @doc-converter/core @doc-converter/wasm \
  @doc-converter/wasm-docx @doc-converter/wasm-pdf
npm install -g @doc-converter/cli
```

`wasm-docx` и `wasm-pdf` — ленивые модули: воркер грузит их по URL при
открытии `.docx` и при первом экспорте в PDF соответственно.

### Просмотрщик XLSX

Воркер — отдельный бандл с единственной строкой:

```ts
// xlsx_worker.ts
import '@doc-converter/core/worker';
```

```ts
import { createXlsxViewer } from '@doc-converter/core';
import workerUrl from './xlsx_worker.ts?worker&url'; // Vite

const viewer = await createXlsxViewer(host, {
  workerUrl,
  pdfWasm: {
    module: new URL(
      '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
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
import { createDocxViewer } from '@doc-converter/core';
import workerUrl from './docx_worker.ts?worker&url'; // Vite

const viewer = await createDocxViewer(host, {
  workerUrl,
  docxWasm: {
    module: new URL(
      '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm.js',
      import.meta.url,
    ).href,
  },
  pdfWasm: {
    module: new URL(
      '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
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
(`node:worker_threads`).

## Браузеры

Chrome 90+, Firefox 105+, Safari 16.4+ — по доступности `OffscreenCanvas`
(проверяется экспортом `supportsOffscreen()`; просмотрщик требует его —
main-thread fallback в v1 не реализован). Работа без COOP/COEP — штатный
путь, см. ниже.

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
| `@doc-converter/core` worker.js / index.js | 7,17 / 3,34 КБ | 550 / 200 КБ |
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
- **PPTX и XLSB** не поддерживаются.
- **Strict OOXML** не поддерживается — только Transitional
  ([ADR-0017](docs/adr/0017-strict-vs-transitional.md)).
- **Макрофайлы `.docm`** не поддерживаются; зашифрованные DOCX и `.doc` —
  фатальная ошибка ([ADR-0016](docs/adr/0016-docx-error-policy.md)).

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
