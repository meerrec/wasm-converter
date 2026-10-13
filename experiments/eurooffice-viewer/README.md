# Спайк: просмотрщик на Euro-Office (self-host из npm-частей)

Self-host статического сайта Euro-Office — это редакторы ONLYOFFICE (форк Euro-Office)
целиком в браузере, с конвертером x2t в WebAssembly, — и наша страница `viewer.html`
поверх него: она встраивает редактор iframe'ом (`/editor.html?embed=1`) и открывает
файлы через Embed API (`postMessage`) в режиме только просмотра. Файл читает родитель
и передаёт байтами (`document:open-buffer`) — ни CORS, ни сети для этого не нужно.

Сайт не собирается из исходников: он скачивается шестью готовыми npm-частями
`@kreijstal/eurooffice-*` (версия пинится одной константой) и раскладывается overlay'ем
в один каталог `site/`. Единственная dev-зависимость спайка — `typescript` (сборка
нашей TS-оболочки просмотрщика в `dist/`); каталог лежит вне pnpm-workspace, как
`experiments/x2t-spike`, со своим `pnpm-lock.yaml`.

## Состав сайта

| Часть | Тарбол | Размер | Файлов |
|---|---|---|---|
| `@kreijstal/eurooffice` (главный) | 19,5 МиБ | 92,4 МиБ | 339 |
| `@kreijstal/eurooffice-sdkjs-common` | 44,7 МиБ | 123,2 МиБ | 422 |
| `@kreijstal/eurooffice-web-apps-1` | 25,1 МиБ | 86,1 МиБ | 1034 |
| `@kreijstal/eurooffice-web-apps-2` | 16,7 МиБ | 84,4 МиБ | 1025 |
| `@kreijstal/eurooffice-fonts-1` | — (не в npm) | — | — |
| `@kreijstal/eurooffice-fonts-2` | — (не в npm) | — | — |

**Порядок overlay** (в `scripts/assemble.mjs` — массив `PARTS`): подчинённые части
первыми, главный пакет последним, файлы поверх:

1. `eurooffice-fonts-1`, `eurooffice-fonts-2` — **в npm не опубликованы** (HTTP 404
   на реестре, проверено 14.10.2026), `assemble` говорит об этом и продолжает;
   их роль (движковый каталог `fonts/`) выполняет шаг зеркалирования — см.
   «Каталог шрифтов» ниже;
2. `eurooffice-sdkjs-common` → `sdkjs/common/**` (включая `sdkjs/common/wasm/x2t/`);
3. `eurooffice-web-apps-1` → `web-apps/apps/{common,api,pdfeditor,presentationeditor,visioeditor}`, часть `sdkjs/common/Images`;
4. `eurooffice-web-apps-2` → `web-apps/apps/{common,documenteditor,spreadsheeteditor}`;
5. `eurooffice` → HTML-страницы, `assets/`, `img/`, `ran-fonts/`, `sdkjs/{word,cell,slide,vendor,visio}`, `sw.js` и прочая обвязка сайта.

Пересечение реально проверено: у `web-apps-1` и `web-apps-2` совпадают 190 файлов, и все
они побайтово одинаковы (сплит разрезал дерево, но не менял содержимое); с главным
пакетом пересекаются только `README.md` / `SHA256SUMS` / `LICENSES/*` / `build/*` — их
перекрывает главный, и это единственное место, где порядок что-то решает. Собранный
`site/`: **2796 файлов, ~566 МиБ** (`web-apps` 1836, `assets` 90, `sdkjs/common` 396,
`fonts` 203 файла / ~184 МиБ). Кэш — **~292 МиБ** в `.cache/` (тарболы частей +
зеркало шрифтов); повторный `assemble` их не перекачивает.

### Каталог шрифтов (`site/fonts/`)

Табличный редактор запрашивает у сайта движковый каталог `/fonts/<индекс>`
(203 файла, суммарно ~184 МБ) по одному файлу за раз, и без него не рисует.
npm-части `eurooffice-fonts-1`/`-2`, которые должны нести каталог, **не
опубликованы**, поэтому `assemble` зеркалирует его из репозитория
[ranuts/document](https://github.com/ranuts/document) (`public/fonts/` — те же
файлы, что на живом сайте): берёт список у GitHub API (без токена, с ретраями),
скачивает `download_url` в `.cache/fonts/` (конкурентность 8, по 2–3 ретрая на
файл; файл из кэша берётся, если он есть и ненулевой) и копирует в
`site/fonts/`; файлы каталога, которых нет в списке, удаляются, чтобы
пересборка не копила мусор. Сеть не роняет сборку: при стойкой неудаче —
громкое предупреждение «шрифты не скачаны — табличный редактор не будет
рисовать» и выход 0 (поломку поймает браузерная приёмка).

