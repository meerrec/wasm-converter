<!-- BEGIN:turborepo-agent-rules -->

# This is NOT the Turborepo you know

Turborepo configuration, task behavior, and CLI commands can vary between installed versions and may differ from your training data. Resolve the `turbo` package from this file's directory or relevant workspace; in monorepos, it may not be visible from the repository root. For example, run `node -p "require.resolve('turbo/package.json')"` from a workspace that depends on `turbo`.

Read `docs/README.md` inside that installed package first, then read the relevant pages from its `docs/` directory before changing Turborepo configuration or commands. Heed deprecation notices. These bundled docs match the installed package version and are available without network access.

This block is written and re-added by `turbo` before repository-scoped commands when an AI agent is detected. In the Turborepo source repository, its template is defined in `crates/turborepo-cli/src/cli/agent_guidance.rs`. Removing the managed block while updates are enabled means a later qualifying invocation will add it again. Set `"agentGuidance": false` in the root `turbo.json` or `turbo.jsonc` to opt out; this does not remove an existing block. Keep the block committed with your work to avoid an uncommitted change on the next agent invocation.
<!-- END:turborepo-agent-rules -->

# Project guidance

Этот файл — инструкция для агентов OpenCode; его близнец для Claude Code — `CLAUDE.md`. Правки конвенций и команд дублируйте в оба файла, иначе они разъедутся.

## Что это

- Монорепозиторий **doc-converter**: просмотрщик OOXML (DOCX/XLSX) и экспортёр в PDF/Markdown/HTML/текст — Rust/WASM, рендер на `OffscreenCanvas` в Web Worker; batch-конвертация — Node-CLI. Cargo workspace (`crates/*`) + pnpm workspaces (`packages/*`, `examples/*`) + Turborepo.
- Планы, бюджеты и DoD — `ROADMAP.md` (§7 состояние, §9 бюджеты, §12 DoD). Закрыты спринты 1–13 (сводка — ROADMAP §7, отчёты — `docs/sprint-6..13`, последний — `docs/sprint-13/report.md`): Спринт 12 — релиз v1.0.0, ожидает публикацию (S4.3); Спринт 13 — спайк LOWA, закрыт решением **no-go** (ADR-0023), попутный пакет-обвязка x2t — ADR-0024. «Статус» корневого README устарел — не ориентир.
- Незаконченное помечено `TODO (…)` / `todo!("…")` — ищите маркеры, а не только роадмап. Архитектурные решения — `docs/adr/` (индекс в `docs/adr/README.md`; принятый ADR не правят, а заводят новый).

## Команды

### Rust

```bash
cargo test --workspace --all-features   # нативные тесты; CI гоняет с --all-features
cargo test -p doc-converter-render      # один крейт
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps   # CI валит битые intra-doc-ссылки
cargo deny check && cargo audit
cargo llvm-cov -p doc-converter-docx --fail-under-lines 85   # гейты CI: docx ≥ 85 %, core ≥ 80 % строк
wasm-pack test --headless --chrome --release crates/wasm     # так же crates/docx-wasm и crates/pdf-wasm; chromedriver лучше в PATH: без него wasm-pack качает «самый свежий», который может разойтись с браузером
```

- `crates/wasm`, `crates/docx-wasm`, `crates/pdf-wasm` целиком под `#![cfg(target_arch = "wasm32")]`: нативный clippy/test их не видит, доккомментарии проверяет только wasm32-нога CI (`cargo doc --target wasm32-unknown-unknown -p …`).
- criterion-бенчи живут в крейтах (`crates/render/benches/render.rs`, `crates/pdf/benches/pdf.rs`, `crates/xlsx/benches/{open,conditional,hit_test}.rs`, `crates/docx/benches/{open,pagination}.rs`): `cargo bench -p doc-converter-<крейт>`. Baseline'ов нет — «до/после» берётся из локального прогона. CI компилирует все бенчи (`cargo bench --workspace --no-run`) — бенчи обязаны собираться.
- `crates/fuzz` — отдельный workspace (исключён из корневого, свой nightly). Фаззинг: `cargo fuzz run docx_document` (цели `core_{zip,xml_reader,rels}`, `xlsx_worksheet`, `docx_{document,numbering,rels,styles}`).

