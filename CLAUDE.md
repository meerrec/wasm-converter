# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Что это

Монорепозиторий **doc-converter**: просмотрщик OOXML (DOCX/XLSX) и экспортёр в PDF — Rust/WASM, рендер на `OffscreenCanvas` внутри Web Worker. Cargo workspace (`crates/*`) + pnpm workspaces (`packages/*`) + Turborepo. Разработка идёт фазами по `ROADMAP.md` (13 спринтов): сделаны Фаза 1 (workspace, CI, core-крейты, RPC Main↔Worker) и Фаза 2 (painter, DisplayList, SAB ring). Незаконченное помечено `TODO (Фаза N)` / `todo!("… (Фаза N)")` — при работе над фазой ищите эти маркеры, а не только ROADMAP.

## Команды

### Rust

```bash
cargo test --workspace                 # нативные тесты (crates/wasm нативно пуст, см. ниже)
cargo test -p doc-converter-render     # один крейт
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p doc-converter-render    # criterion, benches/render.rs
cargo deny check                       # deny.toml: лицензии, баны, advisories
wasm-pack test --headless --chrome crates/wasm   # wasm-bindgen-test (run_in_browser; нужен chromedriver)
```

`crates/wasm` целиком под `#![cfg(target_arch = "wasm32")]`, поэтому на нативном таргете он пуст — `cargo clippy --workspace --all-targets` и `cargo test --workspace` не требуют выбора таргета.

### Сборка WASM

```bash
pnpm build:wasm   # = bash scripts/build-wasm.sh
```

Скрипт гоняет `wasm-pack build crates/wasm --target web --out-dir packages/wasm/pkg --release` и затем системный `wasm-opt -O4`, если он есть в `PATH`. Встроенный в wasm-pack `wasm-opt` отключён (`crates/wasm/Cargo.toml`): rustc эмитит bulk-memory, который та версия не понимает.

### JS/TS

```bash
pnpm install
pnpm build:wasm                       # ОБЯЗАТЕЛЬНО до typecheck/test
pnpm turbo run typecheck test build   # то же, что гоняет CI
pnpm -F @doc-converter/core test      # vitest
pnpm -F @doc-converter/core exec vitest run test/tick.spec.ts        # один файл
pnpm -F @doc-converter/core exec vitest run -t 'coalesces repeated'  # один тест
pnpm size-limit                       # бюджеты из .size-limit.json
```

Порядок не косметический: `@doc-converter/core` типизируется против сгенерированных `packages/wasm/pkg/*.d.ts`, а `packages/wasm/pkg/` в `.gitignore`. На свежем клоне без `pnpm build:wasm` typecheck упадёт. В CI `pkg/` приезжает артефактом из джобы `wasm` (`download-artifact` → `packages/wasm/pkg`).

`pnpm lint` сейчас холостой: скрипт `lint: eslint src` объявлен в `packages/core/package.json`, но eslint не установлен ни в зависимостях, ни в локе.

### Turborepo

Установлена версия 2.11.5 (`package.json` объявляет `^2.3.3`). Перед правкой `turbo.json` или turbo-команд читайте бандленные доки установленного пакета: `node_modules/.pnpm/turbo@2.11.5/node_modules/turbo/docs/` (так требует `AGENTS.md`). Граф задач: `build` → `^build`; `test` → `build`; `typecheck` → `^build`; `clean` без кэша. `packages/wasm/turbo.json` переопределяет `build.outputs: []` — `pkg/` не кэшируется, его собирает скрипт.

Блок `<!-- BEGIN:turborepo-agent-rules -->` в `AGENTS.md` пишет и переписывает сам turbo; не удаляйте его из коммитов.

## Архитектура

### Крейты

| Крейт | Роль |
|---|---|
| `crates/core` | OOXML-примитивы: `Archive` (read-only обёртка над `zip`), `XmlReader` (streaming `quick-xml`, пропускает Decl/Comment/PI/DocType), rels, `Error` (thiserror + miette `Diagnostic`). Ничего не знает про DOCX/XLSX |
| `crates/render` | Вся реальная логика рендера: `DisplayList`, `Painter2D`, `SabRing`, `RingState` |
| `crates/xlsx` | Разбор книги целиком (`open()`), раскладка листа в пикселях и сборка `DisplayList` (`paint::build`) |
| `crates/docx` | Пока заглушка: открывает архив, зовёт `validate_ooxml`, возвращает непрозрачный тип. Парсинг — TODO |
| `crates/pdf` | Заглушка (`todo!()`); `options.rs` с `PdfOptions` уже есть |
| `crates/wasm` | Только wasm-bindgen-экспорты, тонкий слой над render/display_list |

### Контракт Rust ↔ TS: DisplayList

`crates/render/src/display_list.rs` — бинарный формат кадра, общий для Rust и будущих TS-потребителей: магия `"DLST"`, `DL_VERSION = 4`, заголовок 20 байт (`HEADER_SIZE`), команды с однобайтовыми тегами, строки — в общем string pool, на который ссылаются `StringRef { off, len }`. Формат объявлен фиксированным: painter читает его прямо из SAB. Меняя раскладку, синхронизируйте версию/теги и TS-сторону.

Команда `Text` несёт гарнитуру строкой в общем пуле и флаги начертания `bold`/`italic`/`underline`, у `Line` рисунок штриха — `LineStyle` (толщина остаётся в `stroke_w`): painter читает список из SAB и не может спросить у вызывающего, что за шрифт номер три, — формат обязан быть самодостаточным. Цвет в DisplayList — `RRGGBBAA`; в OOXML он записан как `AARRGGBB`, поэтому `xlsx::paint::resolve_color` каналы переставляет.

