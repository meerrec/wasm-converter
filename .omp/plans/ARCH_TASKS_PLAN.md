# План: выполнение всех задач `docs/architecture.md`

## Context

`docs/architecture.md` — архитектурный аудит с роадмапом миграции (Фазы 1–4), матрицей As-Is/To-Be и критическими замечаниями. Просьба: «распланируй все задачи». Фаза 1 аудита уже реализована (ADR-0027/0028/0029); фаззинг из Фазы 4 тоже есть (8 таргетов, Спринт 8); fallback на `bytes`/main-thread (крит. замечание №3) закрыт ADR-0021/0029. План покрывает всё остальное из документа.

Решения, принятые пользователем: (1) «единое ядро» = trait-слой `FormatPlugin`/`DocumentModel`/`CanvasPainter` в Rust, **ленивые wasm-модули остаются** (бюджеты size-limit и гейт «нет `docx_*` в viewer» не трогаем), **wasm-роутера нет** — диспетчеризация форматов на TS-стороне по явному `DocFormat`, каждый формат в своём модуле, границы под будущие отдельные воркеры на формат и воркер-пул (направление в ADR-0030, задачами не покрывается); (2) Euro-Office выносится **жёстко в отдельный репозиторий**; (3) включены **ODF** и **OpenTelemetry**; (4) отдельным решением «отменяем non goal» включаются и остальные матричные строки — **CDN-дистрибуция**, **native COOP/COEP-хостинг**, **MS Office SSIM-оракул** — и **отменяются non-goal'ы v1**, противоречащие To-Be: PPTX и XLSB (`ROADMAP.md` §2), Strict OOXML (ADR-0017), `.docm`, зашифрованные и бинарные `.doc` (`docs/architecture.md` §1.9). Отмена оформляется новым ADR и правкой живых документов (задача T0.2). Реализация планируется там, где To-Be её требует (PPTX, Strict, ODF/ODP, `.docm` — задачи Эпика 3); для XLSB, шифрования и бинарного `.doc` статус non-goal снимается, но реализации в этом плане нет — To-Be документа их не содержит. Редактирование/коллаборация/ИИ/server-side/real-time (`ROADMAP.md` §2) документом архитектуры не затрагиваются и остаются в силе.

Общие правила для всех задач: публичный API меняется с двумя approver'ами; бюджеты из `ROADMAP.md` §9 и тесты-бюджеты (`crates/pdf/tests/budgets.rs`, `crates/xlsx/tests/budget.rs`, `crates/docx/tests/memory.rs`, строки `.size-limit.mjs`) не ослабляются без вопроса пользователю; формат DisplayList и `DL_VERSION` остаются нетронутыми во всех задачах (painter читает кадр прямо из SAB); новые wasm-функции покрываются `wasm-bindgen-test`; комментарии по-русски, сообщения ошибок по-английски.

Порядок эпиков: 0 — закрытие Фазы 1 (npmjs) и отмена non-goal'ов; 1 — Фаза 2 документа (trait-слой → потоковый ZIP → отмена операций); 2 — вынос AGPL (независим, освобождает демо для обёрток); 3 — Фаза 3 документа (PPTX → Strict → ODF → `.docm` → WebGL); 4 — Фаза 4 (обёртки → CLI Deno/Bun → OTel → сверка fuzz → CDN → native COOP/COEP → MS Office SSIM). Эпики 2 и 3 независимы друг от друга; внутри эпика задачи идут последовательно (потоковый ZIP переиспользует trait-слой; PPTX/Strict/ODF — плагины поверх trait-слоя).

---

## Эпик 0. Завершение Фазы 1: первый релиз в npmjs.org

### T0.1. Публикация v1.1.0 в npmjs.org и переключение документации

Размер M. Зависимостей нет.

В репо уже готово всё автоматизируемое: `.github/workflows/release.yml` (джоба `publish`: preflight `npm whoami` до `changeset version`, публикация npmjs первой, GitHub Packages второй, `PUBLISH_CRATES_IO` отдельным выключателем), `scripts/check-package-metadata.mjs`, pack-preflight в dry-run. Осталось внешнее и документационное.

Шаги:
1. **Внешние предпосылки (вручную, вне репо):** зарегистрировать организацию `@meerrec` на npmjs.org; выпустить granular/automation token с правом publish в неё и обходом 2FA; положить в секрет репозитория `NPM_TOKEN`; задать переменную репозитория `PUBLISH_TO_NPMJS=true` (ADR-0027 §6).
2. **Релиз:** в `.changeset/` лежит нерелизнутый `phase-1-npmjs-wire-fallback.md` (minor для `@meerrec/core` и `@meerrec/wasm`). Выполнить `pnpm changeset version` (поднимет эти два пакета до 1.1.0, очистит changeset), свести версию крейта `doc-converter-core` в `Cargo.toml` к 1.1.0 (джоба `publish` сверяет тег с ним), смержить в main, поставить тег `v1.1.0`. Джоба `publish` выложит в npmjs.org: `core@1.1.0`, `wasm@1.1.0`, `cli@1.0.0`, `wasm-docx@1.0.0`, `wasm-pdf@1.0.0` (последние три — первично, их 1.0.0 в npmjs ещё нет) и обновит зеркало GitHub Packages.
3. **Проверка публикации:** `npm view @meerrec/core version` == `1.1.0`, то же для остальных четырёх; `npm pack @meerrec/cli --dry-run` показывает `dist/`, `LICENSE`, `README.md`.
4. **Документация переводится на npmjs как основной путь** (ADR-0027 §7 это явно разрешает): в `README.md` (корень), `AGENTS.md` (абзац «Состояние») и `ROADMAP.md` (строка M12 в §8, строки релиза в §7 и §12, формулировка Фазы 1 аудита в §6) заменить «npmjs.org — целевой основной реестр (публикация готовится)» на фактическую: установка без токена `npm install @meerrec/core`, GitHub Packages — зеркало.
5. **OIDC/trusted publishing** (ADR-0027 §8): настроить на npmjs.org доверенного издателя для репозитория (вручную, вне репо); в `release.yml` убрать `NPM_TOKEN` из preflight/publish-шагов, добавить в `publish` `permissions.id-token: write`; секрет `NPM_TOKEN` удалить из репозитория после первого успешного OIDC-релиза. Если настроить trusted publisher сейчас нельзя — оставить токенную схему и завести issue (шаг откладывается, не блокирует).

Верификация: зелёная джоба `publish` на теге `v1.1.0`; `npm view` по всем пяти пакетам; `node scripts/check-package-metadata.mjs` выходит 0; в `AGENTS.md`/`ROADMAP.md`/`README.md` нет строки «публикация готовится».

### T0.2. ADR-0039: отмена non-goal'ов v1

Размер S. Независима; выполняется первой в Эпике 0, чтобы остальные задачи не спотыкались о non-goal-формулировки в документах.

Шаги:
1. **ADR `docs/adr/0039-cancel-v1-non-goals.md`:** фиксирует отмену non-goal-статусов, противоречащих To-Be `docs/architecture.md`: PPTX и XLSB (`ROADMAP.md` §2), Strict OOXML (пересмотр ADR-0017 подтверждается — ссылка на ADR-0034), `.docm`, зашифрованные документы, бинарный `.doc` (§1.9 документа). В ADR явно записано: реализация планируется только для PPTX, Strict, ODF/ODP, `.docm` (задачи Эпика 3); XLSB, шифрование и бинарный `.doc` — статус снят, но задач нет (To-Be их не содержит), они остаются «не реализовано», а не «non-goal». Редактирование, коллаборация, ИИ-предложения, server-side, real-time (`ROADMAP.md` §2) документом не отменяются и остаются в силе без изменений.
2. **Правки живых документов:** `ROADMAP.md` §2 — убрать «PPTX, XLSB» из «❌ Не делаем», заменить на ссылку «статусы форматов — ADR-0039»; `AGENTS.md` раздел «Скоуп v1» — заменить «Осознанно нет: PPTX и XLSB, Strict OOXML (только Transitional, ADR-0017)» на «PPTX/Strict/ODF/.docm — в плане (ADR-0039); XLSB/шифрование/бинарный .doc — статус non-goal снят, реализации нет»; корневой `README.md` — та же правка в разделе non-goal'ов. Правки T0.1 (реестры) и T0.2 (скоуп) не пересекаются: T0.1 трогает только формулировки реестров, T0.2 — только таблицы скоупа.
3. **Индексация:** добавить ADR-0039 в `docs/adr/README.md` (таблица индекса); в `ROADMAP.md` §11 таблицу «Принятые ADR» — строку про отмену non-goal'ов.

