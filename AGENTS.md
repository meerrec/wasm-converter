<!-- BEGIN:turborepo-agent-rules -->

# This is NOT the Turborepo you know

Turborepo configuration, task behavior, and CLI commands can vary between installed versions and may differ from your training data. Resolve the `turbo` package from this file's directory or relevant workspace; in monorepos, it may not be visible from the repository root. For example, run `node -p "require.resolve('turbo/package.json')"` from a workspace that depends on `turbo`.

Read `docs/README.md` inside that installed package first, then read the relevant pages from its `docs/` directory before changing Turborepo configuration or commands. Heed deprecation notices. These bundled docs match the installed package version and are available without network access.

This block is written and re-added by `turbo` before repository-scoped commands when an AI agent is detected. In the Turborepo source repository, its template is defined in `crates/turborepo-cli/src/cli/agent_guidance.rs`. Removing the managed block while updates are enabled means a later qualifying invocation will add it again. Set `"agentGuidance": false` in the root `turbo.json` or `turbo.jsonc` to opt out; this does not remove an existing block. Keep the block committed with your work to avoid an uncommitted change on the next agent invocation.
<!-- END:turborepo-agent-rules -->

# Repository Guidelines

Монорепозиторий **doc-converter**: просмотр OOXML (DOCX/XLSX) и конвертация в PDF/Markdown/HTML/текст. Движок — Rust/WASM, кадр собирается и рисуется в Web Worker на `OffscreenCanvas`; batch-конвертация — Node-CLI на тех же wasm-модулях. Этот файл — единственный источник правды для агентов; `CLAUDE.md` указывает сюда.

## Обзор проекта

