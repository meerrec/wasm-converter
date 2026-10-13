# Спайк: просмотрщик на Euro-Office (self-host из npm-частей)

Self-host статического сайта Euro-Office — это редакторы ONLYOFFICE (форк Euro-Office)
целиком в браузере, с конвертером x2t в WebAssembly, — и наша страница `viewer.html`
поверх него: она встраивает редактор iframe'ом (`/editor.html?embed=1`) и открывает
файлы через Embed API (`postMessage`) в режиме только просмотра. Файл читает родитель
и передаёт байтами (`document:open-buffer`) — ни CORS, ни сети для этого не нужно.

Сайт не собирается из исходников: он скачивается шестью готовыми npm-частями
`@kreijstal/eurooffice-*` (версия пинится одной константой) и раскладывается overlay'ем
в один каталог `site/`. Ни одной своей зависимости у спайка нет — только Node и `tar`;
каталог лежит вне pnpm-workspace, как `experiments/x2t-spike`.

## Состав сайта

| Часть | Тарбол | Размер | Файлов |
|---|---|---|---|
| `@kreijstal/eurooffice` (главный) | 19,5 МиБ | 92,4 МиБ | 339 |
| `@kreijstal/eurooffice-sdkjs-common` | 44,7 МиБ | 123,2 МиБ | 422 |
| `@kreijstal/eurooffice-web-apps-1` | 25,1 МиБ | 86,1 МиБ | 1034 |
| `@kreijstal/eurooffice-web-apps-2` | 16,7 МиБ | 84,4 МиБ | 1025 |
| `@kreijstal/eurooffice-fonts-1` | — | — | — |
| `@kreijstal/eurooffice-fonts-2` | — | — | — |

**Порядок overlay** (в `scripts/assemble.mjs` — массив `PARTS`): подчинённые части
первыми, главный пакет последним, файлы поверх:

1. `eurooffice-fonts-1`, `eurooffice-fonts-2` — **в npm не опубликованы** (HTTP 404
   на реестре, проверено 14.10.2026), `assemble` говорит об этом и продолжает;
2. `eurooffice-sdkjs-common` → `sdkjs/common/**` (включая `sdkjs/common/wasm/x2t/`);
3. `eurooffice-web-apps-1` → `web-apps/apps/{common,api,pdfeditor,presentationeditor,visioeditor}`, часть `sdkjs/common/Images`;
4. `eurooffice-web-apps-2` → `web-apps/apps/{common,documenteditor,spreadsheeteditor}`;
5. `eurooffice` → HTML-страницы, `assets/`, `img/`, `ran-fonts/`, `sdkjs/{word,cell,slide,vendor,visio}`, `sw.js` и прочая обвязка сайта.

Пересечение реально проверено: у `web-apps-1` и `web-apps-2` совпадают 190 файлов, и все
они побайтово одинаковы (сплит разрезал дерево, но не менял содержимое); с главным
пакетом пересекаются только `README.md` / `SHA256SUMS` / `LICENSES/*` / `build/*` — их
перекрывает главный, и это единственное место, где порядок что-то решает. Собранный
`site/`: **2588 файлов, ~388 МиБ** (`web-apps` 1836, `assets` 90, `sdkjs/common` 396).
Кэш тарболов — **~108 МиБ** в `.cache/`; повторный `assemble` их не перекачивает.

Чего в сборке нет и почему:

- **`site/fonts/` (движковый каталог шрифтов)** — его несли неопубликованные
  `fonts-1`/`fonts-2`. Обвязка редактора терпит отсутствие: каталог она тянет запросами
  `${base}fonts/<index>` и на неуспешный ответ просто пропускает запись (`if (!r.ok) return`),
  так что латинские документы открываются; CJK-шрифты из каталога не подтянутся.
- **`x2t.wasm.br`** — части публикуют только несжатый `x2t.wasm` (35,8 МиБ), а редактор
  ходит за `.br`-именем. `assemble` делает brotli-копию сам (zlib из Node, качество 11:
  6,8 МиБ, ~50 с, при повторном запуске пропускается), и `serve.mjs` отдаёт её с
  `Content-Encoding: br` — ровно так, как сайт устроен в `_headers`. Если копии нет,
  сервер честно отдаёт несжатый `x2t.wasm` без заголовка кодирования (URL с `.br` — это
  только имя).

## Наш просмотрщик (viewer.html)

- iframe `src="/editor.html?embed=1"` (clean URL `/editor` сервер тоже умеет — сайт на
  него ссылается);
- после `document:ready` страница читает выбранный файл сама (`file.arrayBuffer()`) и
  шлёт `document:open-buffer` `{ fileName, buffer, readonly: true }`; выбор до
  `document:ready` кладётся в очередь и открывается по готовности;
- статус — `#viewer-status` с `data-state="ready|opening|opened|error"`, ошибки редактора
  расшифровываются (код -82 — конвертация не удалась, -85 — содержимое не совпадает с
  расширением), кнопок сохранения/редактирования нет;
- повторный выбор файла открывает новый документ в том же iframe;
- `window.euroofficeViewer` (`state`, `openedCount`, `lastFileName`, `lastError`) — хук для
  `tools/viewer-check.mjs`: по счётчику открытий проверка не путает новый файл со старым
  `data-state="opened"`.

## Запуск

```bash
node scripts/assemble.mjs         # ~110 МиБ загрузок в .cache/ + ~390 МиБ в site/; идемпотентен
node scripts/serve.mjs            # http://127.0.0.1:5271/viewer.html (редактор — /editor)
node tools/viewer-check.mjs       # приёмка Playwright: docx и xlsx → document:opened + canvas
```

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
  `viewer.html` → `200 text/html`; `/` → `index.html`; `..`-пути с процентным кодированием →
  404;
- состав `site/`: `editor.html`, `assets/editor-*.js`, `sdkjs/word/sdk-all.js`,
  `ran-fonts/` со шрифтами, x2t со вдвое большим `sdk-all.js` — все на месте.

Не проверено здесь: браузерный прогон. Сторонний wasm в окружении субагента не
запускается, `viewer-check.mjs` запускает координатор. Ожидаемые точки отказа — каталог
`site/fonts/` (отсутствует) и сеть: редактору нужны первые ~35 МиБ JS/WASM с localhost.

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

- `scripts/assemble.mjs` — скачивание частей в `.cache/`, overlay в `site/`, копирование
  `viewer.html`, brotli-копия x2t, проверка обязательных путей;
- `scripts/serve.mjs` — статик-сервер `site/` (clean URL, MIME, `.br`);
- `viewer.html` — наша страница-просмотрщик поверх Embed API;
- `tools/viewer-check.mjs` — приёмка Playwright (браузерный прогон — за координатором);
- `site/`, `.cache/` — генерируемые, в `.gitignore`.