### Сборка WASM

```bash
pnpm build:wasm   # = bash scripts/build-wasm.sh
```

Собирает три модуля: `crates/wasm` → `packages/wasm/pkg` (просмотрщик), `crates/pdf-wasm` → `packages/wasm-pdf/pkg` (экспорт PDF), `crates/docx-wasm` → `packages/wasm-docx/pkg` (просмотр DOCX), затем системный `wasm-opt -O4`, если он в `PATH`. Встроенный в wasm-pack wasm-opt отключён: rustc эмитит bulk-memory, который та версия не понимает. `packages/*/pkg/` — сгенерированное и в `.gitignore`; руками не править.

CI удерживает разрезы: сверяет `.d.ts` ленивых модулей с `#[wasm_bindgen]`-функциями крейтов и падает, если в основном модуле появились экспорты `docx_*` (их возврат — регрессия бюджета `wasm (viewer)` и нарушение ленивости).

### JS/TS

```bash
pnpm install
pnpm build:wasm                        # ОБЯЗАТЕЛЬНО до typecheck/test: core типизируется против сгенерированных .d.ts
pnpm turbo run typecheck test build    # то же, что гоняет CI
pnpm -F @doc-converter/core test       # vitest
pnpm -F @doc-converter/core exec vitest run test/tick.spec.ts        # один файл
pnpm -F @doc-converter/core exec vitest run -t 'coalesces repeated'  # один тест
pnpm -F @doc-converter/cli build && pnpm cli convert report.docx -o report.pdf
pnpm -F @doc-converter/example-viewer exec playwright test e2e/docx --project=chromium   # один e2e
pnpm size-limit                        # после build; бюджеты — .size-limit.mjs
pnpm gen:fixtures                      # XLSX-фикстуры + эталон exceljs; тяжёлые книги — в target/fixtures
pnpm gen:docx-fixtures                 # DOCX-фикстуры; :large (≥ 10 МиБ) — в target/fixtures/docx-large
pnpm diff:mammoth                      # differential DOCX против mammoth (oracle-out-docx/ в .gitignore)
pnpm bench:js                          # vs mammoth/exceljs/xlsx, отчёт — docs/bench-js.md
pnpm -F @doc-converter/example-viewer test   # весь Playwright: 3 браузера + проект chromium-dpr2 (deviceScaleFactor=2, только dpr.spec.ts)
```

- Порядок не косметический: без `pnpm build:wasm` typecheck падает на отсутствующих `pkg/*.d.ts`; в CI `pkg/` трёх модулей приезжает артефактами джобы `wasm`.
- E2E просмотрщика идут в настоящих Chromium/Firefox/WebKit (сервер поднимает Playwright, порт 5178). Браузеры ставятся один раз: `pnpm -F @doc-converter/example-viewer exec playwright install chromium firefox webkit` — без них `turbo run test` падает.
- `pnpm lint` не работает: eslint не установлен (скрипт `lint` объявлен только у `core`); как проверку не использовать.
- Эталонный оракул против LibreOffice (`scripts/oracle-docx-pages.sh`, `scripts/oracle-ssim.sh`) требует системные `soffice`/`pdftoppm`/`ffmpeg`; без них локальный прогон штатно скипается. Живёт в отдельном workflow `Oracle`, методика — `docs/testing/`.
- Бюджеты памяти снимаются `scripts/check-pdf-memory.sh` и `scripts/check-docx-memory.sh` (жёсткие лимиты CI: 40 МиБ PDF, 200 МиБ DOCX).

### Turborepo

- Установлен turbo 2.11.5 (`package.json` объявляет `^2.3.3`). Перед правкой `turbo.json` или turbo-команд читайте бандленные доки именно этой версии: `node_modules/.pnpm/turbo@2.11.5/node_modules/turbo/docs/`.
- Граф: `build` → `^build`; `test` → `build`; `typecheck` → `^build`; `clean` без кэша. `packages/wasm{,-pdf,-docx}/turbo.json` переопределяют `build.outputs: []` — `pkg/` не кэшируется, его собирает скрипт.
- Блок `<!-- BEGIN:turborepo-agent-rules -->` в начале файла пишет и переписывает сам turbo; не удаляйте его из коммитов.

