# S4.2 — Полный локальный прогон релиз-кандидата

Дата: 13.10.2026. Точка замера: `7dcf06e` (`fix(docx): lay table rows on
the twip grid like Word and LibreOffice`, 17:34 +0500) — голова `main` на
момент прогона, правок поверх не делалось. Машина: macOS (Apple Silicon),
та же, что у замеров Спринта 11. Путь прогона — джобы CI из `.github/workflows/ci.yml`
в локальном исполнении (шаги по постановке S4.2).

## Окружение

| Компонент | Версия | Примечание |
|---|---|---|
| rustc / cargo | 1.98.0 | как в CI (`rust-toolchain.toml`) |
| wasm-pack | 0.15.0 | |
| Node | 25.6.1 | CI гоняет на 20 и 22; локально доступна 25 — отмечено, а не скрыто |
| pnpm | 9.12.0 | |
| wasm-opt | отсутствует | `build-wasm.sh` пропускает оптимизацию — ровно как в wasm-джобе CI (binaryen там не ставится), числа size-limit сопоставимы с CI |
| qpdf | отсутствует | qpdf-тесты `crates/pdf/tests/external.rs` скипаются штатно (`missing("qpdf")`), `pdftoppm`/`soffice` есть — локальный режим из AGENTS.md |
| Playwright | chromium 153 (v1243), firefox 155 (v1543), webkit 26.6 (v2359) | все три браузера установлены, полный набор проектов прогнан |

Фикстуры и пробники: `target/fixtures/scale-500-pages.xlsx`,
`target/fixtures/docx-large/memory_50mib.docx` — на месте (сгенерированы
ранее). Оба пробника пересобраны из текущего дерева перед замером памяти
(`mem_probe_docx` на диске был старее HEAD).

## Сводная таблица

| Шаг | Команда | Вердикт | Время | Детали |
|---|---|---|---|---|
| 1a. fmt | `cargo fmt --all --check` | ✅ | 0,7 с | чисто |
| 1b. clippy | `cargo clippy --workspace --all-targets -- -D warnings` | ✅ | 55,1 с | 0 предупреждений |
| 1c. doc | `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` | ✅ | 11,1 с | битых intra-doc-ссылок нет |
| 1d. test | `cargo test --workspace --all-features` | ✅ | 5:01,9 | все сюиты зелёные; 1 doctest ignored (штатный, `docx/src/layout/mod.rs`) |
| 2a. deny | `cargo deny check` | ✅ | 1,6 с | `advisories ok, bans ok, licenses ok, sources ok`; warnings не-фатальные — см. разбор ниже |
| 2b. audit | `cargo audit` | ✅ | 4,0 с | 216 зависимостей, 0 уязвимостей |
| 3a. wasm | `pnpm build:wasm` | ✅ | 23,7 с | три модуля собраны (`packages/*/pkg/`); экспорты `.d.ts` не сверялись — это отдельная проверка CI-джобы wasm (см. «Вне рамок») |
| 3b. JS | `pnpm turbo run typecheck test build` | ✅ | 1:45,6 | 12/12 задач успешны; внутри — полный Playwright (80 passed, 2 skipped) |
| 3c. size | `pnpm size-limit` | ✅ | 1,2 с | все 5 строк зелёные — см. разбор |
| 4. E2E | `pnpm -F @doc-converter/example-viewer test` | ✅ | 1:35,4 | 80 passed, 2 skipped (оба — штатные) |
| 5a. память PDF | `bash scripts/check-pdf-memory.sh target/release/examples/mem_probe target/fixtures/scale-500-pages.xlsx` | ✅ | 0,8 с | **28,12 МиБ** ≤ 40 МиБ, 500 стр., экспорт 159,4 мс |
| 5b. память DOCX | `bash scripts/check-docx-memory.sh target/release/examples/mem_probe_docx target/fixtures/docx-large/memory_50mib.docx 200` | ✅ | 0,5 с | **155,50 МиБ** ≤ 200 МиБ, 6000 абз., 75 изобр., разбор 54,6 мс |

Суммарное время прогона ≈ 11 мин (без пауз между шагами).

## Разбор

### 1. Rust — чисто по всем четырём

fmt, clippy `-D warnings`, rustdoc `-D warnings`, тесты workspace —
все зелёные, ничего чинить не пришлось. Количество тестов не снималось
(вывод прогона не логировался построчно), вердикт по итоговым
`test result: ok` всех сюит и нулевому коду выхода.

### 2. Аудит — чистые, но у deny три не-фатальных warning'а