Верификация: в `AGENTS.md`/`README.md`/`ROADMAP.md` нет формулировок «PPTX — non-goal», «Strict — non-goal», «XLSB — не делаем»; `docs/adr/README.md` содержит ADR-0039; CI не затронут (документная задача), `git diff --stat` — только документы (новый `docs/adr/0039-cancel-v1-non-goals.md`, `docs/adr/README.md`, `ROADMAP.md`, `AGENTS.md`, `README.md`).

---

## Эпик 1. Фаза 2: trait-ядро, потоковый ZIP, отмена операций

### T1.1. ADR-0030: trait-слой форматов (`FormatPlugin`/`DocumentModel`/`CanvasPainter`)

Размер L. Зависит от T0.1 только формально (можно параллельно).

Цель — общий контракт форматов (PPTX/ODF/Strict) без нового клея, при сохранении DisplayList как единственного IR и ленивых модулей. **Единого wasm-роутера нет**: диспетчеризация форматов — на TS-стороне по явному `DocFormat` из протокола, каждый формат живёт в своём ленивом wasm-модуле и сам вызывает свой плагин из собственных экспортов. Модульные границы закладываются под будущий вынос формата в отдельный воркер и воркер-пул: общее per-формат состояние в основном модуле не появляется, а switch по `DocFormat` в воркере — единственная точка, которую такой вынос трогает (направление фиксируется в ADR-0030, задачами этого плана не покрывается).

Шаги:
1. **ADR `docs/adr/0030-format-plugin-traits.md`:** фиксирует: (а) трейты живут в `crates/render` (общая граница форматов, ADR-0003); (б) DisplayList остаётся единственным контрактом кадра — sketch-трейт `RenderCommand` (`Vec<Box<dyn RenderCommand>>`) из `docs/architecture.md` §3.2 **не вводится**: вторая командная пайплайн-иерархия была бы мёртвым дублёром бинарного IR, который painter читает из SAB; вместо него `DocumentModel::build_display_list` возвращает байты DisplayList, как сейчас делают `xlsx::paint` и `docx::render`; (в) `async_trait` не используется — wasm-разбор синхронный, потоковый вход (T1.2) буферизуется на TS-стороне до вызова `parse`; (г) ленивые модули остаются плагинами физически, trait-слой — их общий контракт; дублирование std между модулями признаётся осознанной ценой ленивости; (д) **wasm-роутера нет**: формат задаёт вызывающий явно (`OpenMsg.format`), магические байты не нюхаются (`can_parse` не существует), `registry.rs`/`open_any` не создаются; (е) **направление «отдельные воркеры на формат + воркер-пул»**: границы модулей позволяют в будущем вынести формат в свой воркер без правок Rust — весь per-формат код уже в своём wasm-модуле, в основном модуле остаются только painter/SAB/шрифты; switch по `DocFormat` в `packages/core/src/worker/worker.ts` — единственное место диспетчеризации.
2. **Новый `crates/render/src/plugin.rs`:**
   ```rust
   pub struct RenderContext { pub scale: f32, pub viewport_width: u32, pub viewport_height: u32 }
   pub trait FormatPlugin: Send + Sync {
       fn id(&self) -> &'static str;
       fn parse(&self, bytes: &[u8], limits: &ZipLimits)
           -> Result<Box<dyn DocumentModel>, EngineError>;
   }
   pub trait DocumentModel: Send + Sync {
       fn page_count(&self) -> u32;
       fn build_display_list(&self, ctx: &RenderContext) -> Result<Vec<u8>, EngineError>;
   }
   pub trait CanvasPainter { fn paint_bytes(&mut self, bytes: &[u8]) -> Result<PaintStats, PaintError>; }
   ```
   `EngineError` — новый публичный enum в `crates/render` (`#[non_exhaustive]`), варианты: `Parse(String)`, `Layout(String)`, `Cancelled` (вариант `Cancelled` понадобится в T1.3, добавить сразу). `ZipLimits` импортируется из `doc-converter-core`.
3. **Реализации:** `Painter2D` (`crates/render/src/painter/painter_2d.rs`, уже имеет `paint_bytes`) получает `impl CanvasPainter`; main-painter из `crates/wasm/src/painter_api.rs` (`init_painter_main`) — второй impl с тем же сигнатурным методом. В `crates/xlsx` новая структура `XlsxPlugin` (модуль `crates/xlsx/src/plugin.rs`): `id()` → `"xlsx"`, `parse` оборачивает существующий `open` книги в `Box<dyn DocumentModel>` (`XlsxDocument`). В `crates/docx` — `DocxPlugin`/`DocxDocument` (`id()` → `"docx"`) аналогично над существующим `parse_docx` и раскладкой `crates/docx/src/layout/engine.rs`.
4. **Диспетчеризация на TS-стороне:** в `packages/core/src/worker/worker.ts` обработка `open` уже ветвится по `msg.format` (сейчас: `xlsx` → основной модуль, `docx` → ленивый docx-wasm); это остаётся единственным местом выбора модуля, новые форматы (pptx/odf — T3.1/T3.3) добавляются новыми ветками этого switch с ленивой загрузкой своего модуля. Каждый wasm-модуль вызывает свой плагин из собственных экспортов: `crates/wasm` (`xlsx_api.rs`) — `XlsxPlugin::parse`, `crates/docx-wasm` (`docx_api.rs`) — `DocxPlugin::parse`; книга/документ по-прежнему держатся в `thread_local!` своего модуля. Клей минимальный: экспорт разбирает байты через плагин и хранит `Box<dyn DocumentModel>` (или существующую конкретную модель — решение за исполнителем только в пределах модуля: проще хранить существующий тип, обёртка `XlsxDocument` нужна только там, где экспорт вызывает `DocumentModel`-методы).
5. **Экспорт в DisplayList остаётся без изменений:** `build_display_list` внутри `XlsxDocument`/`DocxDocument` зовёт существующие `xlsx::paint::build_display_list` и `docx::render` — правок формата нет, `DL_VERSION` не трогается.

Верификация: `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm build:wasm && pnpm turbo run typecheck test build`, `pnpm size-limit` (бюджеты не изменились), e2e `pnpm -F @meerrec/example-viewer exec playwright test e2e/docx e2e/xlsx --project=chromium`. Новые unit-тесты: `XlsxPlugin::parse` и `DocxPlugin::parse` на своих фикстурах дают `DocumentModel` с верным `page_count()` и `id()`; в дереве нет `registry.rs`/`open_any`/`can_parse` (grep по `crates/` и `packages/` пуст).

### T1.2. ADR-0031: потоковый ZIP и `ReadableStream`-вход

Размер XL. Зависит от T1.1 (парс через trait-слой) и T1.3 отмену не требует, но проверочные точки отмены удобно делать вместе с T1.3 — можно в одном эпике подряд.

Сейчас `crates/core/src/archive.rs` держит `Archive { inner: zip::ZipArchive<Cursor<Vec<u8>>> }` — файл целиком в RAM воркера и main'а. Лимиты ADR-0015 (`ZipLimits`) уже стоят и остаются. Цель: приём документа чанками (`ReadableStream<Uint8Array>`) с прогрессом, без материализации файла на main-потоке, с парсом только после прихода хвоста ZIP (central directory в конце — парс раньше невозможен по формату).