Файлы каталога **XOR-обфусцированы намеренно** (ключ есть в `assets/document-*.js`
и `sdkjs/slide/sdk-all*.js` собранного сайта) — де-XOR выполняет код приложения
в браузере, поэтому файлы зеркалируются как есть, без де-обфускации.

Чего в сборке нет и почему:

- **`x2t.wasm.br`** — части публикуют только несжатый `x2t.wasm` (35,8 МиБ), а редактор
  ходит за `.br`-именем. `assemble` делает brotli-копию сам (zlib из Node, качество 11:
  6,8 МиБ, ~50 с, при повторном запуске пропускается), и `serve.mjs` отдаёт её с
  `Content-Encoding: br` — ровно так, как сайт устроен в `_headers`. Если копии нет,
  сервер честно отдаёт несжатый `x2t.wasm` без заголовка кодирования (URL с `.br` — это
  только имя).

## Наш просмотрщик (TypeScript)

`viewer.html` — тонкая оболочка (каркас страницы и ссылки на `dist/`); вся логика —
в `src/viewer/`, компилируется `tsc` в `dist/` (`pnpm run build`), `assemble`
копирует `dist/` и `viewer.html` в `site/`.

- `src/viewer/protocol.ts` — типы и клиент Embed API (`createEditorClient`):
  команды `document:open-buffer/open-file/open-url/set-readonly/save/get-state`,
  события `document:ready/opened/saved/state/dirty-changed/readonly-changed/error`;
  очередь команд до `document:ready`, уникальные id, ответные события матчатся
  по id (запоздавший ответ отменённой команды не доставляется), проверка origin
  и source; `describeEditorError` расшифровывает коды ошибок (-82/-85);
- `src/viewer/main.ts` — UI-оболочка: тулбар, выбор файла (кнопка + drag&drop
  с оверлеем поверх области просмотра), статус, iframe. Файл читает родитель
  (`file.arrayBuffer()`) и шлёт `document:open-buffer { fileName, buffer, readonly: true }`;
  выбор до `document:ready` кладётся в очередь и открывается по готовности;
- `src/viewer/style.css` — свой компактный «порт» визуального языка сайта:
  значения сняты с `site/ran-tokens.*.css` (серые/статусные шкалы ran-*, радиусы
  6/12/9999, стек Geist), шрифты — `site/ran-fonts/`; тёмный тулбар, имя документа
  по центру, статус с точкой-индикатором справа;
- статус — `#viewer-status` с `data-state="ready|opening|opened|error"` (плюс
  начальный `loading`), ошибки редактора расшифровываются (код -82 — конвертация
  не удалась, -85 — содержимое не совпадает с расширением), кнопок
  сохранения/редактирования нет;
- повторный выбор файла открывает новый документ в том же iframe;
- `window.euroofficeViewer` (`state`, `openedCount`, `lastFileName`, `lastError`) — хук для
  `tools/viewer-check.mjs`: по счётчику открытий проверка не путает новый файл со старым
  `data-state="opened"`.

## Запуск

```bash
pnpm install                      # свой node_modules и lockfile (каталог вне pnpm-workspace)
pnpm run build                    # tsc → dist/main.js + dist/style.css
node scripts/assemble.mjs         # ~292 МиБ в .cache/ (части + шрифты) + ~566 МиБ в site/; идемпотентен
node scripts/serve.mjs            # http://127.0.0.1:5271/viewer.html (редактор — /editor)
node tools/viewer-check.mjs       # приёмка Playwright: docx и xlsx → document:opened + canvas
```

`pnpm run assemble` = `pnpm run build` + `node scripts/assemble.mjs`; `assemble`
сам требует готовый `dist/` и падает с подсказкой, если его нет.

`serve.mjs` — статик-сервер для `site/`: порт из аргумента или `VIEWER_PORT`/`PORT`
(по умолчанию 5271), MIME-карта (`.wasm` — `application/wasm`, шрифты, картинки), clean
URL (`/editor` → `editor.html`), `.br`-файлы с `Content-Encoding: br`, никаких листингов,
`..` не выпускается за корень, 404 — текстом (умеет HEAD — так его проверяет curl).