Painter (`crates/render/src/painter/painter_2d.rs`, только wasm32) живёт в `thread_local!` (`crates/wasm/src/painter_api.rs`), инициализируется один раз, кэширует состояние canvas-контекста (`painter/state.rs`) и `ImageBitmap` (`painter/bitmap_cache.rs`). Два входа: `paint_display_list_sab` (zero-copy из ring, освобождает слот) и `paint_display_list_bytes` (fallback с копированием). `resize_canvas` пересчитывает размеры под DPR и сбрасывает состояние painter'а.

### SAB ring

Два слота, layout продублирован в двух местах и должен совпадать:

- `crates/render/src/sab/ring.rs` — `SabRing` (Atomics через `js-sys`) для wasm и чистый `RingState` для нативных тестов;
- `packages/core/src/sab/protocol.ts` + `reader.ts` — `HEADER_INTS = 4`, `SLOT_COUNT = 2` (`seq_writer`, `seq_reader`, `slot_len[0..2]`, дальше два слота).

Backpressure — нет свободного слота → кадр дропается (`dropped: true` в статистике).

### Просмотр XLSX

`createXlsxViewer` (`packages/core/src/render/xlsx_viewer.ts`) собирает разметку
сам: прокручиваемая область, распорка по размеру листа и холст, приклеенный к
видимой части трансформом. Единицы в запросе кадра разные и это важно:
`viewport.x/y` — в пикселях раскладки (до зума), `w/h` — в физических пикселях
canvas, `scale` — зум, умноженный на DPR.

Книга живёт в `thread_local!` воркера (`crates/wasm/src/xlsx_api.rs`), кадр
собирается сразу в слот ring'а: гонять сотни килобайт команд через
`postMessage` значило бы платить за копирование на каждом кадре.

Пример — `examples/viewer-xlsx/` (Vite + Playwright; сквозные тесты идут в
настоящем Chromium и проверяют пиксели через `exportPng`).

### Два канала Main ↔ Worker (намеренно разные)

1. **RPC-конверт** — `packages/core/src/protocol.ts` (`WorkerRequest`/`WorkerResponse`) и `rpc.ts` (id, `call`/`notify`/`on`/`dispose`). Источник истины — TS-файл; Rust описывает только payload.
2. **Прямой протокол кадров** — `InitMsg`/`ResizeMsg`/`BitmapMsg`/`RenderMsg`/`ExportPngMsg` → `ReadyMsg`/`TickMsg`/`PngMsg`. Без request/response, чтобы кадр не ждал ответа: воркер на `render` не отвечает, а шлёт `tick` со статистикой (`PaintStats`).

Поток: main вызывает `canvas.transferControlToOffscreen()` и **один раз** отдаёт canvas воркеру (`render/offscreen.ts:initOffscreen`) → воркер инициализирует wasm, делает `alloc_sab`, отвечает `ready` с SAB → `ResizeObserver` (debounce 100 мс, DPR клампится по лимиту 16M пикселей в `render/resize_observer.ts:computeDpr`) шлёт `resize` + `render` → воркер собирает DisplayList в слот ring → painter рисует → `tick`. Кадры коалесцируются через `requestAnimationFrame` в `worker/frame_loop.ts`; ошибки кадра логируются, loop не роняется.

SAB требует COOP/COEP (`Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`); без них — fallback на `ArrayBuffer` + transferables (`docs/workers.md`).

## Работа через субагентов

Работу выполняют субагенты, основной агент — оркестратор: разведка,
декомпозиция, постановка задач, приёмка результата, диалог с пользователем.
Правки кода, тестов, документации и роадмапа идут через `Agent`, а не делаются
в основном контексте.

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

- Комментарии и доккомментарии — по-русски; сообщения об ошибках (`#[error(...)]`) — по-английски, это часть публичного API библиотеки. Секции `# Errors` у публичных fallible-функций обязательны (на них настроен clippy pedantic).
- Комментарии — только необходимые. Нужен тот, что объясняет **почему**: неочевидное решение, ссылка на спецификацию, ловушка в чужом формате, цена ошибки. Лишний тот, что пересказывает код («увеличиваем счётчик»), называет очевидный блок или поясняет имя, которое говорит само за себя, — такой вытесняется кодом, а не соседствует с ним.
- `#![forbid(unsafe_code)]` + `#![deny(clippy::pedantic)]` во всех крейтах, кроме `crates/wasm`.
- TS: `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`; импорты с явным `.js`; JSDoc на экспортируемых функциях; без `any`.
- Conventional Commits, trunk-based (работа в `main`); feature-flags для незавершённого; 2 approver'а на изменения публичного API.
- Соавторов в коммитах нет: трейлер `Co-Authored-By` и любые другие подписи не добавляются.
- Новые WASM-функции покрываются `wasm-bindgen-test`; тяжёлые вычисления — только в воркере; `postMessage` с учётом transferables.
- Покрытие: Rust ≥ 85%, TS ≥ 80%; бюджеты размера и производительности — `ROADMAP.md` §9.

## CI и зависимости

Тулчейн закреплён: `rust-toolchain.toml` — 1.98.0 (совпадает с CI), при этом workspace объявляет MSRV `rust-version = "1.82"`. `.cargo/config.toml` включает `incompatible-rust-versions = "fallback"`, чтобы резолвинг зависимостей не вылезал за MSRV. `deny.toml` разрешает только Apache-2.0/MIT/BSD/ISC/Unicode-3.0/Zlib — из-за этого `printpdf` подключён с `default-features = false` (фича `html` тянет MPL-2.0 и resvg).