Шаги:
1. **ADR `docs/adr/0031-streaming-zip.md`:** фиксирует: (а) вход `ReadableStream<Uint8Array>` дополнителен, `ArrayBuffer` остаётся; (б) парс стартует после EOCD (конец архива), поэтому «инкрементальный парсинг» = экономия памяти main-потока + прогресс + отменяемая загрузка, а не ранний рендер — это фиксируется числами (пик памяти main-потока для 55-МиБ файла падает с 55 МиБ до размера одного чанка); (в) бюджеты §9 не меняются (DOCX ≤ 200 МиБ, PDF ≤ 40 МиБ), для stream-пути добавляется регрессия «пик ≤ путь-через-буфер + 32 МиБ»; (г) `async_trait` не вводится — чанки копятся в буфере воркера, парс синхронный после `end`; (д) sketch-трейт `ChunkReader: Read+Seek+Send` из документа реализуется как внутренний тип `SegmentedBuffer` (не публичный trait): `Read+Seek` над списком чанков, чтобы `zip::ZipArchive<R: Read+Seek>` заработал без переписывания библиотеки.
2. **`crates/core/src/chunk_reader.rs` (новый):** `pub struct SegmentedBuffer { segments: Vec<Box<[u8]>>, total: u64 }` c методами `append(&mut self, bytes: &[u8]) -> u64` (возвращает новый total), `impl Read for &SegmentedBuffer` (при чтении за границей известных байтов — `ErrorKind::UnexpectedEof`, не panic), `impl Seek` (позиции в пределах `total`). `Archive::from_segmented(buf: &SegmentedBuffer) -> Result<Archive>` — открывает `ZipArchive<&SegmentedBuffer>`; существующий `Archive::new(bytes: Vec<u8>)` остаётся (обратная совместимость, оба пути — через общий `fn open<R: Read + Seek>(reader: R)`, к которому сводится и `new`).
3. **wasm-экспорт** в `crates/wasm/src/lib.rs` (новый модуль `stream_api.rs`): `pub fn stream_new() -> u32` (id буфера в `thread_local`-реестре `RefCell<HashMap<u32, SegmentedBuffer>>`), `pub fn stream_append(id: u32, bytes: &[u8]) -> u64`, `pub fn stream_free(id: u32)`. Граница: байты чанка копируются в буфер (копия на границе неизбежна без SAB — зафиксировать в ADR).
4. **Протокол** `packages/core/src/protocol.ts` — три новых InMsg (конверт, не wire-DTO, `gen:wire` не затрагивается):
   ```ts
   export interface OpenStreamMsg { type: 'open-stream'; id: number; format: DocFormat; totalSize?: number }
   export interface AppendMsg { type: 'append'; id: number; bytes: ArrayBuffer }
   export interface EndStreamMsg { type: 'end-stream'; id: number }
   ```
   Ответ — существующие `OpenedMsg`/`DocxOpenedMsg` или `WorkerErrorMsg { id, code: 'open' }`. Воркер (`packages/core/src/worker/worker.ts`) держит `Map<id, { format, streamId }>`; `append` transfer'ом → `stream_append`; `end-stream` → `parse` плагином модуля, выбранного по `format` (диспетчеризация T1.1) → `opened`.
5. **Публичный API:** в `XlsxViewerOptions`/`DocxViewerOptions` поле `onProgress?: (loaded: number, total?: number) => void`; сигнатура `open(source: ArrayBuffer | ReadableStream<Uint8Array>, opts?)` в `packages/core/src/render/xlsx_viewer.ts` и `docx_viewer.ts`. Путь `ReadableStream`: main читает стрим (`reader.read()` цикл), шлёт `open-stream` → `append` чанками по 1 МиБ → `end-stream`; прерывание стрима (`reader.cancel`) шлёт `end-stream` раньше времени → воркер отвечает `WorkerErrorMsg` (code `open`, сообщение «stream aborted»). Обработка пустого стрима (0 байт) — та же ошибка.
6. **Тесты:** Rust — `SegmentedBuffer` (seek вперёд/назад, чтение через границы сегментов, append после seek); `Archive::from_segmented` на сгенерированном docx, разрезанном на чанки по 7 байт. TS — vitest: `open` со `ReadableStream`, собранным из чанков фикстуры, вызывает `onProgress` и резолвит `opened`; обрыв стрима — reject. e2e Playwright: страница открывает документ стримом (новый e2e-файл `examples/viewer/e2e/stream.spec.ts`, chromium).
7. **Замер памяти:** скрипт-гейт по образцу существующего memory-прогона (`scripts/` memory-gates, фикстура `memory_50mib`): stream-путь — пик ≤ путь-через-буфер + 32 МиБ; число зафиксировать в отчёте задачи.

Верификация: `cargo test -p doc-converter-core`, `pnpm -F @meerrec/core exec vitest run test/stream.spec.ts` (новый), e2e `pnpm -F @meerrec/example-viewer exec playwright test e2e/stream --project=chromium`, `pnpm turbo run typecheck test build`, `pnpm size-limit`.

### T1.3. ADR-0032: отмена операций (`AbortSignal`)

Размер L. Зависит от T1.1 (флаг отмены в `EngineError::Cancelled` уже заведён).

Сейчас отмены нет: тяжёлая раскладка (DOCX 150 стр., плотные XLSX) блокирует воркер, `destroy()` лишь отклоняет pending на TS. Цель — кооперативная отмена в Rust и `AbortSignal` в публичном API.

Шаги:
1. **ADR `docs/adr/0032-cancellation.md`:** фиксирует кооперативную модель (wasm однопоточный — принудительного прерывания нет): проверки флага в циклах раскладки между страницами/строками; гранулярность — страница DOCX, полоса строк XLSX; отмена не портит состояние воркера (после `Cancelled` следующая операция работает).
2. **Rust:** в `crates/wasm/src/lib.rs` и `crates/docx-wasm/src/lib.rs` — `thread_local!` флаг `static CANCEL: Cell<bool>` + экспорты `pub fn set_cancel(flag: bool)` (или счётчик-поколение `cancel(id)` — решение: флаг `bool`, одно активное тяжёлое вычисление на воркер). Точки проверки: цикл страниц в `crates/docx/src/layout/engine.rs` (после каждой страницы — `if cancelled { return Err(Error::Cancelled) }`; в `crates/docx` новый вариант ошибки `Error::Cancelled`), цикл полос в `crates/xlsx/src/paint.rs` (аналогично, вариант в `crates/xlsx::Error`). API: функции раскладки получают `cancelled: &Cell<bool>` (не thread_local в библиотечных крейтах — передавать явно, чтобы тесты могли дёргать). Экспорты wasm сбрасывают флаг в начале каждой операции.
3. **Протокол:** `export interface CancelMsg { type: 'cancel'; id: number }` (InMsg); в `WorkerErrorCode` добавить `'cancelled'`. Воркер на `cancel` ставит флаг; при `Error::Cancelled` шлёт `WorkerErrorMsg { id, code: 'cancelled' }` и не роняет цикл кадров.
4. **Публичный API:** в `XlsxViewerHandle`/`DocxViewerHandle` — `open(source, { signal?: AbortSignal })`, `exportPdf({ signal? })`, `exportPng({ signal? })` (у DocxViewer ещё `exportDocxPdf`/`exportText`). `destroy()` шлёт `cancel` для всех pending и резолвит их reject'ом с кодом `'cancelled'`. Новый `open` при активном предыдущем — сначала `cancel` старого. Реализация в `packages/core/src/render/{xlsx_viewer,docx_viewer}.ts`: подписка `signal.addEventListener('abort', …)` с одноразовым listener, снятие в `finally`.
5. **Тесты:** Rust — unit: раскладка фикстуры с выставленным флагом возвращает `Cancelled` (детерминированно: выставить флаг до вызова). wasm-bindgen-test — `open` 150-страничной фикстуры + `set_cancel(true)` → ошибка `cancelled`, затем обычный `open` проходит (состояние цело). TS vitest — `open` с уже отменённым signal реджектится; `destroy` во время open реджектит. e2e — открытие большой фикстуры + немедленный destroy не оставляет виснущих промисов.