`viewer-check.mjs` берёт Playwright из `examples/viewer` (браузеры стоят под его сборку),
сам поднимает `serve.mjs` на свободном порту и гасит его, гоняет
`test-fixtures/docx/styles/doc_defaults.docx` и самый мелкий непустой
`test-fixtures/xlsx/content-*.xlsx`, ждёт `data-state="ready"` (до 120 с) и `"opened"`
(до 180 с), ищет canvas с ненулевыми размерами в рамке редактора (и во вложенных),
складывает скриншоты и `viewer-check.json` в `/tmp/eurooffice-viewer-check`.
Переопределения: `VIEWER_URL` (готовый сервер), `VIEWER_PORT`, `VIEWER_OUT`,
`VIEWER_READY_TIMEOUT`, `VIEWER_OPEN_TIMEOUT`. Сообщения консоли браузера печатаются, но
приёмку валят не они, а неоткрывшийся документ или отсутствие canvas.

## Что проверено и что нет

Проверено без браузера (сборка, сервер, состав):

- `assemble` собран из частей и идемпотентен (повторный прогон — кэш и распаковка поверх,
  brotli не пересчитывается); brotli-копия разжимается обратно в `x2t.wasm` байт-в-байт;
- `serve.mjs`: `/editor` → `200 text/html`; `assets/editor-*.js` → `200 text/javascript`;
  `sdkjs/common/wasm/x2t/x2t.wasm.br` → `200`, `application/wasm`, `Content-Encoding: br`;
  `viewer.html` → `200 text/html`; `dist/main.js` → `200 text/javascript`;
  `dist/style.css` → `200 text/css`; `/` → `index.html`; `..`-пути с процентным
  кодированием → 404;
- сборка TS-оболочки: `pnpm run build` → `dist/main.js` + `dist/style.css`, tsc в strict-режиме;
- состав `site/`: `editor.html`, `assets/editor-*.js`, `sdkjs/word/sdk-all.js`,
  `ran-fonts/` со шрифтами, x2t со вдвое большим `sdk-all.js` — все на месте;
- зеркало шрифтов: в `site/fonts/` 203 файла (~184 МиБ), как в списке GitHub API;
  повторный `assemble` берёт файлы из `.cache/fonts/` без загрузок; лишние файлы
  каталога удаляются; `/fonts/062` → `200 application/octet-stream` (у файлов нет
  расширения — так их отдаёт и MIME-карта `serve.mjs`).

Не проверено здесь: браузерный прогон. Сторонний wasm в окружении субагента не
запускается, `viewer-check.mjs` запускает координатор. Ожидаемая точка отказа —
сеть: редактору нужны первые ~35 МиБ JS/WASM с localhost; каталог шрифтов на
месте, и spreadsheeteditor должен рисовать таблицы.

## Ограничения

- Просмотр — **только чтение**: `readonly: true`, сохранение выключено (в этом режиме
  `document:save` отвечает ошибкой). Редактор в сборке полноценный, но обвязка спайка
  умышленно не даёт сохранять.
- PWA/офлайн, совместное редактирование и прочие фичи сайта — не наша забота: сайт
  регистрирует свой service worker и живёт, как живёт; спайк его не трогает.
- Сервер — дев-статик для спайка: без сжатия на лету и Range; для продакшена нужен
  нормальный хостинг статики (у сайта для этого есть `_headers`).
- Пакеты сайта — **AGPL-3.0-only** (у каждой части свои `LICENSES/`); скачиванием и
  раздачей спайк ничего не лицензирует заново.

## Файлы

- `package.json`, `tsconfig.json`, `pnpm-lock.yaml` — сборка TS-оболочки (devDep: typescript);
- `pnpm-workspace.yaml` — пустой (packages: []): каталог — самостоятельный pnpm-проект,
  без него `pnpm install` отсюда ушёл бы в корневой workspace;
- `src/viewer/protocol.ts` — типы и клиент Embed API;
- `src/viewer/main.ts` — UI-оболочка просмотрщика;
- `src/viewer/style.css` — стили оболочки (в `dist/` их кладёт скрипт сборки);
- `scripts/copy-assets.mjs` — копия `src/viewer/style.css` → `dist/style.css` (часть `pnpm run build`);
- `scripts/assemble.mjs` — скачивание частей в `.cache/`, overlay в `site/`, копирование
  `viewer.html` и `dist/`, brotli-копия x2t, проверка обязательных путей;
- `scripts/serve.mjs` — статик-сервер `site/` (clean URL, MIME, `.br`);
- `viewer.html` — тонкая HTML-оболочка страницы-просмотрщика;
- `tools/viewer-check.mjs` — приёмка Playwright (браузерный прогон — за координатором);
- `site/`, `.cache/`, `dist/`, `node_modules/` — генерируемые, в `.gitignore`.