- **Что делает:** просмотрщики XLSX и DOCX в браузере (разбор, раскладка и отрисовка целиком в воркере), экспорт PDF/PNG/Markdown/HTML/текст, CLI `@meerrec/cli` c командами `convert` и `batch`, демо на GitHub Pages. Браузерные конвертеры (страницы «Конвертер» на x2t и «Euro-Office» на движке Euro-Office) вынесены в отдельный репозиторий [github.com/meerrec/eurooffice-demo](https://github.com/meerrec/eurooffice-demo) (ADR-0038): монорепозиторий — чистый Apache-2.0.
- **Скоуп v1 — только чтение и конвертация** (ADR-0018, ADR-0020). Strict OOXML — поддерживается для DOCX и XLSX наравне с Transitional ([ADR-0034](docs/adr/0034-strict-ooxml.md)); PPTX, ODF (`.odt`/`.ods`/`.odp`) и `.docm` — в плане ([ADR-0039](docs/adr/0039-cancel-v1-non-goals.md)); XLSB, шифрование и бинарный `.doc` — статус non-goal снят, реализаций нет. Осознанно нет: VML-фолбэков `mc:Fallback` (ADR-0014), обтекания текстом вокруг плавающих рисунков; сноски и комментарии парсятся в модели, но в раскладку не попадают.
- **Состояние:** версии крейтов и пяти npm-пакетов — `1.0.0`; npm-пятёрка `@meerrec/*` опубликована в **GitHub Packages** этого репозитория (тег `v1.0.0`), npmjs.org — целевой основной реестр (публикация готовится, [ADR-0027](docs/adr/0027-npmjs-primary-registry.md)); Rust-крейты отдаются ассетами релиза, публикация в crates.io — осознанный non-goal v1.x ([ADR-0026](docs/adr/0026-rust-crates-distribution.md)). Спринты 3–13 закрыты; Спринт 13 (спайк LOWA) завершён решением no-go (ADR-0023, ADR-0024). Фаза 1 архитектурного аудита реализована: npmjs-подготовка (ADR-0027), wire-типы через ts-rs (ADR-0028), main-thread fallback (ADR-0029).
- Раздел «Статус» корневого `README.md` **устарел — не ориентир**; фактическое состояние — `ROADMAP.md` (§7 сводка, §9 бюджеты, §12 DoD).

## Архитектура и поток данных

- **Слои Rust:** `crates/core` (примитивы OOXML: `Archive` над zip, стриминговый `XmlReader`, rels, `Error` на thiserror+miette) → `crates/xlsx` / `crates/docx` (разбор в модель → раскладка в пикселях → сборка DisplayList) → `crates/render` (формат DisplayList, painter, SAB-кольцо, геометрия, метрики текста) → `crates/pdf` (собственный PDF-бэкенд, ADR-0022). Правило границы: `core` не знает про DOCX/XLSX, `render` — общая граница обоих форматов (ADR-0003).
- **wasm-мосты:** `crates/wasm` — основной модуль (painter, SAB-аллокатор, XLSX-просмотрщик), `crates/pdf-wasm` — ленивый экспорт PDF, `crates/docx-wasm` — ленивый просмотр DOCX. Все три целиком под `#![cfg(target_arch = "wasm32")]`: на нативном таргете они пусты, поэтому нативные `cargo test`/`clippy` и `cargo doc` их не видят — их доккомментарии и сборку проверяет wasm32-нога CI.
- **TS-слой:** `@meerrec/core` — воркер просмотра и фабрики `createXlsxViewer`/`createDocxViewer`; канал один и прямой — сообщения `InMsg`/`OutMsg`, источник истины `packages/core/src/protocol.ts` (Rust описывает только payload). `@meerrec/cli` гоняет те же wasm-модули в пуле `node:worker_threads`.
- **Поток кадра:** main отдаёт `OffscreenCanvas` воркеру → воркер инициализирует wasm и отвечает `ready` с транспортом → `resize` + `render` → воркер собирает DisplayList в слот ring → painter рисует → `tick` со статистикой. Без `transferControlToOffscreen` canvas не уезжает в воркер, и включается main-режим: воркер собирает DisplayList в два `ArrayBuffer` и шлёт `frame` на main, main рисует тем же Rust-painter'ом (`init_painter_main`), коалесцирует по rAF и возвращает буфер `frame-ack`; `tick` тогда эмитит main, а painter-main — singleton («один вьюер на страницу»). На `render` ответа нет; по `id` коррелируются `open` (ответ `opened`/`docx-opened`), `hit-test`, `hyperlink-at` и `export-*`; у ошибки есть `id` и `code`, ошибка без `id` считается глобальной. Кадры коалесцируются в `packages/core/src/worker/frame_loop.ts`; ошибка кадра логируется, цикл не роняется.
- **XLSX:** книга живёт в `thread_local!` воркера (`crates/wasm/src/xlsx_api.rs`), кадр собирается сразу в слот ring (`crates/xlsx/src/paint.rs`). **DOCX:** раскладка считается один раз при открытии (`crates/docx/src/layout/engine.rs`), кадры рисуют стопку страниц (`crates/docx/src/render.rs`); ленивый модуль painter'а и SAB не несёт — рисует основной модуль.
- **DisplayList — бинарный контракт Rust ↔ TS:** магия `"DLST"`, `DL_VERSION = 4`, заголовок 20 байт, однобайтовые теги команд, строки в общем string pool (`StringRef`), цвет `RRGGBBAA` (в OOXML — `AARRGGBB`, каналы переставляет `xlsx::paint::resolve_color`). TS кадр не разбирает, painter читает его прямо из SAB, поэтому формат обязан быть самодостаточным. Меняя раскладку, синхронно поднимайте версию, теги и потребителей.
- **Транспорты кадра (ADR-0021):** `sab` — кольцо из двух слотов в `SharedArrayBuffer` (нужны COOP/COEP), `bytes` — два `ArrayBuffer` с transfer. Выбор автоматический по `crossOriginIsolated`; явный `transport: 'sab'` без заголовков — ошибка инициализации, а не тихая подмена. Раскладка ring продублирована и должна совпадать: `crates/render/src/sab/ring.rs` ↔ `packages/core/src/sab/protocol.ts` (`HEADER_INTS = 4`, `SLOT_COUNT = 2`). Нет свободного слота — кадр дропается (`dropped: true`).

## Ключевые каталоги

| Путь | Содержимое |
|---|---|
| `crates/core`, `crates/render`, `crates/xlsx`, `crates/docx`, `crates/pdf` | Ядро: OOXML-примитивы, рендер и DisplayList, разбор и раскладка XLSX/DOCX, PDF-экспорт |
| `crates/wasm`, `crates/docx-wasm`, `crates/pdf-wasm` | wasm-bindgen-модули: основной, ленивый DOCX, ленивый PDF |
| `crates/fuzz` | Отдельный workspace (nightly, свой `target/`): 8 таргетов, запуск `cargo fuzz run --fuzz-dir . <target>` |
| `packages/core` | `@meerrec/core`: протокол, воркер, транспорты кадра, просмотрщики |
| `packages/cli` | `@meerrec/cli`: `convert`/`batch`, пул воркеров |
| `packages/wasm`, `packages/wasm-pdf`, `packages/wasm-docx` | Сгенерированные wasm-bindgen-биндинги; `pkg/` в `.gitignore`, руками не править |
| `examples/viewer` | Демо (Vite + React) и Playwright-e2e; страницы `#/docx`, `#/xlsx` (конвертер и Euro-Office — в eurooffice-demo) |
| `test-fixtures/` | `docx/` — 104 фикстуры с json-сайдкарами, `xlsx/` — 116 книг и `oracle.json` (эталон exceljs), `corrupted/`, `conf-xlsx/` (git-ignored) |
| `scripts/` | Сборка wasm, генераторы фикстур, оракулы LibreOffice, memory-гейты, стайдинг Pages (`pages/sw.js` — корневой воркер заголовков изоляции демо) |
| `docs/` | `adr/` (34 ADR и индекс), `sprint-6…13/`, `cookbook/`, `testing/`, `workers.md`, `bench-js.md`, `agent-contract.md` |
| `experiments/` | Спайк Спринта 13 (`lowa-spike`); часть артефактов в Git LFS (`x2t-spike` перенесён в eurooffice-demo) |
| `target/`, `build/`, `.pages/`, `oracle-out/`, `oracle-out-docx/` | Выводы прогонов, git-ignored |

## Команды разработки

Rust (из корня; тулчейн 1.98.0 берётся из `rust-toolchain.toml`):

```bash
cargo test --workspace --all-features                  # CI гоняет именно так
cargo test -p doc-converter-docx --test differential   # один файл
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
cargo bench -p doc-converter-pdf --bench pdf           # criterion; базлайнов нет, «до/после» — локальный прогон
```

Сборка и тесты WASM:

```bash
pnpm build:wasm   # wasm-pack --target web --release ×3 → packages/{wasm,wasm-pdf,wasm-docx}/pkg, затем системный wasm-opt -O4, если он в PATH
wasm-pack test --headless --chrome --release crates/wasm      # так же crates/docx-wasm и crates/pdf-wasm
```

`wasm-pack test` идёт в настоящий Chrome: положите `chromedriver` в `PATH` — иначе wasm-pack скачает «самый свежий», который может разойтись с браузером.

JS/TS:

```bash
pnpm install
pnpm build:wasm                        # ОБЯЗАТЕЛЬНО до typecheck/test: core типизируется против сгенерированных pkg/*.d.ts
pnpm gen:wire                          # перегенерировать wire-типы Rust → TS (packages/core/src/generated/wire.ts)
pnpm turbo run typecheck test build    # то же, что гоняет CI
pnpm -F @meerrec/core test                                     # vitest
pnpm -F @meerrec/core exec vitest run test/tick.spec.ts        # один файл
pnpm -F @meerrec/core exec vitest run -t 'coalesces repeated'  # один тест
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx --project=chromium   # один e2e
pnpm -F @meerrec/example-viewer test   # весь Playwright: chromium/firefox/webkit + chromium-dpr2
pnpm size-limit                        # после build; бюджеты — .size-limit.mjs
pnpm -F @meerrec/cli build && pnpm cli convert report.docx -o report.pdf
```

E2E сам поднимает dev-сервер демо на порту 5178; браузеры ставятся один раз: `pnpm -F @meerrec/example-viewer exec playwright install chromium firefox webkit`. `pnpm lint` не работает (eslint не установлен) — как проверку не использовать.

Фикстуры, оракулы, шрифты:

```bash
pnpm gen:fixtures              # XLSX-фикстуры и эталон exceljs; тяжёлые книги — в target/fixtures
pnpm gen:docx-fixtures         # DOCX-фикстуры; :large — в target/fixtures/docx-large
pnpm diff:mammoth              # differential DOCX против mammoth (оракул test-fixtures/docx/mammoth-oracle.json)
pnpm bench:js                  # против mammoth/exceljs/SheetJS, отчёт — docs/bench-js.md
bash scripts/oracle-docx-pages.sh <docx_export> test-fixtures/docx <workdir>   # LibreOffice-оракул; для XLSX — scripts/oracle-ssim.sh
bash scripts/subset-fonts.sh   # пересборка подрезанных Carlito (нужен uv)
```

Оракулы требуют системных `soffice`, `pdftoppm`, `ffmpeg` и ImageMagick; без них скрипты выходят с кодом 3 — это не провал тестов.

## Конвенции кода и типовые паттерны

- **Язык:** комментарии и доккомментарии — по-русски; сообщения ошибок `#[error(...)]` — по-английски (это публичный API). У публичных fallible-функций обязательна секция `# Errors` (clippy pedantic). Комментарий объясняет **почему** (неочевидное решение, ловушка формата, цена ошибки), а не пересказывает код.
- **Ошибки:** `thiserror` 2 + `miette` 7; в каждом крейте свой `Error` и `pub type Result<T>`, вложенность — `#[error(transparent)]` / `#[diagnostic(transparent)]`; `doc-converter-docx::Error` помечен `#[non_exhaustive]`.
- **Линты:** `#![forbid(unsafe_code)]` + `#![deny(clippy::pedantic)]` в `core`, `xlsx`, `docx`, `pdf`; в `crates/render` их нет; wasm-крейты несут только `#![cfg(target_arch = "wasm32")]`. Узкие `allow` — с комментарием-обоснованием. `unsafe` вне `crates/wasm` не добавляется: остановиться и спросить.
- **Состояние:** в wasm — `thread_local!` + `RefCell` (буферы `DOC`/`FRAME`/`SCRATCH`, singleton painter, LRU-реестр шрифтов); в TS — фабрики-замыкания (`createXlsxViewer`/`createDocxViewer` возвращают handle), не классы.
- **TS:** `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`; относительные импорты с явным `.js`; `import type`; JSDoc на экспортируемых функциях; без `any`. Файлы — `snake_case.ts`, типы и классы — PascalCase, тесты — `*.spec.ts`.
- **Wire-формат:** поля camelCase — на Rust-стороне за это отвечает `serde(rename_all = "camelCase")`.
- **Единицы — в именах** (`_px`, `_pt`, `_mm`, twips). В запросе кадра `viewport.x/y` — пиксели раскладки, `w/h` — физические пиксели canvas, `scale` — зум, умноженный на DPR.
- **Rust-нейминг:** крейты `doc-converter-*`, модули и методы snake_case, варианты ошибок PascalCase, `#[must_use]` на чистых геттерах.
- **Архитектурные решения — только через ADR:** `docs/adr/` (индекс `docs/adr/README.md`); принятый ADR не правят — заводят новый.
- **Документация:** `ROADMAP.md` ведётся как живой документ — устаревшие числа и статусы (включая записи о спринтах и снимки) переписываются до текущего факта, обновлённое число подтверждается командой. Конвенция изменена 15.10.2026 (прежнее «роадмап задним числом не переписываем» отменено).
- **Тесты:** unit — inline `#[cfg(test)]` рядом с кодом, интеграционные — `crates/*/tests/*.rs` с общими помощниками в `tests/common/`; снапшоты — insta, свойства — proptest, wasm-функции — `wasm-bindgen-test`.

## Важные файлы

- `Cargo.toml` — workspace (8 крейтов, `crates/fuzz` исключён), MSRV 1.88, release-профиль `lto = "fat"`, `opt-level = "s"`, `panic = "abort"`; `rust-toolchain.toml` закрепляет 1.98.0.
- `packages/core/src/protocol.ts` — источник истины протокола Main ↔ Worker.
- `crates/render/src/display_list.rs` — бинарный формат кадра (`DL_VERSION`).
- `.size-limit.mjs` — бюджеты бандлов и wasm-модулей (жёсткий гейт CI) и заодно сторож ленивости PDF/DOCX.
- `.github/workflows/ci.yml` — канонический набор гейтов; `oracle.yml`, `fuzz.yml`, `release.yml`, `pages.yml` — отдельные контуры.
- `examples/viewer/playwright.config.ts` — порт 5178, проекты браузеров, каталог e2e.
- `ROADMAP.md` — статус, бюджеты (§9) и DoD (§12); `docs/agent-contract.md` — правила постановки задач субагентам и список стоп-сигналов.
- `CLAUDE.md` — заглушка, указывающая на этот файл.

## Требования к среде и инструментам

- **Node ≥ 20** (`engines`), **pnpm 9.12.0** (`packageManager`), **turbo 2.11.5** установлен при объявленном `^2.3.3`.
- **Turborepo:** перед правкой `turbo.json` или turbo-команд читать бандленные доки именно установленной версии — `node -p "require.resolve('turbo/package.json')"`, дальше `docs/README.md` внутри пакета. Граф задач: `build → ^build` (`outputs: dist/**`), `test → build`, `typecheck → ^build`, `clean` без кэша; `packages/wasm*/turbo.json` переопределяют `build.outputs: []` — `pkg/` не кэшируется. Управляемый блок turbo в начале этого файла машинный: удалять его из коммитов нельзя.
- **Rust:** 1.98.0 закреплён `rust-toolchain.toml` (совпадает с CI), MSRV — 1.88 (поднят из-за `zip 9`, RUSTSEC-2026-0369); `.cargo/config.toml` держит `incompatible-rust-versions = "fallback"`, чтобы резолвинг не вылезал за MSRV. `crates/fuzz` — свой workspace на nightly.
- **Системные инструменты по требованию шага:** `wasm-pack` — обязателен для `build:wasm`; `wasm-opt` — системный, шаг опционален (встроенный в wasm-pack отключён: rustc эмитит bulk-memory, который та версия не понимает); `chromedriver` — для браузерных wasm-тестов; `uv` — для `subset-fonts.sh`; `soffice`, `pdftoppm`, `pdfinfo`, `ffmpeg`, ImageMagick — для оракулов; `/usr/bin/time` — для memory-гейтов; `qpdf` и `pdftotext` — для части тестов PDF (без них они скипаются).
- **Генерируемое не править:** `packages/*/pkg/` (wasm-pack), `docs/api/` (TypeDoc), `packages/core/src/generated/` (wire-типы из Rust через `pnpm gen:wire`, ADR-0028).
- **Git LFS:** `experiments/lowa-spike/**` — после клона нужен `git lfs pull` (артефакты x2t уехали вместе с пакетом в [github.com/meerrec/eurooffice-demo](https://github.com/meerrec/eurooffice-demo), ADR-0038).

## Тестирование и контроль качества

- **Уровни:** inline `#[cfg(test)]` в крейтах; интеграционные `crates/*/tests/*.rs` (у `crates/core` своего `tests/` нет); снапшоты insta и свойства proptest; `wasm-bindgen-test` в трёх wasm-крейтах (`run_in_browser`, фикстуры через `include_bytes!`); дифференциальные тесты и оракулы; fuzz-таргеты в `crates/fuzz/fuzz_targets` (8 таргетов); criterion-бенчи в крейтах (базлайнов нет, CI их только компилирует).
- **Фикстуры:** генераторы детерминированы (свой ZIP-писатель, фиксированные таймстампы). Пропавшая фикстура — падение, а не скип (`crates/docx/tests/layout.rs`, `crates/pdf/tests/docx_budgets.rs`).
- **Дифференциальные гейты:** mammoth (`crates/docx/tests/differential.rs`: кандидатов ≥ 50, `non-goal` ≤ 8, доля совпадений ≥ 0.95; путь оракула переопределяется `DOCX_MAMMOTH_ORACLE`) и exceljs (`crates/xlsx/tests/fixtures.rs`: сверка с `oracle.json` в обе стороны).
- **Жёсткие гейты CI** (`ci.yml`): `cargo fmt --all -- --check`; clippy с `-D warnings`; `cargo doc` с `-D warnings` на обеих ногах; `cargo test --workspace --all-features`; сборка трёх wasm-крейтов под wasm32; сверка `.d.ts` с `#[wasm_bindgen]`-экспортами и запрет экспортов `docx_*` в основном модуле; `wasm-pack test --headless --chrome --release` для трёх крейтов; `pnpm turbo run typecheck test build` на Node 20 и 22; `pnpm size-limit` (Node 20); память — PDF ≤ 40 МиБ и DOCX ≤ 200 МиБ; `cargo deny check` и `cargo audit`; покрытие — `cargo llvm-cov -p doc-converter-core --fail-under-lines 80` и `-p doc-converter-docx --fail-under-lines 85`.
- **Мягкие и информационные:** время в бенчах (500 страниц PDF < 3000 мс, разбор 50 МиБ DOCX < 1500 мс), `pnpm audit`.
- **Бюджет-тесты не ослаблять без вопроса:** `crates/pdf/tests/budgets.rs` (200 КиБ на 1000 ячеек, 32 КиБ для сжатого плотного PDF, мягкие 900 мс и 9000 мс), `crates/xlsx/tests/budget.rs` (`size_of::<Cell>() == 48`, `#[ignore]`-тест на миллион ячеек), `crates/docx/tests/memory.rs` (модель: Paragraph ≤ 256 Б, Run ≤ 128 Б, Cell ≤ 192 Б).
- **Отдельные workflow:** `oracle.yml` (LibreOffice SSIM, cron и path-фильтр), `fuzz.yml` (nightly, cron и path-фильтр), `release.yml` (changesets, dry-run пакетов), `pages.yml` (демо).
- **Границы:** TS-порог покрытия (≥ 80 %) в CI не проверяется; e2e-тесты CLI скипаются, если `dist/cli.js` не собран.