Верификация: `cargo test --workspace --all-features`, `wasm-pack test --headless --chrome --release crates/wasm` и `crates/docx-wasm`, `pnpm -F @meerrec/core test`, e2e docx chromium.

---

## Эпик 2. Вынос AGPL (критическое замечание №5)

### T2.1. ADR-0038: вынос Euro-Office/x2t/browser-convert в отдельный репозиторий

Размер M. Независим от Эпика 1; делать до обёрток (T4.1/T4.2), чтобы демо-страницы не конфликтовали.

Сейчас в монорепо: `packages/eurooffice` (Apache-обёртка над AGPL-движком), `packages/x2t` (обёртка над vendor `packages/x2t/vendor/*` — ONLYOFFICE x2t wasm, Git LFS, ~39 МБ `x2t.wasm`), `packages/browser-convert`, `scripts/eurooffice/assemble.mjs` (сборка AGPL-движка в `build/eurooffice/site`), демо-страницы `examples/viewer/src/pages/{convert,eurooffice}`, шаги сборки движка в `.github/workflows/pages.yml`. Цель — монорепо чистое Apache-2.0.

Шаги:
1. **ADR `docs/adr/0038-agpl-separation.md`:** фиксирует границу: всё, что грузит или собирает AGPL-движок, живёт в отдельном репозитории; монорепо не ссылается на него кодом (только README-ссылка на сайт).
2. **Новый репозиторий `meerrec/eurooffice-demo`:** создать через `gh repo create` (если прав нет — это ручной шаг, сообщить в отчёте). Скопировать **свежей историей** (решение: `git subtree`/`filter-repo` не используется — дешевле и лицензионно чище): `packages/eurooffice`, `packages/x2t` (включая `vendor/`, LFS-объекты переносятся заново через `git lfs track`), `packages/browser-convert`, `scripts/eurooffice/`, страницы `examples/viewer/src/pages/{convert,eurooffice}` и связанные e2e-файлы (`examples/viewer/e2e/{convert,eurooffice}*`), кусок `pages.yml` (сборка движка + стайдинг), `scripts/pages/stage.mjs` целиком (он двухуровневый: корень — движок). В новом репо — свой `pnpm-workspace`, свой `pages.yml` с `lfs: true`, деплой на собственный Pages-сайт.
3. **Монорепо после переезда:** удалить перечисленные пакеты/страницы/скрипты; из `.github/workflows/pages.yml` убрать `lfs: true`, шаги `assemble.mjs` и сборки `eurooffice`/`browser-convert`/`x2t` (демо остаётся одностраничным viewer'ом); из `AGENTS.md` — строки про `packages/{x2t,eurooffice,browser-convert}` в таблице каталогов и LFS-пометку; из `ROADMAP.md` §3.1/§7 — упоминания пакетов; в `examples/viewer/README.md` и корневом README — ссылка на сайт нового репозитория. История монорепо сохранит старые LFS-пойнтеры (полный scrub истории — отдельное решение, не входит).
4. **Проверка чистоты:** `grep -riE 'eurooffice|x2t|browser-convert' --exclude-dir={node_modules,dist,target,.git}` в монорепо находит только документирующие ссылки (README/ADR-0038), не код.

Верификация: в монорепо — `pnpm turbo run typecheck test build` зелёный, `pages.yml` собирает только viewer-демо; в новом репо — деплой движка работает, страница конвертера открывается; в монорепо нет файлов AGPL (проверка grep'ом из шага 4).

---

## Эпик 3. Фаза 3: форматы (PPTX, Strict, ODF, `.docm`, WebGL)

### T3.1. ADR-0033: PPTX-плагин

Размер XXL. Зависит от T1.1 (пишется сразу по trait-контракту).

PPTX — non-goal по `ROADMAP.md` §2; отменён T0.2/ADR-0039. Скоуп (решение, фиксируется в ADR): парс, раскладка слайдов, просмотр, экспорт PDF, PNG и текста (Markdown/HTML/text — конспект из текстовых рамок слайдов; решено «отменяем non goal», документ его не исключает) — полный паритет с DOCX/XLSX.

Шаги:
1. **ADR `docs/adr/0033-pptx-plugin.md`:** скоуп, границы (пересмотр non-goal §2 ROADMAP), модель, порядок работы.
2. **Новый крейт `crates/pptx`:** модель (`Presentation`: слайды `p:sldSz` размеры, шейпы `p:sp` (текст с `a:r`/`a:p`, стили через `a:defRPr`), `p:pic` (изображения), `p:graphicFrame` с диаграммами — геометрию переиспользовать из `crates/xlsx` диаграмм по ADR-0011), мастер-слайды `p:sldMaster`, темы `a:theme` (шрифты/цвета); раскладка — фиксированный размер слайда без пагинации; рендер в DisplayList (переиспользовать команды `crates/render::display_list`); PDF-экспорт через `crates/pdf` (слайд = страница, размер из `p:sldSz`). Линты/тесты/доккомментарии по конвенциям репо (`#![forbid(unsafe_code)]`, pedantic).
3. **Ленивый wasm-модуль `crates/pptx-wasm`** (по образцу `crates/docx-wasm`): экспорты `pptx_open`/`pptx_render`-аналоги docx_api + `export_pptx_pdf` + `export_pptx_text`. Пакет `packages/wasm-pptx` (сгенерированный, руками не править), `exports` c `./pkg/*`, `LICENSE`/`README` (pack-preflight в release.yml перечислить шестым пакетом, `scripts/check-package-metadata.mjs` — добавить `pptx` в список).
4. **Протокол/TS:** `DocFormat = 'xlsx' | 'docx' | 'pptx'`; `InitMsg` + `PptxModuleUrls` (по образцу `DocxModuleUrls`); воркер грузит модуль лениво; фабрика `createPptxViewer(host, options)` в `packages/core/src/render/pptx_viewer.ts` с методами `open/showSlide/setZoom/exportPdf/exportPng/exportText/hitTest/destroy`; ответ `pptx-opened { id, slides: SlideInfo[] }` (SlideInfo — wire-DTO из Rust, регенерация `pnpm gen:wire`); сообщение-ответ текста — по образцу `DocxTextMsg` (`pptx-text { id, text, format }`).
5. **CI-гейты:** в `ci.yml` `check_exports` добавить сверку `packages/wasm-pptx/pkg/*.d.ts` с `crates/pptx-wasm/src`; в `.size-limit.mjs` строку `wasm (pptx)` с лимитом **480 КБ** gzip (решение: как у viewer-модуля, запас на мастер-слайды/темы); `crates/fuzz` — таргет `pptx_presentation` + строчки в `fuzz.yml` (setup-джоба: список целей и seed).
6. **Фикстуры и оракул:** генератор `scripts/generate_pptx_fixtures.ts` (детерминированный, свой ZIP-писатель, по образцу docx-генератора), каталог `test-fixtures/pptx/`; в `.github/workflows/oracle.yml` джоба `pptx` (LibreOffice Impress: `libreoffice-impress` в apt-списке, `soffice --convert-to pdf`, SSIM через ffmpeg) + скрипт `scripts/oracle-pptx.sh`. Дифференциальный гейт по образцу `crates/docx/tests/differential.rs` (порог ≥ 0.95) — только если есть независимый эталон для слайдов; если нет — SSIM-оракул достаточен (зафиксировать в отчёте).

Верификация: `cargo test -p doc-converter-pptx`, `wasm-pack test --headless --chrome --release crates/pptx-wasm`, `pnpm build:wasm && pnpm turbo run typecheck test build`, `pnpm size-limit` (новая строка зелёная), e2e `examples/viewer/e2e/pptx.spec.ts` (новая страница `#/pptx` в демо), оракул-джоба `pptx` зелёная, fuzz-таргет собирается (`cargo fuzz list --fuzz-dir crates/fuzz` показывает `pptx_presentation`).

### T3.2. ADR-0034: Strict OOXML (пересмотр ADR-0017)

Размер L. Зависит от T1.1 (только организационно; можно параллельно с T3.1).

Сейчас `crates/docx/src/parse.rs::ensure_transitional` отклоняет Strict с `Error::StrictNotSupported` (ADR-0017 §1). Парсеры сравнивают `(namespace, local_name)` — механизм готов к таблице namespace'ов.

Шаги:
1. **ADR `docs/adr/0034-strict-ooxml.md`:** пересмотр ADR-0017 (новым ADR, не правкой): Strict поддерживается для DOCX **и XLSX** (решение: оба формата — механизм общий, таблицы namespace'ов единообразны; SpreadsheetML Strict `http://purl.oclc.org/ooxml/spreadsheetml/main`). Вариант определяется по корню `document.xml`/`workbook.xml`.
2. **Rust:** в `crates/docx` — модуль `ns.rs` (новый): `pub struct NsTable { wordprocessingml: &'static str, drawingml: &'static str, relationships: &'static str }` + константы `TRANSITIONAL`/`STRICT`; заменить сравнения namespace-строк по коду на выбор таблицы по детекту варианта; `ensure_transitional` удалить, оставить `OoxmlVariant`-детект и ошибку `Unknown` для прочих namespace'ов; `Error::StrictNotSupported` удалить (вариант ошибки публичный — снять с публичного API сразу, без alias'а). В `crates/xlsx` — аналогичный `ns.rs` и детект по `xl/workbook.xml`. `[Content_Types].xml` и rels: хвосты типов связей у Strict совпадают (уже так в `crates/docx/src/rels.rs`).
3. **Фикстуры:** в генераторы `scripts/generate_docx_fixtures.ts` и `scripts/gen-fixtures.ts` добавить флаг `--strict`, генерирующий те же фикстуры с purl-namespace'ами; коммитить небольшое подмножество в `test-fixtures/docx/strict/` и `test-fixtures/xlsx/strict/`. Удалить тест `strict_package_is_rejected` (`crates/docx/src/parse.rs`), заменить на `strict_package_opens` + roundtrip.
4. **Оракул:** LibreOffice читает Strict — джобы `oracle.yml` прогнать на strict-подмножестве (добавить параметр каталога в `scripts/oracle-docx-pages.sh`/`oracle-ssim.sh` или второй прогон в джобе).
5. **Документация:** снять non-goal-формулировку из `README.md` и `AGENTS.md` (скоп «только Transitional»), заменить на «Transitional + Strict».

Верификация: `cargo test -p doc-converter-docx -p doc-converter-xlsx`, roundtrip по strict-фикстурам, `pnpm gen:docx-fixtures --strict` детерминирован (два прогона — одинаковые байты), оракул на strict-подмножестве зелёный, в `AGENTS.md`/`README.md` нет «Strict — non-goal».

### T3.3. ADR-0036: ODF (.odt/.ods/.odp)

Размер XXL. Зависит от T1.1; параллельно с T3.1/T3.2.

Формат из матрицы To-Be (роадмапа в документе нет — включён решением пользователя). Скоуп (решение, в духе «отменяем non goal» — полный паритет с офисным стеком): `.odt` — просмотр + PDF + текст (Markdown/HTML/text, паритет с DOCX); `.ods` — просмотр + PDF (паритет с XLSX); `.odp` — просмотр + PDF + текстовый конспект (паритет с PPTX из T3.1).

Шаги:
1. **ADR `docs/adr/0036-odf-plugin.md`:** скоуп (odt/ods/odp), ODF 1.2, namespace `urn:oasis:names:tc:opendocument:xmlns:…`, решения по стилям (automatic + named styles, `fo:`/`style:`-свойства), лимиты переиспользуют `ZipLimits`.
2. **Новый крейт `crates/odf`** (решение: один крейт на все три формата, модули `odt`/`ods`/`odp` — общий манифест `META-INF/manifest.xml`, общий `content.xml`-каркас): модель ODT (абзацы `text:p`, заголовки `text:h`, списки `text:list`, таблицы `table:table`, изображения `draw:frame`/`draw:image`, поля `text:page-number`), модель ODS (таблицы, ячейки, стили чисел по образцу `crates/xlsx`), модель ODP (`draw:page`, текст-боксы, изображения — по образцу `crates/pptx` из T3.1); раскладка — метрики `render::text_measure`, пагинация по `fo:page-width/height` (для ODP — фиксированный размер слайда); рендер в DisplayList; PDF через `crates/pdf`.
3. **Ленивый модуль `crates/odf-wasm`** (образец — `crates/docx-wasm`): `odt_open`/`odt_render`/`export_odt_pdf`/`export_odt_text`, `ods_open`/`ods_render`/`export_ods_pdf`, `odp_open`/`odp_render`/`export_odp_pdf`/`export_odp_text`. Пакет `packages/wasm-odf`; release.yml pack-preflight — седьмым пакетом; `check-package-metadata.mjs` — добавить `odf`; `ci.yml` `check_exports` — сверка `packages/wasm-odf/pkg/*.d.ts`.
4. **Протокол/TS:** `DocFormat = 'xlsx' | 'docx' | 'pptx' | 'odt' | 'ods' | 'odp'`; `OdfModuleUrls` в `InitMsg`; фабрики `createOdtViewer`/`createOdsViewer`/`createOdpViewer` (решение: три фабрики, по образцу pptx/docx; общий внутренний модуль `packages/core/src/render/odf_viewer.ts`); ответы `odf-opened { id, pages | sheets | slides }`.
5. **Бюджеты/фаззинг:** `.size-limit.mjs` строка `wasm (odf)` — лимит **480 КБ** gzip (решение: как у viewer/docx); fuzz-таргеты `odt_content`, `ods_content`, `odp_content` + регистрация в `fuzz.yml`.
6. **Фикстуры и оракул:** генератор `scripts/generate_odf_fixtures.ts` (детерминированный), `test-fixtures/odf/`; джоба `odf` в `oracle.yml` (LibreOffice Writer/Calc/Impress — `libreoffice-impress` добавляется в apt-список; скрипт `scripts/oracle-odf.sh`).

Верификация: `cargo test -p doc-converter-odf`, `wasm-pack test --headless --chrome --release crates/odf-wasm`, `pnpm size-limit`, e2e `examples/viewer/e2e/odf.spec.ts` (страницы `#/odt`, `#/ods`, `#/odp`), оракул `odf` зелёный, fuzz-таргеты собираются.

### T3.5. Поддержка `.docm` (отмена non-goal §1.9)

Размер S. Независима (правки в `crates/docx`), можно параллельно с T3.1–T3.3.

Сейчас `.docm` отклоняется ранним отказом `Error::MacroEnabledDocument` (`crates/docx/src/error.rs`); фикстура `macro_enabled.docx` уже есть в `test-fixtures/docx/broken/` и по замыслу фатальна. Отмена non-goal (T0.2) делает `.docm` поддерживаемым: структура DOCX идентична, отличие — часть `word/vbaProject.bin`.

Шаги:
1. **Rust:** в `crates/docx/src/parse.rs` убрать ранний отказ по `vbaProject.bin`; пакет разбирается как обычный DOCX, макрос-часть игнорируется (в модель не попадает). `Error::MacroEnabledDocument` удалить из публичного API (`crates/docx/src/error.rs`) — вариант публичный, снимается сразу без alias'а; убедиться, что ссылок на него нет в `crates/wasm`/`docx-wasm` (grep `MacroEnabledDocument`).
2. **Тесты:** фикстуру `test-fixtures/docx/broken/macro_enabled.docx` перенести из «фатальных» в обычные (обновить список фатальных в `crates/docx/tests/layout.rs` и любых списках-сайдкарах; ожидание — документ открывается, макрос-часть отсутствует в модели); новый unit-тест: пакет с `vbaProject.bin` открывается, roundtrip модели проходит.
3. **Документация:** в `AGENTS.md`/`README.md` убрать `.docm` из списка неподдерживаемых (правка уже зарезервирована за T0.2 — здесь только фактическая поддержка в коде).

Верификация: `cargo test -p doc-converter-docx`, дифференциальный прогон не регрессировал (`cargo test -p doc-converter-docx --test differential`), фикстура `macro_enabled.docx` открывается (тест из шага 2 зелёный).

### T3.4. ADR-0035: WebGL-рендерер для больших XLSX

Размер XL. Независим от T3.1–T3.3 (можно параллельно); зависит от T1.1 только через общий `CanvasPainter`.

Цель документа: WebGL для XLSX > 100 000 ячеек, «60 FPS на сложных таблицах». Painter сейчас только Canvas2D (`crates/render/src/painter/painter_2d.rs`).

Шаги:
1. **ADR `docs/adr/0035-webgl-renderer.md`:** фиксирует: (а) DisplayList остаётся единственным IR — GL-painter читает те же байты, командный формат не меняется, `DL_VERSION` не трогается; (б) выбор рендерера на TS-стороне (кто владеет контекстом канваса): `RenderConfig.backend?: '2d' | 'webgl' | 'auto'`; `auto` = WebGL при числе ячеек > 100 000 (из `opened`-метаданных листа), иначе 2d; (в) текст — глиф-атлас: растеризация глифов через 2d-канвас в текстуры, метрики из `FontRegistry`/`render::text_measure`; (г) бюджет-гейт мягкий по прецеденту `painter_perf` (headless-раннер рисует программно — жёсткие 16 мс мерили бы среду).
2. **Новый `crates/render/src/painter/painter_gl.rs`** (`#[cfg(target_arch = "wasm32")]`): `PainterGL { ctx: WebGlRenderingContext2d, atlas: GlyphAtlas, state: PaintState }` с `impl CanvasPainter`; `GlyphAtlas` — LRU-кэш глифов по (font, size, glyph), по образцу `BitmapCache` (`crates/render/src/painter/bitmap_cache.rs`), с `clear()`.
3. **TS:** `packages/core/src/protocol.ts` — `RenderConfig.backend`; `packages/core/src/render/offscreen.ts`/`xlsx_viewer.ts` — получение `webgl`-контекста (`OffscreenCanvas.getContext('webgl')` в воркере, `canvas.getContext('webgl')` в main-режиме); выбор `auto` по `SheetInfo` (добавить в wire-DTO `cellCount` — правка `crates/core/src/wire.rs` + `pnpm gen:wire` + гейт дрейфа).
4. **Тесты:** wasm-bindgen-test `crates/wasm/tests/painter_gl_perf.rs` — кадр 1200×800 на 100k ячеек, мягкий порог 300 мс (по прецеденту `painter_perf.rs`); e2e — страница XLSX с книгой на 100k+ ячеек в chromium (фикстура генерируется `scripts/gen-fixtures.ts`), проверка что при `backend: 'webgl'` кадр рисуется и `tick` приходит.

Верификация: `wasm-pack test --headless --chrome --release crates/wasm`, `pnpm turbo run typecheck test build`, `pnpm size-limit` (viewer-модуль не вышел из бюджета 480 КБ — GL-код в нём; если выйдет — стоп и вопрос, бюджет не ослаблять молча), e2e webgl chromium зелёный.

---

## Эпик 4. Фаза 4: индустриализация

### T4.1. Обёртка `@meerrec/react`

Размер M. Независима; демо-страница — после T2.1 (страницы демо освобождены).

Бюджет из `ROADMAP.md` §9: `@meerrec/react` — 25 КБ gzip.

Шаги:
1. **Новый пакет `packages/react`** (`@meerrec/react`, версия 1.0.0, лицензия Apache-2.0, `repository`/`homepage`/`bugs` по образцу `packages/core`, `LICENSE` = корневой): dependency `@meerrec/core`, peerDependency `react ^18 || ^19`; tsup-сборка ESM+CJS+d.ts; `files: [dist, README.md, LICENSE]`.
2. **API (решение — тонкая обёртка над фабриками core, без своего рендера/стилей):**
   ```ts
   export function useXlsxViewer(canvasRef: RefObject<HTMLCanvasElement>, options?: XlsxViewerOptions): XlsxViewerHandle | null;
   export function useDocxViewer(canvasRef: RefObject<HTMLCanvasElement>, options?: DocxViewerOptions): DocxViewerHandle | null;
   export function XlsxViewer(props: { canvasRef?: RefObject<HTMLCanvasElement>; options?: XlsxViewerOptions }): JSX.Element;
   export function DocxViewer(props: { canvasRef?: RefObject<HTMLCanvasElement>; options?: DocxViewerOptions }): JSX.Element;
   ```
   Хуки: `useEffect` создаёт вьюер, `destroy()` в cleanup, повторное создание при смене `options`; при отсутствии `OffscreenCanvas` полагаются на `renderMode: 'auto'` из ADR-0029. JSDoc на всех экспортах.
3. **Тесты:** vitest + jsdom (devDeps `react`, `react-dom`, `@testing-library/react`) — монтирование компонента на заглушке канваса, вызовы `open`/`destroy`; mock `@meerrec/core`.
4. **Бюджет:** строка в `.size-limit.mjs` `{ path: 'packages/react/dist/index.js', limit: '25 KB', gzip: true }`.
5. **Демо:** после T2.1 — страница `examples/viewer/src/pages/react` + маршрут `#/react`, показывающая тот же просмотрщик через компонент (переиспользовать `components/office` по образцу страниц docx/xlsx); e2e `examples/viewer/e2e/react.spec.ts`.

Верификация: `pnpm -F @meerrec/react test`, `pnpm -F @meerrec/react exec tsc --noEmit`, `pnpm size-limit` (новая строка ≤ 25 КБ), e2e react chromium.

### T4.2. Обёртки `@meerrec/vue` и `@meerrec/svelte`

Размер M. Независима; параллельно с T4.1.

Шаги:
1. **`packages/vue`** (`@meerrec/vue`): dependency `@meerrec/core`, peer `vue ^3`; API: composable `useXlsxViewer(canvasRef: Ref<HTMLCanvasElement | null>, options?)` (создание в `onMounted`, `destroy()` в `onUnmounted`) + компонент `XlsxViewer` через `defineComponent` с render-функцией (без SFC — сборка tsup проще). DevDeps: `vue`, `@vue/test-utils` для vitest.
2. **`packages/svelte`** (`@meerrec/svelte`): dependency `@meerrec/core`, peer `svelte ^4 || ^5`; компонент `XlsxViewer.svelte` (bind:this на canvas, `onMount`/`onDestroy`) + store `createViewerStore(options)`; devDeps `svelte`, `@testing-library/svelte`.
3. **Бюджеты (решение):** обеим по строке `.size-limit.mjs` с лимитом **25 КБ** gzip (как у react — §9 задаёт 25 КБ только react, но единый лимит для тонких обёрток). Тесты — vitest: монтирование, создание/уничтожение вьюера, проброс `open`.
4. **Демо:** страницы `#/vue`, `#/svelte` в `examples/viewer` НЕ добавляются (приложение демо — React; интеграция Vue/Svelte в него — лишняя зависимость демо). Покрытие — vitest + typecheck; обоснование в README пакетов.

Верификация: `pnpm -F @meerrec/vue test && pnpm -F @meerrec/svelte test`, `pnpm size-limit` (две новые строки ≤ 25 КБ), typecheck обоих пакетов.

### T4.3. CLI под Deno и Bun

Размер L. Независима.

Сейчас CLI — Node-only: `node:worker_threads` (`pool.ts`, `worker.ts`), `node:fs/promises` (`wasm.ts`, `convert.ts`, `expand.ts`, `run.ts`), `node:module::createRequire` (`wasm.ts`, `run.ts`), `node:os` (`args.ts`), `node:path`. Bun реализует `node:*` достаточно полно; Deno — частично (worker_threads ограничены, `createRequire` нет смысла без node_modules-раскладки).

Шаги:
1. **Слой адаптеров** в `packages/cli/src/runtime.ts` (новый): интерфейс
   ```ts
   export interface PoolBackend { submit(job: Job): Promise<Result>; close(): Promise<void>; }
   export interface CliRuntime {
     createPool(workerUrl: string, size: number): PoolBackend;
     loadWasmBytes(jsSpecifier: string): Promise<Uint8Array>;
     availableParallelism(): number;
   }
   export function detectRuntime(): 'node' | 'bun' | 'deno'; // Deno: 'Deno' in globalThis; Bun: process.versions.bun
   ```
   `runtime-node.ts` — текущие `node:worker_threads` + `createRequire` + `readFile` (используется и под Bun); `runtime-deno.ts` — Web `Worker` (`new Worker(new URL('./worker-deno.ts', import.meta.url), { type: 'module' })`, тот же `postMessage`-протокол `worker-protocol.ts`), wasm-байты через `import.meta.resolve` npm-спекуляра + `Deno.readFile`. `pool.ts`/`worker.ts`/`wasm.ts` переписываются на `CliRuntime` без смены внешнего поведения; `worker-protocol.ts` не меняется.
2. **Публикация:** dist остаётся одним (ESM, `type: module`) — Node/Bun запускают `dist/cli.js`, Deno — `deno run --allow-read --allow-write npm:@meerrec/cli` (или локальный путь). `engines` оставить `node >= 20`; в README CLI — таблица рантаймов и команды для Deno/Bun.
3. **CI:** новый workflow `.github/workflows/cli-runtimes.yml`: джоба собирает `pnpm build:wasm && pnpm -F @meerrec/cli build`, затем матрица `[deno, bun]`: установка рантайма, `convert test-fixtures/docx/simple/one_paragraph.docx -o out.pdf` (для deno — `deno run` с флагами разрешений, для bun — `bun dist/cli.js`), проверка `qpdf --check out.pdf` и `pdfinfo` (poppler-utils, как в ci.yml).
4. **Контингенция (предрешена):** если Deno-реализация `node:worker_threads`/`node:fs` окажется несовместимой с каким-то путём CLI — соответствующий путь переходит на Deno-native API внутри `runtime-deno.ts` (интерфейс это уже допускает); Bun при несовместимости `node:*` — использовать его нативные API в отдельном `runtime-bun.ts` (сейчас не ожидается, файл не создавать до факта).

Верификация: локально `bun dist/cli.js convert …` и `deno run … convert …` на docx- и xlsx-фикстурах → валидный PDF (`qpdf --check`); `pnpm -F @meerrec/cli test` (vitest, node) зелёный; workflow `cli-runtimes.yml` зелёный на обеих ногах матрицы.

### T4.4. ADR-0037: OpenTelemetry-инструментация (opt-in)

Размер M. Независима.

Строка матрицы «Локальные callbacks → OpenTelemetry | APM». Конфликт с политикой «файлы пользователя не покидают устройство» (`docs/agent-contract.md` §7) снимается ADR.

Шаги:
1. **ADR `docs/adr/0037-opentelemetry.md`:** фиксирует: (а) в `@meerrec/core` — только `@opentelemetry/api` (peerDependency `^1.0.0`), никакого SDK и экспортёра в зависимостях; (б) дефолтный экспортёр отсутствует — без регистрации tracer'а потребителем спаны не создаются и по сети ничего не уходит (policy «нет сетевых вызовов в рантайме» сохранена); (в) в атрибутах спанов — только размеры/формат/идентификаторы, **никогда не байты документа**.
2. **`packages/core/src/telemetry.ts` (новый):** `const tracer = trace.getTracer('@meerrec/core', VERSION)`; функции `startSpan(name, attrs)`, `endSpan(span, error?)` с проверкой `span.isRecording()`; обёртки спанов в `xlsx_viewer.ts`/`docx_viewer.ts` вокруг `open`, `render` (по одному спаны на вызов, не на кадр — решение: на кадр только `tick`-статистика остаётся локальной), `exportPdf`, `exportPng`; атрибуты: `format`, `bytes`, `pageCount`/`sheetCount`.
3. **Тесты:** vitest — без зарегистрированного tracer'а код работает как раньше (no-op); с devDep `@opentelemetry/sdk-trace-base` + `InMemorySpanExporter` — спаны `viewer.open`/`viewer.export` создаются с атрибутами и без байтов в атрибутах.
4. **Бюджет:** `@meerrec/core` dist ≤ 200 КБ gzip (существующая строка `.size-limit.mjs`) — otel/api должен влезть; если не влезает — спаны за `import('@opentelemetry/api')` динамическим импортом (предрешённый fallback).

Верификация: `pnpm -F @meerrec/core test` (новый файл `test/telemetry.spec.ts`), `pnpm size-limit` (core ≤ 200 КБ), `pnpm turbo run typecheck`.

### T4.5. Сверка фаззинга (закрытие пункта «cargo-fuzz» из Фазы 4)

Размер S. Независима.

Фаззинг уже существует (Спринт 8): `crates/fuzz/` — отдельный workspace, 8 зарегистрированных целей (`Cargo.toml`: `core_zip`, `core_xml_reader`, `core_rels`, `xlsx_worksheet`, `docx_document`, `docx_numbering`, `docx_rels`, `docx_styles`), ночной прогон 15 мин/цель + `workflow_dispatch` до 330 мин. Пункт документа закрыт сверкой, не новой работой.

Шаги:
1. **Убрать мёртвый файл:** `crates/fuzz/fuzz_targets/docx_package.rs` не зарегистрирован в `Cargo.toml` и отсутствует в `fuzz.yml` — удалить (мёртвый код).
2. **Сверка списков:** `cargo fuzz list --fuzz-dir crates/fuzz` (nightly) == 8 целей из `fuzz.yml` setup-джобы == 8 `[[bin]]` в `crates/fuzz/Cargo.toml`; расхождения — выровнять по факту кода.
3. **Документирование статуса:** в отчёт задачи — что 24-ч непрерывный прогон из DoD §12 не проводился и остаётся ⚠️ (документ его не требует); фаззинг работает ночным расписанием и ручными длинными прогонами.

Верификация: `cargo fuzz list --fuzz-dir crates/fuzz` — 8 целей; `cargo build --manifest-path crates/fuzz/Cargo.toml` на nightly зелёный; новый fuzz-таргет из T3.1/T3.3 зарегистрирован и виден в списке (после этих задач).

### T4.6. CDN-дистрибуция (строка матрицы «npmjs.org + CDN»)

Размер S. Зависит от T0.1 (первый npmjs-релиз).

Шаги:
1. **Проверка работоспособности с CDN:** smoke-скрипт `scripts/cdn-smoke.mjs` (новый): импорт `@meerrec/core` из `https://esm.sh/@meerrec/core@1` и из `https://cdn.jsdelivr.net/npm/@meerrec/core@1/+esm`, создание вьюера в headless-странице Playwright против CDN-бандлов; wasm-модули (`@meerrec/wasm*`) и воркер обязаны резолвиться с того же CDN — если esm.sh/jsdelivr не отдают `.wasm`/worker-файлы как модули, зафиксировать рабочий паттерн: передача `PdfModuleUrls`/`DocxModuleUrls`/`workerUrl` явными URL на CDN (поля в `ViewerOptions` уже есть/добавляются в T1.2/T3.x — проверить фактический состав опций на момент задачи).
2. **Документация:** в корневой `README.md` раздел «Установка» — сниппеты CDN-импорта для esm.sh и jsdelivr + ограничения (CORS на `.wasm`, worker через `workerUrl`); в `docs/cookbook/` рецепт «Viewer с CDN».
3. **CI:** smoke-скрипт не в постоянный CI (сеть к CDN нестабильна на раннерах) — запуск вручную, результат фиксируется в отчёте задачи; гейт не создаётся.

Верификация: `node scripts/cdn-smoke.mjs` — вьюер открывает docx-фикстуру с CDN-бандлами, `tick` приходит; README содержит оба сниппета.

### T4.7. Native COOP/COEP-хостинг демо (строка матрицы «SW + native COOP/COEP»)

Размер M. Независима (инфра-задача + документы).

Сейчас демо на GitHub Pages получает SAB-изоляцию только через sw-патчинг (`scripts/pages/stage.mjs`, хрупко во вложенных iframe — крит. замечание №3). Цель — второй хостинг с настоящими заголовками.

Шаги:
1. **Решение по хостингу (зафиксировано): Cloudflare Pages** — бесплатный, заголовки через конфиг-файл, домен организации. В репо: файл `demo/cloudflare/_headers` (новый) с `/*: Cross-Origin-Opener-Policy: same-origin` и `Cross-Origin-Embedder-Policy: require-corp`; `demo/cloudflare/README.md` — инструкция подключения проекта CF Pages к артефакту `.pages/` (сборка — существующий `pages.yml`, артефакт забирается вручную/по токену; CI-деплой в CF не вводится, чтобы не плодить секреты — решение).
2. **Проверка:** после деплоя — `curl -sI https://<домен>/viewer/` показывает оба заголовка; в консоли страницы `crossOriginIsolated === true` и e2e-прогон `sab`-транспорта против развёрнутого домена (локальный запуск Playwright с `baseURL` на CF-домен — команда задокументирована в `demo/cloudflare/README.md`).
3. **Документация:** корневой `README.md` (раздел COOP/COEP) — два способа: Pages+SW (текущий) и native-заголовки (рекомендуемый для продакшена, пример конфига).

Верификация: `curl -sI` возвращает оба заголовка; e2e `sab` на CF-домене зелёный; README содержит оба способа.

### T4.8. MS Office SSIM-оракул (строка матрицы «+ MS Office SSIM»)

Размер M. Независима; в постоянный CI не входит (лицензии Office и Windows-раннер — решение; «MS Word в CI нет» из DoD §12 не отменяется этим планом, оракул — ручной харнесс).

Шаги:
1. **Скрипт `scripts/oracle-msoffice.ps1` (новый, PowerShell для Windows):** для каталога фикстур — Word/Excel COM-автоматизация (`Documents.Open` → `ExportAsFixedFormat` в PDF; Excel `ExportAsFixedFormat`), затем попарный SSIM с нашим экспортом (наш PDF — из `cargo build --release -p doc-converter-pdf --example mem_probe`/`docx_export`, Windows-сборка; SSIM — через ImageMagick `compare`/ffmpeg, как в `scripts/oracle-ssim.sh`). Вывод — отчёт `oracle-out-msoffice/report.md` в том же формате, что у существующих оракулов.
2. **Требования и запуск** (документируются в шапке скрипта): Windows + установленный Microsoft Office (Word/Excel) + Rust-тулчейн 1.98.0 + ImageMagick; запуск — `powershell -File scripts/oracle-msoffice.ps1` на машине с Office.
3. **Документация:** `docs/testing/` — заметка «MS Office-оракул: когда запускать и как читать» (пороги — те же ≥ 0.95 SSIM по образцу LibreOffice-гейтов; расхождения фиксируются в `docs/sprint-*/`-отчётах, не в CI).

Верификация: один прогон на доступной Windows-машине с Office — отчёт создан, числа SSIM задокументированы; скрипт без Office выходит с понятным сообщением (не падает молча).

---

## Critical files & anchors

- `crates/core/src/archive.rs` — `Archive` над `Cursor<Vec<u8>>`: точка входа T1.2 (обобщение до `Read+Seek`, `from_segmented`).
- `packages/core/src/protocol.ts` — источник истины конверта InMsg/OutMsg: правки T1.2 (`open-stream`/`append`/`end-stream`), T1.3 (`cancel`, код `'cancelled'`), T3.1/T3.3 (`DocFormat`, `*ModuleUrls`), T3.4 (`RenderConfig.backend`).
- `crates/docx/src/layout/engine.rs` — цикл раскладки страниц: проверочные точки отмены T1.3.
- `crates/wasm/src/lib.rs` + `xlsx_api.rs` — `thread_local!`-состояние; диспетчеризация форматов — в `packages/core/src/worker/worker.ts` (без wasm-роутера, T1.1); `stream_api.rs` (T1.2), флаг отмены (T1.3).
- `.github/workflows/release.yml` — джоба `publish`: шаги npmjs/GHP-публикации для T0.1 (OIDC-правка), расширение pack-preflight новыми пакетами (T3.1/T3.3).

---

## Verification (сквозная)

После каждого эпика, в дополнение к пошаговым командам:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
pnpm build:wasm && pnpm turbo run typecheck test build
pnpm size-limit          # все строки бюджета зелёные, включая новые wasm (pptx)/(odf) и обёртки
```

E2E: `pnpm -F @meerrec/example-viewer exec playwright test --project=chromium` (все новые spec-файлы: `stream`, `pptx`, `odf`, `react`, webgl-сценарий). Оракулы: `oracle.yml` (существующие ssim/docx-джобы не регрессировали; новые pptx/odf/strict-прогоны зелёные). Fuzz: `cargo fuzz list --fuzz-dir crates/fuzz` полон. Публикация: `npm view` по всем пакетам `@meerrec/*`. CDN: `node scripts/cdn-smoke.mjs` зелёный (T4.6). Native-заголовки: `curl -sI <CF-домен>/viewer/` показывает COOP/COEP (T4.7). MS Office: `scripts/oracle-msoffice.ps1` — один прогон с отчётом (T4.8, ручной).

## Assumptions & contingencies

- **npmjs-предпосылки** (организация `@meerrec`, токен, переменная) — ручные и внешние. Если они недоступны исполнителю: вся кодовая часть T0.1 (шаги 4–5) выполняется, релиз помечается блокированным с точным чек-листом для владельца репо.
- **Новый репозиторий для AGPL** — нужны права на создание репо в организации `meerrec`. Если прав нет: файлы переносятся в локальную заготовку (tar/патч-набор + README-инструкция создания репо), монорепо чистится; деплой нового сайта — после создания репо владельцем.
- **WebGL-перф-гейт:** если headless-раннер даёт разброс среды ~×1.7 (прецедент `painter_perf`), порог держать мягким (300 мс), жёсткие 16 мс — цель для живой машины; порог не занижать без вопроса.
- **OTel и бюджет core (200 КБ):** если `@opentelemetry/api` не влезает — динамический импорт (предрешено в T4.4).
- **Stream-бюджет:** если «пик ≤ путь-через-буфер + 32 МиБ» не сходится на `memory_50mib` — зафиксировать фактические числа в отчёте T1.2 и вернуться с вопросом; бюджет не ослаблять молча.
- **PPTX/ODF размеры:** лимиты 480 КБ — стартовые; превышение на реальных фикстурах — стоп и вопрос (бюджетные строки не двигать без пользователя).
- **Deno/Bun:** Bun не требует отдельного адаптера (совместим с `node:*`); файл `runtime-bun.ts` создаётся только при фактической несовместимости.
- **CDN:** если esm.sh/jsdelivr не отдают `.wasm`/worker как модули (CORS/тип контента) — рабочий паттерн фиксируется на явных `workerUrl`/`*ModuleUrls` (предрешено в T4.6), задача не считается заблокированной.
- **MS Office-оракул:** если Windows-машины с Office нет в доступе — скрипт сдаётся с шапкой-требованиями и smoke-проверкой «без Office выходит с понятной ошибкой»; реальный прогон помечается блокированным для владельца (лицензии Office — внешняя предпосылка).
- **Cloudflare Pages:** если организации CF Pages недоступны/не заведён домен — конфиг и инструкция сдаются в репо (`demo/cloudflare/`), развёртывание помечается блокированным; заголовочная проверка переносится на локальный статик-сервер с `_headers` (netlify-style) как замена доказательства.