## Архитектура

### Крейты

| Крейт | Роль |
|---|---|
| `crates/core` | OOXML-примитивы: `Archive` (обёртка над `zip`), `XmlReader` (streaming `quick-xml`, пропускает Decl/Comment/PI/DocType), rels, `Error` (thiserror + miette `Diagnostic`). Ничего не знает про DOCX/XLSX |
| `crates/render` | painter (OffscreenCanvas, только wasm32), `DisplayList`, SAB-кольцо, `geometry`, `text_measure`, `font` (метрики через `skrifa`); общая граница `xlsx`/`docx` (ADR-0003) |
| `crates/xlsx` | разбор книги целиком (`open()`), раскладка листа в пикселях, сборка кадра (`paint::build`) |
| `crates/docx` | разбор (`open()`/`parse_docx`), пагинация, сборка кадра (`render::build` / `build_stack`) |
| `crates/pdf` | экспорт в PDF собственным бэкендом (`src/pdf/`, ADR-0022): лист/книга XLSX (`export_xlsx_sheet`/`export_xlsx_book`), документ DOCX (`export_docx`) |
| `crates/wasm` | основной wasm-модуль просмотрщика: painter, SAB-аллокатор, XLSX-просмотрщик |
| `crates/pdf-wasm` | ленивый модуль PDF-экспорта: `export_pdf`, `export_pdf_book`, `export_docx_pdf` |
| `crates/docx-wasm` | ленивый модуль просмотра DOCX: экспорты `docx_*`, грузится воркером при открытии `.docx` |

### Пакеты

| Пакет | Роль |
|---|---|
| `@doc-converter/core` | воркер просмотра (прямой протокол, оба транспорта кадра), OffscreenCanvas-просмотрщики XLSX и DOCX |
| `@doc-converter/wasm`, `-wasm-pdf`, `-wasm-docx` | сгенерированные wasm-bindgen-биндинги; `pkg/` в `.gitignore`, не редактировать |
| `@doc-converter/cli` | `convert` и `batch` на тех же wasm-модулях, пул воркеров |
| `examples/viewer` | демо (Vite + React) и Playwright-e2e; страницы «Слово» `/#/docx`, «ДваИксель» `/#/xlsx`, «Конвертер» `/#/convert`, Euro-Office `/#/eurooffice` |

### Контракт Rust ↔ TS: DisplayList

`crates/render/src/display_list.rs` — бинарный формат кадра: магия `"DLST"`, `DL_VERSION = 4`, заголовок 20 байт, команды с однобайтовыми тегами, строки — в общем string pool (`StringRef { off, len }`). TS-сторона кадр только переносит, не разбирает; painter читает его прямо из SAB и не может спросить у вызывающего, что за шрифт номер три, — формат обязан быть самодостаточным. Меняя раскладку, синхронизируйте `DL_VERSION`, теги и потребителей; это стоп-сигнал (`docs/agent-contract.md` §5).

Команда `Text` несёт гарнитуру строкой в общем пуле и флаги начертания, у `Line` рисунок штриха — `LineStyle` (толщина остаётся в `stroke_w`). Цвет — `RRGGBBAA`; в OOXML он записан как `AARRGGBB`, поэтому `xlsx::paint::resolve_color` каналы переставляет.

### Транспорты кадра и SAB-кольцо

Два равноправных транспорта (ADR-0021): `sab` — кольцо из двух слотов в `SharedArrayBuffer` (при COOP/COEP), `bytes` — `ArrayBuffer` + transfer (iframe, расширения, GitHub Pages). Выбор автоматический по `crossOriginIsolated`; опция `transport: 'sab' | 'bytes' | 'auto'`, явный `sab` без заголовков — ошибка инициализации, а не тихая подмена. Кадр и API одинаковы, отличается транспорт.