`cargo deny check` вышел с кодом 0 и итогом `advisories ok, bans ok,
licenses ok, sources ok`. При этом напечатаны warning'и (не падают CI,
но фиксируются как шум):

- `license-not-encountered`: в `deny.toml` разрешён `BSD-2-Clause`, но
  ни один крейт дерева эту лицензию не использует — несматченное
  разрешение;
- `duplicate`: в `Cargo.lock` по две версии у `miniz_oxide` (0.8.9 /
  0.9.1), `syn` (2.0.119 / 3.0.6), `unicode-width` (0.1.14 / 0.2.2) —
  нормальный результат разнородного дерева (miette/backtrace держат
  старые версии, wasm-bindgen-макросы — новые). Бансов и конфликтов
  лицензий нет.

`cargo audit`: 0 уязвимостей на 216 зависимостях — апгрейд lopdf 0.45.0
чистый.

### 3. JS — зелёное, viewer упирается в новый бюджет

`pnpm turbo run typecheck test build` — 12/12 задач, кэша нет (полный
прогон). Внутри `test` отработал весь Playwright (те же 80 passed,
2 skipped, что и в шаге 4 — E2E здесь и там один и тот же сюит).

`pnpm size-limit` — все строки зелёные:

| Строка | Бюджет | Факт |
|---|---|---|
| `packages/core/dist/worker.js` | 550 КБ | 7,17 КБ |
| `packages/core/dist/index.js` | 200 КБ | 3,34 КБ |
| `wasm (viewer)` | 480 КБ | **464,2 КБ** |
| `wasm (docx)` | 400 КБ | 385,13 КБ |
| `wasm (pdf)` | 1,05 МБ | 1,01 МБ |

`wasm (viewer)` зелёный, но впритык: 464,2 из 480 КБ (запас 3 %) —
поднаборы Carlito из `b43a2de` никуда не делись, бюджет поднят в Спринте
12 легитимно (`docs/sprint-12/budgets.md`), строка выполняет роль
стража. Заметка на будущее: любой +16 КБ в основном модуле покрасит
гейт.

### 4. E2E — 80 passed, 2 skipped, оба скипа штатные

Проекты: `chromium`, `firefox`, `webkit`, `chromium-dpr2` — все
отработали, exit 0. Два скипа — один и тот же тест
`e2e/docx/corpus.spec.ts:203:1` («открывает и рисует каждую фикстуру
корпуса») в firefox и webkit: обход корпуса фикстур задуман только для
chromium (тест сам себя скипает в остальных браузерах), это не регрессия
и не непроверенная ветка — в chromium он прошёл (в обоих прогонах).

### 5. Память — оба жёстких гейта с запасом

- PDF: 28,12 МиБ при пределе 40 (факт Спринта 11 после ADR-0022 —
  28,41 МиБ; ±1 % — шум).
- DOCX: 155,50 МиБ при пределе 200 (факт бюджета S0.2 — 155,48 МиБ).

## Вне рамок локального прогона

Следующие ноги CI в постановку S4.2 не входили и локально не
запускались — за ними остаётся CI:

- wasm-джоба: сверка экспортов `.d.ts` с `#[wasm_bindgen]`-функциями и
  проверка «в основном модуле нет `docx_*`» (`build-wasm.sh` сам эти
  проверки не делает);
- `wasm-pack test --headless --chrome` трёх модулей (браузерные
  wasm-bindgen-тесты);
- wasm32-нога rust-джобы (`cargo build --target wasm32-unknown-unknown`
  и `cargo doc --target wasm32-unknown-unknown` — проверка
  доккомментариев wasm-крейтов);
- coverage (`cargo llvm-cov`: docx ≥ 85 %, core ≥ 80 %);
- `cargo bench --workspace --no-run` (компиляция бенчей);
- `pnpm audit` (информационно, `|| true` в CI).

## Вердикт

**RC готов.** Все шаги локального прогона зелёные: Rust (fmt/clippy/
doc/test), deny + audit, сборка wasm, typecheck/test/build, size-limit
(все бюджеты соблюдены), полный Playwright на трёх браузерах + DPR=2,
оба жёстких бюджета памяти с запасом. Красных шагов нет, чинить ничего
не потребовалось. Блокирующих проблем не зафиксировано; единственные
замечания — не-фатальные warning'и `cargo deny` (несматченное
разрешение лицензии и дубликаты версий в логе) и запас бюджета
`wasm (viewer)` 3 % — носят информационный характер.
