# Демо на GitHub Pages: сборка и дым-прогон обоих маршрутов (S2.3)

Прогон 13.10.2026, локально (macOS). Цель слайса: убедиться, что `pages.yml`
собирает демо-приложение с обеими страницами («Слово» `/#/docx`, «ДваИксель»
`/#/xlsx`) и что на Pages — сервере без заголовков COOP/COEP — штатно
работает `bytes`-транспорт (ADR-0021).

## Что делает pages.yml

**Прогон ниже — исторический** (13.10.2026): он описывает пайплайн на тот
момент, когда публиковалось только демо и корень сайта был самим приложением.
Текущий пайплайн шире и собирает на один сайт два артефакта — движок
Euro-Office в корне и демо в `/viewer/`:

1. сборка wasm (`scripts/build-wasm.sh`) → `core` → `browser-convert...` →
   `@doc-converter/eurooffice`;
2. сборка сайта движка с кэшем `build/eurooffice/.cache` и
   `EUROOFFICE_X2T_BR=copy node scripts/eurooffice/assemble.mjs`;
3. сборка демо с `VITE_BASE=/<имя-репозитория>/viewer/`;
4. `node scripts/pages/stage.mjs --out .pages` — раскладка артефакта
   публикации, затем upload `.pages`.

Датированный прогон ниже не переписывался и не удалялся: он остаётся
свидетельством того, что было проверено руками — сборкой, сервингом в режиме
Pages и дым-прогоном Playwright.

Workflow `.github/workflows/pages.yml` (triggers: push в `main`,
`workflow_dispatch`; permissions: `pages: write`, `id-token: write`;
concurrency-группа `pages` без отмены — деплои Pages не параллелятся):

1. Устанавливает Rust 1.98.0 + target `wasm32-unknown-unknown`, `wasm-pack`,
   pnpm и Node 20.
2. `pnpm install --frozen-lockfile` (с fallback на `--no-frozen-lockfile`).
3. `bash scripts/build-wasm.sh` — пересобирает `pkg/` всех трёх wasm-модулей
   из текущих исходников (в репозитории `pkg/` нет).
4. `pnpm -F @doc-converter/core build` — core импортируется из `dist/`.
   Turborepo намеренно не используется: strict-режим не пробрасывает
   незадекларированную `VITE_BASE`.
5. `VITE_BASE=/${{ github.event.repository.name }}/` (то есть `/doc-converter/`)
   и `pnpm -F @doc-converter/example-viewer build`: база запекается в
   `index.html` и пути ассетов.
6. Проверяет, что `examples/viewer/dist/coi-serviceworker.js` непустой
   (service worker, который на Pages сам отдаёт COOP/COEP после первой
   загрузки и включает SAB).
7. `configure-pages` → `upload-pages-artifact` (`examples/viewer/dist`) →
   джоба `deploy` через `deploy-pages`.

Ошибок в workflow не найдено; файл не правился. Замечание (не ошибка,
информационно): комментарий к шагу 6 говорит «без него SharedArrayBuffer не
поднимется и пример покажет пустой лист» — это устарело после ADR-0021:
без service worker вьюер работает на `bytes` (подтверждено ниже), SAB просто
не включается. Комментарий актуализировать не стал — правка `pages.yml` вне
допустимого для слайса набора файлов.

## Что проверено локально

### Сборка

- `pnpm build:wasm` — зелёный: `pkg/` трёх модулей пересобран из текущего
  дерева (включая правки `crates/docx/src/layout/*` других слайсов).
- `pnpm -F @doc-converter/example-viewer build` (база по умолчанию `/`) —
  зелёный.
- `VITE_BASE=/doc-converter/ pnpm -F @doc-converter/example-viewer build` —
  зелёный; в `dist/index.html` пути ассетов запечены как
  `/doc-converter/assets/…`, в бандле присутствуют оба ленивых чанка страниц
  (`DocxPage-*.js`, `XlsxPage-*.js`) — обе страницы существуют в сборке, а не
  только дефолтная.
- `dist/coi-serviceworker.js` непустой — шаг `check coi service worker`
  из pages.yml прошёл бы на этой сборке.

### Сервинг в режиме Pages (без COOP/COEP)

Собранный `dist/` раздавался минимальным статическим сервером на Node **без
каких-либо заголовков COOP/COEP**, под префиксом `/doc-converter/` — так, как
отдаёт GitHub Pages. `curl`-проверка — всё 200, 404 нет:

```
200  /doc-converter/                    200  /doc-converter/#/docx
200  /doc-converter/#/xlsx              200  /doc-converter/coi-serviceworker.js
200  /doc-converter/fixtures/{docx,xlsx}/index.json
200  /doc-converter/assets/index-*.js   200  /doc-converter/assets/*.wasm (все три)
```

Хеш-маршруты на сервер не попадают (hash routing), поэтому `/#/docx` и
`/#/xlsx` с точки зрения сервера — один и тот же `index.html`; сами маршруты
проверены браузерным прогоном ниже.

### Дым-прогон Playwright (Chromium, установленные браузеры)

Для каждого маршрута: сбор ошибок консоли и `pageerror`, ожидание моста
`window.docConverter` нужного вида, чтение `viewer.transport` и
`crossOriginIsolated`, затем открытие фикстуры через мост
(`viewer.open(…)`) — полный конвейер worker + wasm + парсер + кадр.
Прогон в двух режимах:

- **Run A — service worker включён** (ровно как на настоящих Pages: COI-SW
  регистрируется, после перезагрузки страницы сам отдаёт COOP/COEP).
- **Run B — service worker заблокирован** (COOP/COEP нет в принципе,
  `crossOriginIsolated === false`: чистый `bytes`).

| Маршрут | Режим | Транспорт | crossOriginIsolated | Фикстура открылась | Ошибки консоли |
| --- | --- | --- | --- | --- | --- |
| `/#/docx` | A (SW) | `sab` (после перезагрузки SW) | true | да, 1 страница отрисована | нет |
| `/#/xlsx` | A (SW) | `sab` (после перезагрузки SW) | true | да, 1 лист отрисован | нет |
| `/#/docx` | B (без SW) | **`bytes`** | **false** | да, 1 страница отрисована | нет¹ |
| `/#/xlsx` | B (без SW) | **`bytes`** | **false** | да, 1 лист отрисован | нет¹ |
| `/` (корень) | B (без SW) | **`bytes`** | **false** | да, редирект на DOCX работает | нет¹ |

¹ В режиме B единственная запись — `pageerror` от вендоренного
`coi-serviceworker` v0.1.7 (`Cannot read properties of undefined (reading
'scope')`): при заблокированной Playwright'ом регистрации SW `register()`
разрешается `undefined`. Это артефакт среды прогона: на настоящих Pages
регистрация успешна (режим A, ошибок ноль), а реальный аналог «SW недоступен»
(Firefox private mode) скрипт обрабатывает ранним выходом. На работу
приложения ошибка не влияет: вьюер, воркер, wasm и рендер в режиме B отработали
полностью. Чинить вендоренный скрипт не стал — `public/` вне допустимого
набора файлов слайса.

## Вердикт по bytes-транспорту

**Работает штатно.** На сервере без COOP/COEP автовыбор транспорта
(`crossOriginIsolated === false` → `tryAllocSab` недоступен → `bytes`) прошёл
на обоих маршрутах и на корне: вьюер инициализировался, воркер поднялся,
wasm загрузился, документ и книга распарсились и отрисовались в canvas —
без единой ошибки приложения. Первая отрисовка на Pages идёт именно по этому
пути (service worker включается только со второй загрузки).

## Правки конфига

Не потребовались: `vite.config.ts` (base из `VITE_BASE`), `package.json` и
`index.html` не трогались, исходники вьюера тоже. Единственный артефакт
слайса — этот файл.

## Что осталось проверить только на самом Pages (деплой)

1. **Живой деплой**: сам прогон `pages.yml` (push в `main` или
   `workflow_dispatch`) и открытие публичного URL `/<имя-репозитория>/` по
   обоим маршрутам. Локально воспроизведены все шаги джобы `build`
   (включая префикс базы и check COI-SW), но не сам `deploy-pages`.
2. **SW на реальном домене (HTTPS, не localhost)**: локальный режим A
   симулирует это на localhost; на Pages регистрация SW и COOP/COEP после
   первой перезагрузки должны пройти так же.
3. **Браузерная матрица bytes-пути**: дым здесь — только Chromium.
   Существующий e2e-набор просмотрщика гоняет Firefox/WebKit на dev-сервере
   (COOP/COEP, `sab`); сквозной прогон `bytes` в Firefox/WebKit на собранном
   дисте остаётся опциональным.