Раскладка кольца продублирована и должна совпадать: `crates/render/src/sab/ring.rs` (`SabRing` на Atomics для wasm, чистый `RingState` для нативных тестов) ↔ `packages/core/src/sab/protocol.ts` + `reader.ts` (`HEADER_INTS = 4`, `SLOT_COUNT = 2`). Нет свободного слота — кадр дропается (`dropped: true`), а не копится в очереди.

### Просмотр XLSX и DOCX

`createXlsxViewer` (`packages/core/src/render/xlsx_viewer.ts`) собирает разметку сам: прокручиваемая область, распорка по размеру листа и холст, приклеенный к видимой части трансформом. Единицы в запросе кадра разные и это важно: `viewport.x/y` — в пикселях раскладки (до зума), `w/h` — в физических пикселях canvas, `scale` — зум, умноженный на DPR.

Книга живёт в `thread_local!` воркера (`crates/wasm/src/xlsx_api.rs`), кадр собирается сразу в слот ring'а: гонять сотни килобайт команд через `postMessage` значило бы платить за копирование на каждом кадре. `createDocxViewer` (`packages/core/src/render/docx_viewer.ts`) устроен так же, но в ленивом модуле `crates/docx-wasm`: `docx_build_display_list*` рисуют стопку страниц.

### Протокол Main ↔ Worker

Канал один — прямой: сообщения `InMsg`/`OutMsg` из `packages/core/src/protocol.ts`, без конверта `{id, type, payload}` и слоя `call`/`notify`. Источник истины — TS-файл; Rust описывает только payload. Кадр не ждёт ответа: воркер на `render` не отвечает, а шлёт `tick` со статистикой (`PaintStats`); без `id` приходят и `ready` на `init`, и `opened`/`docx-opened` на `open`. У части команд ответ коррелируется по `id`: `hit-test` → `hit`, `hyperlink-at` → `hyperlink`, `export-png` → `png`, `export-pdf`/`export-docx-pdf` → `pdf`, `export-docx-text` → `docx-text`. Ошибка — отдельное `error` (`WorkerErrorMsg`) без `id`.

Поток: main вызывает `canvas.transferControlToOffscreen()` и один раз отдаёт canvas воркеру (`render/offscreen.ts:initOffscreen`) → воркер инициализирует wasm, делает `alloc_sab`, отвечает `ready` с SAB → `ResizeObserver` (debounce 100 мс, DPR клампится по лимиту 16M пикселей в `render/resize_observer.ts:computeDpr`) шлёт `resize` + `render` → воркер собирает DisplayList в слот → painter рисует → `tick`. Кадры коалесцируются через `requestAnimationFrame` в `worker/frame_loop.ts`; ошибки кадра логируются, loop не роняется. Без COOP/COEP работает тот же путь на `bytes` — детали в `docs/workers.md`.

### Шрифты

Четыре подрезанных Carlito лежат в `crates/render/src/fonts/`; пересобрать — `bash scripts/subset-fonts.sh` (нужен `uv`). `OFL.txt` обязан оставаться рядом (требование лицензии). Руками `.ttf` не править.

## Работа через субагентов

Работу выполняют субагенты, основной агент — оркестратор: разведка,
декомпозиция, постановка задач, приёмка результата, диалог с пользователем.
Правки кода, тестов, документации и роадмапа идут через субагента, а не делаются
в основном контексте. Как ставить задачу, что исполнитель возвращает и когда
остановиться, — `docs/agent-contract.md`.

**Почему:** независимые задачи выполняются параллельно, а основной контекст не
забивается содержимым файлов, которое после правки уже не нужно.

Исключение — правка в одну-две строки внутри задачи, которая и так ведётся
основным агентом: постановка такой задачи дороже выполнения. Если строк
больше или правок несколько — это уже задача для субагента.

Задачи делятся мелко: один субагент — один слайс (парсер, маппинг, тесты,
краевые случаи), а не «сделай фичу целиком». Контекст субагента ограничен, и
чтение больших файлов съедает его быстрее всего: несколько агентов на правках
`paint.rs` и `display_list.rs` упали с «Prompt is too long», не доделав работу.
В постановке прямо указывай, что читать `grep`-ом и участками, а не файл целиком,
и какие файлы трогать нельзя — чтобы параллельные слайсы не конфликтовали.

Дробление рекурсивно: если слайс всё равно велик, субагент дробит его сам и
раздаёт своим субагентам, а не тянет до переполнения контекста. Это разрешено
и ожидается — в постановке стоит снимать сомнения на этот счёт, иначе агент
считает, что обязан сделать всё сам.

Коммит — сразу после того, как проверки позеленели, до дальнейших изысканий.
Агенты, уходившие в «проверю ещё одно перед коммитом», падали с переполнением
контекста и оставляли готовую работу незакоммиченной; та же осторожность в
постановке («сначала коммит, потом разбирательства») стоит дешевле.

## Соглашения

- Комментарии и доккомментарии — по-русски; сообщения об ошибках (`#[error(...)]`) — по-английски, это часть публичного API. Секции `# Errors` у публичных fallible-функций обязательны (clippy pedantic).
- Комментарии — только необходимые: тот, что объясняет **почему** (неочевидное решение, ссылка на спецификацию, ловушка чужого формата, цена ошибки), а не пересказ кода.
- `#![forbid(unsafe_code)]` + `#![deny(clippy::pedantic)]` стоят в `core`, `docx`, `xlsx`, `pdf`; у `render` их нет (факт, а не разрешение). `unsafe` вне `crates/wasm` не добавляется — вместо этого остановиться и спросить (`docs/agent-contract.md` §5).
- TS: `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`; импорты с явным `.js`; JSDoc на экспортируемых функциях; без `any`.
- Conventional Commits, trunk-based (работа в `main`); feature-flags для незавершённого; 2 approver'а на изменения публичного API.
- Соавторов в коммитах нет: `Co-Authored-By` и любые другие подписи не добавляются.
- Документация: `ROADMAP.md` ведётся как живой документ — устаревшие числа и статусы (включая записи о спринтах и снимки) переписываются до текущего факта, обновлённое число подтверждается командой. Конвенция изменена 15.10.2026 (прежнее «роадмап задним числом не переписываем» отменено).
- Новые WASM-функции покрываются `wasm-bindgen-test`; тяжёлые вычисления — только в воркере; `postMessage` с учётом transferables.
- Покрытие — гейты CI: `doc-converter-docx` ≥ 85 %, `doc-converter-core` ≥ 80 % строк; TS-порог (≥ 80 %) в CI не проверяется. Числовые бюджеты — ROADMAP §9; тесты-бюджеты (`crates/pdf/tests/budgets.rs`, `crates/xlsx/tests/budget.rs`) не ослаблять без вопроса.

## CI и зависимости

- Тулчейн закреплён: `rust-toolchain.toml` — 1.98.0 (совпадает с CI), workspace объявляет MSRV `rust-version = "1.82"`; `.cargo/config.toml` включает `incompatible-rust-versions = "fallback"`, чтобы резолвинг не вылезал за MSRV.
- `deny.toml` разрешает только Apache-2.0/MIT/BSD-2-Clause/BSD-3-Clause/ISC/Unicode-3.0/Zlib; `openssl` забанен, `wildcards = "deny"`. PDF-бэкенд собственный (ADR-0022); сторонние библиотеки в нём — только `lopdf` (модель объектов), `subsetter`, `skrifa`, `image` — все из allow-списка. Новая зависимость вне allowlist — ADR и вопрос пользователю.
- Версии держатся не случайно: `quick-xml = 0.41` и `lru = 0.18` закрывают RUSTSEC-адвизори; `ttf-parser` намеренно не взят (unmaintained) — метрики на `skrifa` (ADR-0002). Понижение или замена — только с разбором advisories.
- Гейты CI: fmt, clippy `-D warnings`, `cargo doc` `-D warnings`, тесты workspace; браузерные `wasm-pack test` трёх модулей; JS на Node 20 и 22 + `pnpm size-limit` (только Node 20, бюджеты `.size-limit.mjs`); бюджеты памяти PDF/DOCX — жёстко, времени — мягко; покрытие; `cargo deny` + `cargo audit` — жёстко, `pnpm audit` — информационно (`|| true` в CI). Оракул LibreOffice и fuzz — отдельные workflow (по расписанию и по путям, с nightly).
