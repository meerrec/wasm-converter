# 🗺️ ROADMAP v2: doc-converter

> Пересмотренный план разработки проекта **doc-converter** — движков просмотра и
> конвертации DOCX/XLSX в браузере и Node.js на Rust/WASM + Web Worker +
> OffscreenCanvas + PDF. Ревизия после архитектурного ревью от 05.10.2026: что
> изменилось относительно v1 — в §6.
>
> **Длительность:** 25 недель (13 спринтов: 12 плановых по 2 недели + внеплановый 5.5).
> **Команда:** 2 Rust-инженера, 1 TS-инженер, 1 QA, 1 DevOps.
> **Лицензия:** Apache-2.0.
> **Релиз v1.0:** неделя 25.
> **Текущее состояние — в §6**, там же отклонения от плана и замеры.

---

## 📑 Содержание

1. [Обзор](#1-обзор)
2. [Скоуп](#2-скоуп)
3. [Архитектура](#3-архитектура)
4. [Стек](#4-стек)
5. [Азы по фазам](#5-азы-по-фазам)
6. [Roadmap по спринтам](#6-roadmap-по-спринтам)
7. [Сводная таблица](#7-сводная-таблица)
8. [Ключевые вехи](#8-ключевые-вехи)
9. [Бенчмарки и бюджеты](#9-бенчмарки-и-бюджеты)
10. [Тесты](#10-тесты)
11. [Риски и митигация](#11-риски-и-митигация)
12. [Definition of Done v1.0](#12-definition-of-done-v10)
13. [Правила кода](#13-правила-кода)
14. [Чек-лист старта](#14-чек-лист-старта)

---

## 1. Обзор

**doc-converter** — монорепозиторий с высокопроизводительными движками для:
- Парсинга и чтения DOCX (WordprocessingML) и XLSX (SpreadsheetML).
- Рендеринга на `OffscreenCanvas` внутри `Web Worker`.
- Экспорта в PDF (ручная отрисовка через `printpdf`), PNG, Markdown, HTML, CSV.
- Работы в браузере (Chrome 90+, Firefox 96+, Safari 16.4+) и Node.js (batch-CLI).

**Публичные пакеты:**
- `@doc-converter/core` — есть
- `@doc-converter/docx` — не начат
- `@doc-converter/xlsx` — не начат (вьюер живёт в `core`)
- `@doc-converter/react` — не начат
- `@doc-converter/cli` — не начат

**Rust-крейты на crates.io:**
- `doc-converter-core`
- `doc-converter-render`
- `doc-converter-docx`
- `doc-converter-xlsx`
- `doc-converter-pdf`
- `doc-converter-wasm`

---

## 2. Скоуп

| ✅ Делаем | ❌ Не делаем |
|---|---|
| Парсинг и чтение OOXML | Редактирование документов |
| Рендеринг на OffscreenCanvas | Колаборация (Yjs / CRDT) |
| Экспорт в PDF / PNG / MD / HTML / CSV | ИИ-предложения (human-in-the-loop) |
| DOCX и XLSX | PPTX, XLSB |
| Web Worker + OffscreenCanvas | Server-side (кроме batch-CLI) |
| Zero-copy через Transferables; SAB — ускорение, когда доступен | Real-time синхронизация |

---

## 3. Архитектура

### 3.1 Структура репозитория

Дерево ниже — план целиком. Что из него уже есть на 05.10.2026: все шесть
крейтов, `packages/core`, `packages/wasm`, `examples/viewer-xlsx`,
`test-fixtures/xlsx` (100 книг + эталон), `docs/adr`. Остальных пакетов,
примеров и `test-fixtures/docx` пока нет.

```
doc-converter/
├── Cargo.toml                       # [workspace]
├── package.json                     # pnpm workspaces
├── pnpm-workspace.yaml
├── turbo.json
├── rust-toolchain.toml
├── .editorconfig
├── .github/workflows/
│   ├── ci.yml
│   ├── benchmarks.yml
│   ├── fuzz.yml
│   └── release.yml
├── crates/
│   ├── core/                        # OOXML-парсер, ZIP, XML, rels
│   ├── render/                      # DisplayList + OffscreenCanvas painter
│   ├── docx/                        # парсинг + pagination + layout
│   ├── xlsx/                        # парсинг + раскладка
│   ├── pdf/                         # ручной PDF-экспорт
│   └── wasm/                        # wasm-bindgen точка входа
├── packages/
│   ├── core/
│   ├── docx/
│   ├── xlsx/
│   ├── react/
│   └── cli/
├── examples/
│   ├── viewer-docx/
│   ├── viewer-xlsx/
│   ├── batch-convert/
│   └── worker-pool-demo/
├── benches/
├── test-fixtures/
│   ├── docx/
│   ├── xlsx/
│   └── corrupted/
├── docs/
└── scripts/
```

### 3.2 Main ↔ Worker

```
┌──────────────────────────────────────────────────────────────┐
│                       MAIN THREAD                            │
│  React UI  │  Input events  │  ResizeObserver  │  a11y tree  │
│                    │                                          │
│                    │  postMessage / transferables / SAB        │
│                    ▼                                          │
│  canvas.transferControlToOffscreen() ──┐                     │
└──────────────────────────────────────────┼────────────────────┘
                                           │
┌──────────────────────────────────────────┼────────────────────┐
│                    WORKER (N=1 viewer)   ▼                    │
│  OffscreenCanvas ← ctx = canvas.getContext('2d')              │
│  WASM heap       ← openDocx / openXlsx / buildDisplayList     │
│  Painter (Rust)  → fillText / fillRect / stroke               │
│  Exporter (Rust) → export_*_to_pdf → Uint8Array (transfer)    │
└───────────────────────────────────────────────────────────────┘
```

### 3.3 Принципы

1. Main не рендерит и не парсит — только DOM, ввод, UI.
2. `OffscreenCanvas` создаётся один раз: `transferControlToOffscreen()` → `postMessage([offscreen])`.
3. WASM инстанцируется в воркере; main получает RPC-обёртку.
4. Zero-copy: `Transferable` для DisplayList и PDF/PNG — основной путь;
   `SharedArrayBuffer` — ускорение, когда страница контролирует COOP/COEP (см. §3.4).
5. Коалесцирование кадров через `requestAnimationFrame` в воркере.
6. Fallback на main-thread рендер при отсутствии OffscreenCanvas.

### 3.4 COOP/COEP заголовки

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

`SharedArrayBuffer` включается только при этих заголовках. При embed — iframe на
чужом сайте, расширение, Electron, webview — заголовки контролирует потребитель,
а не библиотека, поэтому путь без SAB обязан быть полноценным, а не fallback'ом:
тот же API, `ArrayBuffer` + transfer вместо кольца в общей памяти.

**SAB — ускорение, не обязательное условие.** Все пути (RPC, DisplayList,
PDF/PNG transfer) работают без SAB с приемлемой производительностью; SAB даёт
×N, а не «включает». Тесты гоняют оба пути — с заголовками и без; бюджеты
производительности фиксируются для обоих отдельно (§9). Требование COOP/COEP
для SAB документируется в README (Спринт 11).

---

## 4. Стек

### Rust

| Крейт | Версия | Назначение |
|---|---|---|
| `wasm-bindgen` | 0.2 | WASM-биндинги |
| `js-sys`, `web-sys` | 0.3 | JS/DOM API |
| `quick-xml` | 0.41 | Streaming XML |
| `zip` | 2.x | Распаковка OOXML |
| `serde`, `serde_json` | 1 | Сериализация |
| `thiserror`, `miette` | 2 / 7 | Ошибки |
| `chrono` | 0.4 | Даты |
| `printpdf` | 0.8 | Генерация PDF |
| `rustybuzz` | 0.20 | Shaping, kerning |
| `skrifa` | — | Парсинг шрифтов, метрики глифов |
| `unicode-linebreak` | 0.1 | Точки переноса |
| `unicode-bidi` | 0.3 | RTL |
| `image` | 0.25 | Растровые изображения |
| `bytemuck` | 1 | Разбор байтов DisplayList |
| `flate2` | 1 | Сжатие |
| `smallvec`, `lru` | — | Оптимизации |
| `tracing`, `tracing-wasm` | — | Логи |
| `criterion`, `insta`, `proptest` | — | Тесты |
| `wasm-bindgen-test` | 0.3 | WASM-тесты |

`#![forbid(unsafe_code)]` во всех крейтах, кроме `wasm`.

**Отклонения от первоначального плана:**

- `quick-xml` 0.36 → 0.41: в 0.36 две уязвимости отказа в обслуживании
  (RUSTSEC-2026-0194, -0195) — срабатывают на файле, который пользователь просто
  открыл, а это и есть модель угроз просмотрщика чужих документов.
- `ttf-parser` заменён на `skrifa`: с 2026 года `ttf-parser` помечен
  `unmaintained` (RUSTSEC-2026-0192), а `skrifa` даёт и разбор шрифтов, и метрики
  глифов, на которых стоит `render::text_measure`. Решение принято, детали — в
  `docs/adr/0002-font-parsing.md`.
- `printpdf` 0.7 → 0.8: версия из манифеста, 0.7 в дереве не было.

### Frontend

| Технология | Назначение |
|---|---|
| TypeScript 5.5 | Типы |
| tsup 8 | ESM + CJS + d.ts |
| Vite 5 | Dev и сборка примеров |
| React 18 | Обёртки |
| OffscreenCanvas | Рендер в воркере |
| SharedArrayBuffer | Ускорение передачи кадра; без него — ArrayBuffer + transferables |
| Transferables | Zero-copy байты |
| Vitest + Playwright | Тесты |
| size-limit | Бюджеты |

### Инфра

`wasm-pack`, `wasm-opt -O4` (Binaryen), `pnpm`, Turborepo, `changesets`, GitHub Actions, ESLint 9 flat, `clippy -D warnings`, `rustfmt`.

---

## 5. Азы по фазам

### Перед фазой 1 (Фундамент)
- Rust: главы 1–15, 19 «The Rust Book»; «Crust of Rust» (Jon Gjengset).
- `wasm-bindgen` guide, `wasm-pack`, `wasm-opt`.
- OOXML: ECMA-376 Part 1 (выборочно), officeopenxml.com.
- `zip` crate: streaming read.
- `quick-xml`: namespace-aware streaming.

### Перед фазой 2 (XLSX)
- SpreadsheetML: `workbook.xml`, `worksheet.xml`, `styles.xml`, `sharedStrings.xml`.
- Cell refs (`A1`, `$A$1`, `A1:B10`), merged cells, frozen panes.
- Форматы чисел Excel (`numFmt`), serial dates.
- Canvas 2D API: `fillRect`, `fillText`, `measureText`, `save/restore`, `clip`.
- `rustybuzz`, `unicode-bidi`, `unicode-linebreak`; разбор шрифтов — `skrifa`
  (`docs/adr/0002-font-parsing.md`).
- Conditional formatting rules.
- Диаграммы: `c:barChart`, `c:lineChart`, etc.

### Перед фазой 3 (PDF)
- ISO 32000-1 (PDF 1.7) — content streams, XObject, шрифты.
- `printpdf` docs, `PdfDocument`, `PdfLayerReference`.
- Единицы: pt, mm, inch, EMU.
- Subsetting шрифтов (`allsorts`, `subsetter`).
- Line dash, annotations, PDF/UA.

### Перед фазой 4 (DOCX)
- WordprocessingML: `document.xml`, `styles.xml`, `numbering.xml`, `settings.xml`.
- Runs, paragraphs, sections, tables, floats.
- Line breaking (жадный / Knuth-Plass), hyphenation.
- Table layout: autofit, fixed, borders collapsing.

### Перед фазой 5 (Полировка)
- `SharedArrayBuffer`, `Atomics.wait/notify`, двойная буферизация.
- COOP/COEP.
- Node.js `worker_threads`.
- `cargo-deny`, `cargo-audit`.

---

## 6. Roadmap по спринтам

> **Состояние после Спринта 5 (05.10.2026).** Закрыты Фаза 1 (workspace, CI,
> core-крейты), Фаза 2 (RPC Main↔Worker, OffscreenCanvas, SAB ring) и Спринты
> 3–5: XLSX разбирается целиком и **показывается в браузере** со стилями,
> границами, условным форматированием, изображениями и гиперссылками.
>
> **Что работает.** `crates/xlsx` разбирает книгу `open()` — каталог листов,
> общие строки, стили, форматы чисел, геометрию, объединения, гиперссылки.
> `paint::build` раскладывает лист в пикселях и собирает `DisplayList`, который
> painter рисует на `OffscreenCanvas`. Спринт 5 добавил к этому:
>
> - **тему оформления** — `theme1.xml` через rels книги: `clrScheme`/
>   `fontScheme`, палитра из 12 слотов в порядке SpreadsheetML, theme-цвета
>   резолвятся в RGB (раньше рисовались чёрным);
> - **границы ячеек** — из модели в кадр: `LineStyle`, приоритет по весам
>   Excel, контур merged-диапазонов, скрытые строки и столбцы;
> - **условное форматирование** — разбор (`cellIs`, `expression`,
>   `colorScale`, `dataBar`, `iconSet`, прочие виды как `Other`), применение
>   к стилю ячейки (с `priority` и `stopIfTrue`, узкий вычислитель формул)
>   и визуальные правила (шкалы, гистограммы, значки);
> - **изображения** — чертежи и якоря (one-cell/two-cell, EMU→px), реестр
>   байтов книги с лимитами (32 МиБ на картинку, 128 МиБ на книгу),
>   отрисовка с обрезкой по квадрантам, регистрация `ImageBitmap` в воркере
>   (с перерегистрацией при ресайзе — это чинило исчезновение картинок
>   после изменения размера окна);
> - **гиперссылки** — подчёркивание и цвет `hlink` из темы, если у ячейки нет
>   своего шрифта, и клик наружу (`xlsx_hyperlink_at`, `onHyperlink`);
> - **тёмную тему** — `PaintOptions.dark`, инверсия слотов палитры
>   `lt1↔dk1`/`lt2↔dk2`, тёмное оформление просмотрщика и охрана читаемости
>   по контрасту WCAG 2.1 AA (≥ 4.5) для явных цветов; в JS —
>   `theme: 'light' | 'dark'` с переключением на лету.
>
> `createXlsxViewer` из `@doc-converter/core` даёт прокрутку, зум,
> переключение листов и определение ячейки под курсором; пример —
> `examples/viewer-xlsx/`.
>
> **Чем подтверждено.** 291 нативный тест; 110 фикстур, собранных exceljs
> (+7 по условному форматированию, +3 по изображениям; генератор стал
> детерминированным), и дифф-тест нашего разбора против эталона того же
> exceljs — в обе стороны; 28 тестов vitest в `@doc-converter/core`; пять
> сквозных тестов в настоящем Chromium, которые декодируют PNG из воркера
> и смотрят на пиксели; criterion-замеры открытия книги и условного
> форматирования.
>
> **Замеры** (Apple Silicon, релиз): миллион ячеек разбирается за **0,38 с** при
> бюджете 2 с; модель занимает **48 байт на ячейку** (≈45 МБ на миллион);
> экран таблицы собирается за **1–3 мс** и рисуется за **0–5 мс**; wasm
> просмотрщика весит **275 КБ gzip** при бюджете 300 КБ; модуль экспорта PDF
> (1,76 МБ gzip) грузится по требованию. Кадр по листу из 10k ячеек
> (`cargo bench -p doc-converter-xlsx --bench conditional`): без правил
> ≈ 5,2–5,6 мс, шкала ≈ 9,2–9,6, гистограмма ≈ 7,1–7,4, значки **8,34 мс**
> (было 14,4 — форма набора значков определялась поиском подстроки на каждом
> вызове, ~47% времени кадра; классификация переехала в разбор правила).
> Накладные расходы правил: шкала ≈ 4 мс, гистограмма ≈ 2 мс, значки ≈ 3 мс —
> все укладываются в бюджет «< 5 мс».
>
> **Отклонения от плана:**
>
> - **Дифф-тест идёт против exceljs, а не SheetJS.** `xlsx` на npm застрял на
>   0.18.5 с двумя неустранёнными advisories (prototype pollution, ReDoS);
>   исправленные версии живут только на cdn.sheetjs.com. Тянуть в дерево
>   заведомо уязвимый пакет ради теста — плохая сделка, а exceljs уже пишет
>   фикстуры, то есть остаётся независимой реализацией SpreadsheetML.
> - **Бюджет памяти считается на ячейку, а не «×2 от размера файла».** Размер
>   сжатого пакета зависит от повторяемости содержимого, а не от модели: у
>   синтетического листа из одних чисел пакет весит 2 МБ при 45 МБ модели.
>   Ориентир — 48 байт на ячейку; его сторожит тест `cell_stays_small`.
> - **Раскладка и попадание по координате живут в `crates/xlsx`, а не в
>   `crates/render`.** Сетка листа — не то же самое, что поток текста в DOCX;
>   общими у них остаются `DisplayList` и painter. Когда появится DOCX, общее
>   переедет в `render`, а не наоборот.
> - **Метрик текста пока нет.** Ширину в `xlsx::paint::estimate_width`
>   оценивают по таблице ширин Calibri, а `render::font::FontRegistry` в путь
>   рисования не подключён вовсе. Для чисел оценки хватает — цифры в Calibri
>   моноширинные, и на ширине нуля построена вся система ширин Excel, — а для
>   произвольного текста нет: переноса по словам не будет до Фазы 4.
>
> **Не сделано из запланированного:** покрытие не измеряется (в CI нет
> `cargo-llvm-cov`); `render/src/viewport.rs`, `canvas.rs` и `hit_test.rs` не
> появились как отдельные модули — их работу выполняют `xlsx::layout`,
> `painter_2d` и `xlsx::paint`; стресс-тесты на 1M ячеек в браузере (60 FPS,
> 30 минут стабильности) не гонялись; эталонов против настоящего Excel для
> границ нет — правило сверено с документацией (приоритет линий Excel 5.0/7.0,
> KB 98152) и тай-брейком по CSS 2.1; фикстур с изображениями три, а не
> двадцать, как в DoD; тест DPR=2 написан и проходит, но не закоммичен;
> painter не покрыт `wasm-bindgen-test`; переполнение слота SAB-кольца (1 МиБ)
> по-прежнему неотличимо от нехватки свободного слота — обе причины дают
> `written: false`; границы рисуют каждое внутреннее ребро дважды (визуально
> безвредно, но кадр толще); в `examples/viewer-xlsx` нет переключателя тёмной
> темы — задача решается подписчиком через `setView({ theme })`.
>
> **Дальше — Спринт 5.5.** Метрики текста (`FontRegistry` в путь рисования,
> перенос и «#####» для произвольного текста) и вынос
> `render::{viewport, canvas, hit_test}`; там же `insta`, замеры RPC, DPR=2 и
> памяти, время CI.

### Что изменилось относительно v1

| # | Изменение | Причина |
|---|---|---|
| 1 | Вставлен **Спринт 5.5** (1 неделя) между Спринтами 5 и 6 | Закрыть метрики текста и границу `render` до PDF |
| 2 | **ADR-0002** (шрифты) и **ADR-0003** (граница `render`) принимаются до Спринта 5.5 | Блокеры, а не «решим в Фазе 4» |
| 3 | Спринт 8 (DOCX) переработан: fuzz первым делом, дифференциальный тест vs mammoth, ADR 0013–0020, бюджеты памяти, лимиты ZIP, `NodeId` | Парсер чужого XML — fuzz с первого дня; DoD должен быть проверяемым; модель должна быть пригодна для Спринта 9 |
| 4 | §3.4 и §11 переписаны: SAB — ускорение, не обязательное условие | Embed-сценарий — основной, не edge case |
| 5 | Бюджет `@doc-converter/core` снижен 200 КБ → 30 КБ | Бюджет, который не может провалиться, бесполезен |
| 6 | Playwright расширен на Firefox + WebKit с Спринта 6 | Заявлена поддержка трёх браузеров — тестируем три |
| 7 | `insta` вводится с Спринта 5.5 | Snapshot DisplayList — дешёвый регресс-тест |
| 8 | CI-замер времени введён с Спринта 5 | 181 тест + 100 фикстур — уже не «< 5 мин» |
| 9 | ADR-0013 (каскад стилей DOCX) — принят до Спринта 8 | Блокер: без него модель не спроектировать |
| 10 | ADR-0014 (mc:AlternateContent) — принят до Спринта 8 | Блокер: без него часть DOCX парсится неверно |
| 11 | ADR-0015 (лимиты ZIP) — принят до Спринта 8 | Безопасность: zip bomb, zip slip |
| 12 | ADR-0016 (политика ошибок) — принят до Спринта 8 | Recover-and-continue вместо fail-fast |
| 13 | ADR-0017 (Strict vs Transitional) — принят до Спринта 8 | Non-goal: Strict OOXML |
| 14 | ADR-0018 (границы редактирования) — принят | v1 — read-only, модель только в Rust, Jotai — view state |
| 15 | ADR-0019 (NodeId) — принят до Спринта 8 | Hit-test, snapshot-тесты, отладка |
| 16 | ADR-0020 (парсер/writer) — принят до Спринта 8 | Writer в DOCX — non-goal v1 |

### Накопленные долги после Спринта 4

Перенесены в Спринт 5.5:

- `render::viewport`, `render::canvas`, `render::hit_test` — не вынесены
- метрики текста — не подключены
- `insta` — не введён
- замеры RPC, DPR=2, памяти — не сделаны
- CI-время — не замерено

### Неделя 0: Подготовка

**Задачи:**
- [ ] Установить: `rustup`, target `wasm32-unknown-unknown`, `wasm-pack`, `wasm-opt`, Node 20+, `pnpm`.
- [ ] Создать пустой репозиторий, `.editorconfig`, `.gitignore`, лицензию.
- [ ] Pre-commit хуки: `rustfmt`, `clippy`, `prettier`, `eslint`.
- [ ] Минимальный WASM-пример работает в браузере.

**DoD:** окружение воспроизводимо, `wasm-pack build` собирает бандл.

---

### Спринт 1 (недели 1–2): Workspace + Core

**Цель:** монорепа, CI, парсинг OOXML-архивов.

**Задачи:**
1. Создать структуру `crates/{core,docx,xlsx,render,pdf,wasm}`.
2. `[workspace.dependencies]` с общими версиями.
3. В `core`:
   - `OoxmlArchive` — обёртка над `zip`, lazy-read.
   - `ContentTypes` — `[Content_Types].xml`.
   - `Relationships` — `_rels/*.rels`.
   - `XmlReader` — namespace-aware `quick-xml`.
   - `Error` — `thiserror`.
   - `Metadata` — `docProps/*.xml`.
4. `wasm-pack` сборка `crates/wasm` с функцией `version()`.
5. CI: `.github/workflows/ci.yml`.
6. Фикстуры: 20 XLSX + 20 DOCX в `test-fixtures/`.
7. Тесты: parse `[Content_Types].xml`, `proptest` XML round-trip.

**Артефакт:** `@doc-converter/core@0.1.0` в npm + crates.io.

**DoD:**
- ✅ `cargo test --workspace` зелёный.
- ✅ `wasm-pack build` < 200 KB gzip — 168 КБ (на момент спринта); после
  Спринта 6 модуль просмотрщика — 275 КБ gzip при бюджете 300 КБ, модуль
  экспорта PDF (1,76 МБ gzip) грузится по требованию.
- ⬜ CI < 5 мин — время джоб не замерялось.
- ⬜ Покрытие ≥ 80% — не измеряется, `cargo-llvm-cov` в CI нет.
- ⚠️ `ContentTypes` и `Metadata` как отдельные типы не появились:
  `validate_ooxml` только проверяет наличие `[Content_Types].xml`, а
  `docProps/*.xml` не читаются вовсе.

---

### Спринт 2 (недели 3–4): RPC + Worker Protocol

**Цель:** канал Main ↔ Worker.

**Задачи:**
1. `packages/core/src/worker/protocol.ts` — `WorkerRequest`, `WorkerResponse`.
2. `packages/core/src/worker/rpc.ts` — RPC с `id`, `notify`, `cancel`.
3. `packages/core/src/render/offscreen.ts` — `createViewer`.
4. `packages/core/src/worker/worker.ts` — точка входа: `init`, `resize`, `dispose`.
5. Vite-конфиг воркера (`worker.format: 'es'`).
6. Dev-сервер с COOP/COEP.
7. Feature detection + fallback.
8. Playwright-тест: init → resize → dispose.

**DoD:**
- ⬜ Открытие воркера < 50 мс — не замерялось.
- ⬜ Round-trip RPC < 1 мс — не замерялся.
- ⚠️ Resize + DPR: `computeDpr` покрыт vitest, но в браузере с DPR=2 не
  проверялся.
- ⚠️ Playwright зелёный **в одном браузере** (Chromium), не в трёх.

---

### Спринт 3 (недели 5–6): Парсинг XLSX

**Цель:** полный парсинг XLSX в модель.

**Задачи:**
1. Модель: `Workbook`, `Worksheet`, `Cell`, `CellValue`, `CellRef`, `Range`, `StyleTable`, `Style`, `Font`, `Fill`, `Border`, `NumberFormat`, `SharedStrings`, `Merge`, `ColWidth`, `RowHeight`.
2. Парсеры: `workbook.rs`, `worksheet.rs`, `styles.rs`, `strings.rs`.
3. `NumberFormat`: `0.00`, `#,##0`, `%`, `$`, dates.
4. Тесты: 100 фикстур → 100% парсинг.
5. `proptest`: round-trip.

**DoD:**
- ✅ 100/100 фикстур — сверены с эталоном exceljs, а не просто «открылись».
- ✅ 1M ячеек < 2 с (native) — 0,38 с.
- ⚠️ Память: вместо «×2 от размера файла» бюджет переформулирован в 48 байт на
  ячейку (см. отклонения выше).
- ⬜ Покрытие ≥ 85% — не измеряется.

---

### Спринт 4 (недели 7–8): DisplayList + OffscreenCanvas

**Цель:** видимый рендер XLSX в воркере.

**Задачи:**
1. `render/src/display_list.rs` — `DrawCommand`, `DisplayList`.
2. `render/src/text_measure.rs` — `FontRegistry` + LRU-кэш.
3. `render/src/layout.rs` — раскладка, wrap, ellipsize.
4. `render/src/viewport.rs` — видимый диапазон.
5. `render/src/canvas.rs` — `OffscreenPainter`.
6. `render/src/hit_test.rs` — `cellAtPoint`, `cellRect`.
7. `xlsx/src/render.rs` — `build_display_list_for_sheet`.
8. WASM-экспорты: `build_display_list`, `paint_display_list_to_offscreen`, `hit_test`, `resize_canvas`.
9. TS-обёртка `XlsxViewer`.
10. `examples/viewer-xlsx/`.

**DoD:**
- ⬜ 60 FPS при scroll 1M ячеек — стресс-теста нет; на фикстурах кадр
  собирается за 1–3 мс и рисуется за 0–5 мс.
- ✅ Первый рендер < 100 мс — книга из фикстур открывается за 4–7 мс (строка
  состояния в примере), первый кадр — за считанные миллисекунды.
- ⬜ `hitTest` < 2 мс — работает, но не замерен.
- ⬜ Память стабильна 30 мин — не проверялась.
- ⚠️ Пункты 2–6 задач (text_measure, layout, viewport, canvas, hit_test как
  модули `crates/render`) не появились: их работу выполняют `xlsx::layout` и
  `xlsx::paint`.

---

### Спринт 5 (недели 9–10): Стили, границы, изображения

**Цель:** довести XLSX-визуал до состояния, при котором DOCX-парсер получит
готовую инфраструктуру.

**Задачи:**

1. ✅ **Тема оформления** (сделано в спринте) — `theme1.xml`:
   - разбор `<a:clrScheme>`, `<a:fontScheme>` через rels книги;
   - палитра из 12 слотов в порядке SpreadsheetML;
   - маппинг `theme`-цветов в RGB, подключение в `xlsx::paint`.
2. ✅ **Границы ячеек** (сделано в спринте):
   - `DrawCommand::Line` с `LineStyle` (`Solid`/`Dashed`/`Dotted`/`Double`), толщина — в `stroke_w`;
   - приоритет границ при конфликте (правило Excel), контур merged-диапазонов, скрытые строки и столбцы;
   - эталонов против настоящего Excel нет — правило сверено с документацией (KB 98152) и тай-брейком по CSS 2.1.
3. ✅ **Conditional formatting** (сделано в спринте):
   - разбор `conditionalFormatting` из `worksheet.xml` (`CellIs`, `Expression`, `ColorScale`, `DataBar`, `IconSet`, прочие виды — `Other`);
   - применение к финальному `Style` (`priority`, `stopIfTrue`, узкий вычислитель формул для `Expression`);
   - визуальные правила: шкалы, гистограммы, значки.
4. ✅ **Изображения** (сделано в спринте):
   - `ImageRegistry` с лимитами (32 МиБ на картинку, 128 МиБ на книгу);
   - `DrawCommand::Image` с якорем one-cell / two-cell (EMU→px), clip по границам;
   - регистрация `ImageBitmap` в воркере с перерегистрацией при ресайзе.
5. ✅ **Frozen panes** — сделано **до** спринта.
6. ✅ **Заголовки строк/столбцов** — сделано **до** спринта.
7. ✅ **Гиперссылки** (сделано в спринте): подчёркивание и цвет `hlink` из темы, клик наружу (`xlsx_hyperlink_at`, `onHyperlink`).
8. ✅ **Тёмная тема** (сделано в спринте): `PaintOptions.dark`, инверсия слотов палитры, тёмное оформление просмотрщика, контраст WCAG 2.1 AA; в JS — `theme: 'light' | 'dark'`.

**Перед началом:** фикстур под условное форматирование и изображения не было — `scripts/gen-fixtures.ts` расширен (+7 и +3 фикстуры), генератор стал детерминированным. Эталоны приоритета границ против Excel так и не собирались — вместо них документация и тай-брейк по CSS 2.1.

**DoD:**
- ✅ Тема: theme-цвета резолвятся в RGB; дефолтный `<color theme="1"/>` остаётся чёрным
- ⚠️ Границы в кадре, при конфликте побеждает сторона по правилу Excel; но проверено по документации, а не на эталонах против настоящего Excel
- ✅ Условное форматирование: накладные расходы правил < 5 мс на 10k ячеек — шкала ≈ 4 мс, гистограмма ≈ 2 мс, значки ≈ 3 мс (сборка кадра целиком — отдельный бюджет, §9)
- ⚠️ Изображения в кадре и переживают ресайз, но фикстур с картинками три, а не двадцать
- ✅ Гиперссылки: подчёркивание, цвет из темы и клик наружу (`onHyperlink`)
- ✅ Тёмная тема включается и не ломает читаемость (контраст ≥ 4.5 WCAG 2.1 AA для явных цветов)
- ⚠️ DPR=2 без потери качества: тест написан и проходит, но не закоммичен

**Риски:**
- ✅ Решено в спринте: границы строятся из модели с правилом приоритета, контур merged-диапазонов, учёт скрытых строк и столбцов.
- ✅ Закрыто: фикстуры добавлены (+7 по условному форматированию, +3 по изображениям), `scripts/gen-fixtures.ts` расширен, генератор стал детерминированным.

---

### Спринт 5.5 (неделя 11): Метрики текста + граница render

> **Внеплановый спринт.** Обоснование: три зависимости из четырёх будущих
> спринтов (6, 7, 9, 10) стоят на метриках текста и вынесенной границе `render`.

**Цель:** снять Риски A и B до начала PDF и DOCX.

**Задачи:**

1. ✅ **ADR-0002, 0003, 0005 — принять и зафиксировать** (приняты 05.10.2026;
   после реализации статусы обновлены).
2. ✅ **`render::font::FontRegistry`** — подключён в путь рисования:
   - загрузка TTF/OTF через `skrifa`;
   - метрики: advance, kerning, ascent/descent;
   - LRU-кэш по `(font_id, glyph_id, size)`.
3. ✅ **`render::text_measure`** — публичный API:
   - `measure_text(text, font, size) -> Width`;
   - `break_lines(text, font, size, max_width) -> Vec<Range>`;
   - замена `xlsx::paint::estimate_width`.
4. ✅ **`render::viewport`** — чистая геометрия:
   - `Viewport { offset, size, dpr }`;
   - `visible_range(viewport, content) -> Rect`;
   - `scroll_to(viewport, target) -> Viewport`.
5. ✅ **`render::hit_test`** — чистая геометрия над DisplayList:
   - `hit_test(dl, point) -> Option<HitTarget>`;
   - `rect_for(target) -> Rect`;
   - `xlsx::paint::cellAtPoint` — адаптер над `render::hit_test`.
6. ✅ **`render::canvas`** — painter как отдельный модуль:
   - `OffscreenPainter` — обёртка над `web_sys::OffscreenCanvasRenderingContext2d`;
   - коалесцирование кадров через `rAF` в воркере;
   - `PushClip` / `PopClip` / `PushTransform`.
7. ✅ **Диаграммы** — перенос из Спринта 5:
   - парсинг `chart*.xml`;
   - рендер в DisplayList (`DrawCommand::Chart` + примитивы).
8. ✅ **Snapshot-тесты** `insta` для DisplayList на 20 фикстурах.
9. ⚠️ **Замеры** (закрыть долги):
   - RPC round-trip;
   - DPR=2 в браузере;
   - память после 1000 открытий/закрытий;
   - время CI.

**DoD:**
- ✅ `FontRegistry` подключён, `estimate_width` удалён.
- ✅ `measure_text` < 1 мкс на 100 символов (native): **278 нс**.
- ✅ `render::{viewport, canvas, hit_test}` существуют как модули.
- ⬜ SSIM ≥ 0.95 vs MS Office на 30 эталонах — эталонов MS Office нет, долг
  переходит в Спринт 6 (дифференциальные тесты против exceljs уже есть).
- ✅ 5 типов диаграмм рендерятся — разбор пяти видов и painter; проверено
  сквозным тестом до команды кадра, не пиксельным сравнением.
- ✅ `insta` snapshot для 20 фикстур.
- ⬜ RPC round-trip < 1 мс (замер) — нативный цикл SAB есть в бенчмарках,
  браузерный замер Main↔Worker не автоматизирован.
- ✅ DPR=2 проверен в Chromium.
- ⬜ CI < 8 мин (замер) — время джоб GitHub Actions локально не измерить.

**Отклонения от плана:**

- LRU кэширует метрику по `(font_id, символ, size)`, а не по id глифа: без
  shaping символ → глиф детерминирован, а `cmap`-поиск уходил с горячего пути
  (1.4 мкс → 278 нс при бюджете 1 мкс). Кэшируется при этом advance одного
  глифа, как и требовал ADR-0005.
- Политика подстановки: у реестра есть встроенный шрифт по умолчанию —
  подрезанный Carlito (SIL OFL 1.1), метрически совместимый с Calibri. Ширины
  колонок (`MAX_DIGIT_WIDTH = 7`) остались в `xlsx`: это единица формата, а не
  измерение текста. Цифра Carlito — 0.5068 em (≈7.43 px в 11 pt), Excel
  округляет MDW до 7 px, поэтому `#####` в узких колонках появляется на
  полглифа раньше, чем раньше по таблице; снапшоты это фиксируют.
- `xlsx::paint::hit_test` остаётся раскладочным адаптером (точка → ячейка):
  DisplayList не несёт идентичности ячеек. Геометрия попадания по командам
  кадра (картинки, текст) живёт в `render::hit_test`.
- Точечная диаграмма равномерно раскладывает точки по категориям: числовая
  ось X пока не поддержана.
- `render::canvas` владеет жизненным циклом контекста, но коалесирование
  кадров осталось в `packages/core/src/worker/frame_loop.ts`: цикл событий JS
  не принадлежит Rust.
- ✅ Заодно отрисован **перенос внутри ячейки** (`wrapText`): строки
  раскладываются `render::text_measure::break_lines`, блок позиционируется по
  вертикальному выравниванию, кадр обрезан по ячейке. Это закрывает пункт
  «wrap» из Спринта 4 для XLSX.

**Влияние на срок:** +1 неделя. Релиз сдвигается на неделю 25.

---

### Спринт 6 (недели 12–13): Базовый PDF

**Цель:** экспорт XLSX-листа в PDF с корректной типографикой.

**Задачи:**

1. `pdf/src/styles.rs` — `Color`, `BorderStyle`, `CellStyle`, `TextAlign`.
2. `pdf/src/layout.rs` — `PageSize`, `PageOrientation`, `Margins`.
3. **`pdf/src/fonts.rs`** — теперь опирается на `render::FontRegistry`:
   - TTF/OTF, subsetting через `allsorts` или `subsetter`;
   - fallback на full embed для CJK.
4. **`pdf/src/text.rs`** — теперь опирается на `render::text_measure`:
   - измерение и перенос — та же логика, что в canvas.
5. `pdf/src/border.rs` — границы, включая `Double`.
6. `pdf/src/background.rs`.
7. `pdf/src/painter.rs` — `export_xlsx_sheet`.
8. Тесты: 10×10 лист → PDF открывается.

**DoD:**
- ✅ PDF открывается в Chrome/Firefox/Adobe (проверено `qpdf --check`).
- ✅ Кириллица корректна.
- ✅ 1000 ячеек → PDF < 200 КБ с subsetting: **60,8 КиБ** (62 291 Б).
- ✅ 10 страниц < 300 мс: **13 страниц — 7,95 мс** (медиана criterion).
- ✅ **Playwright расширен на Firefox + WebKit** (закрытие долга).
- ✅ Единая логика переноса: canvas и PDF дают одинаковые точки разрыва.

**Отклонения от плана:**

- Отдельный крейт `layout-core` с `LayoutEngine` не создан (ADR-0007);
  вынос раскладки отложен до Спринта 9, к DOCX-раскладке.
- PDF-экспорт вынесен в лениво загружаемый wasm-модуль (`crates/pdf-wasm` →
  `packages/wasm-pdf/pkg`, 1,76 МБ gzip): просмотрщик стартует с прежним
  лёгким модулем, модуль экспорта грузится по клику (блокер B-1, решение
  архитектора).
- Потоковой записи нет: printpdf 0.8.2 копит документ в памяти. Память
  линейна — ≈ 8,3 МиБ + 0,44 МиБ на страницу (96 страниц — 50,7 МиБ,
  `docs/sprint-6/memory-report.md`); стриминг — Спринт 7.
- `PdfOptions.compress` в printpdf 0.8.2 — no-op: `doc.compress()`
  закомментирован, потоки пишутся `with_compression(false)`, сжатия потоков
  нет.
- Условное форматирование в PDF не применяется: берутся базовые флаги стиля
  ячейки, как в canvas-пути DisplayList; отдельный срез, если понадобится.
- Сетка (`print_grid_lines`), картинки, диаграммы, гиперссылки — Спринт 7;
  пагинация пока построчная, ячейки за правой границей страницы
  отбрасываются, разбивка по столбцам и повтор заголовков — там же.

---

### Спринт 7 (недели 14–15): Пагинация + продвинутый PDF

**Цель:** превратить одностраничный экспорт в полноценный документ — пагинация с повторяющимися заголовками, векторные диаграммы, ссылки и закладки.

**Задачи:**

1. `pdf/src/pagination.rs`:
   - разбиение по строкам и столбцам;
   - `repeat_header_rows`, `repeat_first_columns`;
   - `avoid_row_break`, `orphan_rows`, `widow_rows`;
   - `fit_to_width`, `fit_to_height`.
2. Диаграммы → векторные PDF-примитивы (из DisplayList).
3. Изображения → XObject с обрезкой.
4. Гиперссылки → `/Link`.
5. Комментарии → `/Text`.
6. Закладки → outline.
7. Водяной знак, header/footer.
8. PDF/UA теги (опц.).
9. Сжатие `flate2`.
10. Batch-экспорт всех листов.

**DoD:**
- ✅ 500 страниц < 3 с.
- ✅ Заголовки на каждой странице.
- ✅ Диаграммы векторные.
- ✅ Гиперссылки кликабельны.
- ✅ Все edge cases покрыты.
- ✅ PDF открывается в 3 браузерах (Playwright).

---

### Спринт 8 (недели 16–17): Парсинг DOCX + fuzz

**Цель:** разобрать DOCX в нормализованную модель, пригодную для layout в Спринте 9, с fuzz-инфраструктурой с первого дня, дифференциальной проверкой против mammoth.js и зафиксированными ADR по каскаду стилей, лимитам ZIP, политике ошибок и стабильным идентификаторам узлов.

**Definition of Ready (до старта):**

- ADR 0013, 0014, 0015, 0016, 0017, 0019, 0020 — приняты и лежат в `docs/adr/`.
- `cargo-fuzz`, `cargo-llvm-cov`, `cargo-deny`, `cargo-audit` — в CI.
- `mammoth` — dev-зависимость.
- `scripts/diff-mammoth.ts` — заготовка.
- `test-fixtures/docx/` — 75+ новых фикстур (базовые, стили, нумерация, таблицы, изображения, mc:AlternateContent, track changes, поля, RTL, CJK, колонтитулы, сноски, комментарии, битые, большие).

**Задачи:**

1. **Fuzz-инфраструктура — первым делом:**
   - `crates/fuzz/` — крейт, `cargo-fuzz`;
   - цели: `core_zip`, `core_xml_reader`, `core_rels`, `docx_document`, `docx_styles`, `docx_numbering`, `docx_rels`, `xlsx_worksheet`;
   - corpus: 100 фикстур + 20 реальных документов;
   - `fuzz.yml`: ночной прогон 15 мин на цель;
   - проверки: падение, OOM, зависание > 30 с, превышение лимитов ZIP.
2. **Модель** (см. §3 `docs/sprint-8/plan.md`):
   - `Document`, `Body`, `BlockItem`, `Paragraph`, `Inline`, `Run`, `RunContent`, `InlineOrAnchor`, `Anchor`, `Table`, `Row`, `Cell`, `Section`, `StyleTable`, `NumberingTable`, `Settings`;
   - `RawPPr`, `RawRPr` — сырые свойства, не нормализованные;
   - `Toggle` — тринстейт (`On`/`Off`/`Inherit`);
   - `NodeId(u64)` — на всех узлах, присваивается монотонным аллокатором при парсинге;
   - `#[derive(Serialize, Deserialize)]` на модели;
   - round-trip `Model → JSON → Model` lossless.
3. **`core`:**
   - `OoxmlArchive::open_with_limits` — лимиты из ADR-0015 (per-part 64 МиБ, per-archive 256 МиБ, ratio ≤ 200:1, запрет path traversal);
   - `OoxmlArchive::rels_for(part_name)` — rels конкретной части;
   - `XmlReader` — namespace-aware, BOM, DTD, `mc:AlternateContent`;
   - `NodeIdAllocator`.
4. **`docx`:**
   - `document.rs`, `styles.rs`, `numbering.rs`, `settings.rs`, `rels.rs`, `footnotes.rs`, `comments.rs`;
   - полный разбор `w:body`, `w:p`, `w:r`, `w:tbl`, `w:sectPr`, `w:drawing`, `w:hyperlink`, `w:bookmarkStart/End`, `w:fldSimple`, `w:instrText`, `w:fldChar`, `w:ins`, `w:del`;
   - `styles.rs` — `docDefaults`, paragraph/character/table/numbering, `basedOn`, `next`, `link`, `default`;
   - `numbering.rs` — `abstractNum`, `num`, `lvl`, `lvlOverride`, `startOverride`, `lvlText`, `numFmt`, `suff`, `picBullet`;
   - защита от циклического `basedOn` (глубина ≤ 32, warning);
   - resolve стилей — только валидация ссылок, полный каскад в Спринте 9 (ADR-0013).
5. **Дифференциальный тест vs mammoth:**
   - `scripts/diff-mammoth.ts`;
   - 50 фикстур через mammoth.js;
   - сравнение: плоский текст, структура абзацев, списки, таблицы, гиперссылки;
   - расхождения: классификация (`expected`/`bug`/`non-goal`);
   - порог: ≥ 95% совпадений;
   - отчёт: `docs/sprint-8/diff-report.md`.
6. **Бюджеты памяти:**
   - `test_paragraph_stays_small` — ≤ 256 байт;
   - `test_run_stays_small` — ≤ 128 байт;
   - `test_cell_stays_small` — ≤ 192 байт;
   - пиковая память на 50 МБ DOCX с изображениями ≤ 200 МиБ.
7. **Покрытие:**
   - `cargo-llvm-cov` в CI;
   - ≥ 85% для `crates/docx`, ≥ 80% для нового кода `crates/core`;
   - gate в CI.

**DoD:**

- ✅ 100/100 фикстур парсятся без фатальных ошибок.
- ✅ 50 из них — дифференциальный тест vs mammoth ≥ 95%; расхождения задокументированы в `docs/sprint-8/diff-report.md`.
- ✅ 50 МБ DOCX < 1,5 с native на профиле: 100k абзацев, 500 таблиц, 50 изображений, без вложений (`docs/sprint-8/bench-profile.md`).
- ✅ Покрытие ≥ 85% для `crates/docx`, ≥ 80% для нового кода `crates/core` — gate в CI.
- ✅ Fuzz 24 ч без падений, OOM, зависаний > 30 с. OSS-Fuzz подключён или обоснован отказ (`docs/sprint-8/fuzz-report.md`).
- ✅ ZIP-лимиты из ADR-0015 покрыты тестами: zip bomb, zip slip, битый central directory, превышение ratio.
- ✅ Политика ошибок из ADR-0016 покрыта тестами: битый rels, циклический `basedOn`, отсутствующий `abstractNum`, отсутствующий `style_ref` — warning, не падение.
- ✅ ADR 0013–0020 в `docs/adr/`.
- ✅ `cargo-deny` + `cargo-audit` — чисто.
- ✅ Модель расширена: `Break`, `Tab`, `Symbol`, `Field`, `Anchor`, `Unknown`, `TableLook`, `Section`, `RawPPr`, `RawRPr`, `NodeId`.
- ✅ `Inline` — enum, не «run с флагами».
- ✅ `Toggle` — тринстейт, покрыт тестами.
- ✅ `NodeId` детерминирован, не выставляется в публичный API.
- ✅ Round-trip `Model → JSON → Model` lossless на 100 фикстурах.
- ✅ Writer в DOCX не реализуется (ADR-0020).

**Non-goals:**

- Strict OOXML — только Transitional (ADR-0017).
- Track changes — парсятся как `Unknown`.
- VML — только `mc:Choice`, `mc:Fallback` игнорируется.
- TOC — поле парсится, содержимое не генерируется.
- Уравнения OMML, SmartArt, embedded OLE — `Unknown`.
- Encrypted DOCX, `.docm`, `.doc` — ошибка.
- Writer в DOCX — ADR-0020.
- Command API, undo/redo, diff/patch — ADR-0018.
- Резолвинг каскада стилей — Спринт 9.
- Layout, pagination — Спринт 9.

**Бюджеты:**

| Метрика | Бюджет |
|---|---|
| `openDocx` 50 МБ (native) | < 1,5 с |
| `openDocx` 50 МБ (wasm) | < 4 с |
| Память на абзац | ≤ 256 байт |
| Память на run | ≤ 128 байт |
| Память на ячейку таблицы | ≤ 192 байт |
| Пиковая память на 50 МБ DOCX | ≤ 200 МиБ |
| Покрытие `crates/docx` | ≥ 85% |
| Покрытие нового кода `crates/core` | ≥ 80% |
| Fuzz | 24 ч без падений |
| Differential vs mammoth | ≥ 95% |

**Риски:**

| Риск | Митигация |
|---|---|
| Модель недостаточна для Спринта 9 | ADR-0013 фиксирует каскад; `RawPPr`/`RawRPr` дают манёвр |
| `numbering.xml` сложнее ожидаемого | 5 фикстур; R2 фокусируется в день 5 |
| Fuzz найдёт падения в `quick-xml` | upstream fix или workaround |
| Differential < 95% | классификация расхождений; часть — `expected` |
| ZIP-лимиты слишком строгие | конфигурируемы; override через API |
| Не хватит 2 недель | резерв 2 дня; вынос `footnotes`/`comments` в Спринт 9 при перерасходе |
| `mc:AlternateContent` — неожиданные ветки | 5 фикстур; политика `Unknown` |

**Артефакты:** см. `docs/sprint-8/plan.md`.

---

### Спринт 9 (недели 18–19): Layout + Pagination DOCX

**Цель:** разложить поток DOCX по страницам, опираясь на общие с XLSX метрики текста: перенос строк, таблицы, floats, пагинация.

**Задачи:**

1. `docx/src/layout/`:
   - `engine.rs`;
   - `line_break.rs` — **использует `render::text_measure`**;
   - `hyphenation.rs` (опц.);
   - `pagination.rs`;
   - `float.rs`;
   - `tables.rs`.
2. Интеграция с `render::text_measure`.
3. `PageLayout { pages: Vec<Page> }`.
4. Тесты: 50 фикстур, сравнение числа страниц с MS Word ±1.

**DoD:**
- ✅ Число страниц ±1 vs MS Word на 50 фикстурах.
- ✅ SSIM ≥ 0.95 для первой страницы.
- ✅ Pagination 100 страниц < 500 мс.
- ✅ Переносы совпадают с canvas-рендером (единая логика).

---

### Спринт 10 (недели 20–21): Рендер + PDF DOCX

**Цель:** показать DOCX в воркере и выгрузить его в PDF, Markdown и HTML.

**Задачи:**

1. `docx/src/render.rs` — `render_page`.
2. `pdf/src/docx.rs` — `export_docx_to_pdf`.
3. Экспорт MD / HTML / plain text.
4. `examples/viewer-docx/`.
5. Интеграция с воркером.

**DoD:**
- ✅ Viewer DOCX на 100 фикстурах.
- ✅ PDF 200 страниц < 2 с.
- ✅ Markdown корректен для 50 фикстур.
- ✅ HTML со стилями.
- ✅ Работает в 3 браузерах.

---

### Спринт 11 (недели 22–23): SAB + CLI

**Цель:** уравнять путь с SAB и путь без него и дать batch-конвертацию из командной строки.

**Задачи:**

1. **Оба пути:**
   - SAB-путь для DisplayList (при COOP/COEP);
   - **first-class путь без SAB** — тот же API, `ArrayBuffer` + transfer;
   - двойная буферизация (2 SAB);
   - тесты обоих путей в Playwright.
2. Transferable везде.
3. `WorkerPool` для batch.
4. `@doc-converter/cli`:
   - `convert input.docx -o out.pdf`;
   - `batch ./in/*.xlsx --out ./pdf --workers 4`;
   - прогресс-бар, сжатие, опции.
5. Бенчмарки vs `exceljs`, `xlsx`, `mammoth`.
6. **README:** требование COOP/COEP для SAB при embed; что даёт SAB.

**DoD:**
- ✅ SAB работает при COOP/COEP.
- ✅ **Путь без SAB работает без деградации функциональности**.
- ✅ CLI конвертирует 100 файлов < 30 с.
- ✅ Отчёт ×10–300 vs JS-аналоги.
- ✅ Бюджеты производительности зафиксированы для обоих путей.
- ✅ COOP/COEP задокументированы в README.

---

### Спринт 12 (недели 24–25): Релиз v1.0

**Цель:** довести проект до публикации — аудит, документация, примеры в трёх браузерах, npm и crates.io.

**Задачи:**

1. Аудит: `cargo-deny`, `cargo-audit`, `npm audit`, `size-limit`.
2. TypeDoc + rustdoc.
3. Cookbook (40+ рецептов).
4. Migration guides: `migration-from-mammoth.md`, `-from-exceljs.md`.
5. Демо-приложение.
6. Видео-демо: scroll 1M ячеек 60 FPS.
7. Публикация в npm + crates.io.
8. GitHub Release + changelog.

**DoD:**
- ✅ Все пакеты v1.0.0 опубликованы.
- ✅ Документация на docs.doc-converter.dev.
- ✅ Примеры в 3 браузерах (Playwright Chromium + Firefox + WebKit).
- ✅ Покрытие ≥ 85% / 80%.
- ✅ Бюджеты размера соблюдены.
- ✅ Все ADR в `docs/adr/`.

---

## 7. Сводная таблица

| Недели | Фаза | Артефакт | Состояние |
|---|---|---|---|
| 0 | Подготовка | Окружение | сделано |
| 1–2 | Фундамент | `core@0.1.0`, CI | сделано |
| 3–4 | Фундамент | RPC + Worker | сделано |
| 5–6 | XLSX | Парсер | сделано |
| 7–8 | XLSX | DisplayList + рендер | сделано, кроме стресс-тестов |
| 9–10 | XLSX | Стили, границы, изображения | сделано |
| **11** | **XLSX** | **Метрики текста + граница `render`** | **сделано (5.5), кроме SSIM** |
| 12–13 | PDF | Базовый | план v2 |
| 14–15 | PDF | Пагинация | план v2 |
| 16–17 | DOCX | Парсинг + fuzz | сделано волнами 1–3: парсер, 97 фикстур, fuzz (8 целей), differential vs mammoth, бюджеты времени и памяти, покрытие с гейтом в CI |
| 18–19 | DOCX | Layout | план v2 |
| 20–21 | DOCX | Рендер + PDF | план v2 |
| 22–23 | Полировка | SAB (оба пути) + CLI | план v2 |
| 24–25 | Релиз | v1.0.0 | план v2 |

**Сдвиг:** +1 неделя от v1 (25 недель вместо 24). Обоснование — закрытие
трёх системных рисков без рефакторинга на Спринте 9.

---

## 8. Ключевые вехи

| Веха | Неделя | Критерий | Состояние |
|---|---|---|---|
| M1: Workspace + CI | 2 | Зелёный CI, `core@0.1.0` | достигнута |
| M2: RPC + Worker | 4 | Round-trip < 1 мс | достигнута, кроме замера |
| M3: XLSX парсер | 6 | 100% фикстур | достигнута |
| M4: XLSX viewer | 8 | 60 FPS scroll | достигнута, кроме 60 FPS на 1M |
| M5: XLSX полный | 11 | SSIM ≥ 0.95 (закрывается в Спринте 5.5) | метрики, граница, диаграммы; SSIM переходит в Спринт 6 |
| M6: PDF базовый | 13 | 3 браузера | не начата |
| M7: PDF полный | 15 | 500 страниц < 3 с | не начата |
| M8: DOCX парсер | 17 | 100% фикстур | 97/97 коммитируемых фикстур (94 без фатальных ошибок + 3 по замыслу); differential vs mammoth и 24-ч fuzz — впереди |
| M9: DOCX layout | 19 | ±1 страница vs MS Word | не начата |
| M10: DOCX viewer | 21 | 200 страниц < 2 с | не начата |
| M11: Оптимизация | 23 | Оба пути SAB + CLI | SAB есть, CLI нет |
| M12: v1.0.0 | 25 | Публикация | не начата |

---

## 9. Бенчмарки и бюджеты

### Производительность

| Операция | Цель (native) | Цель (WASM) | Факт |
|---|---|---|---|
| `openDocx` 50 МБ | < 1.5 с | < 4 с | **356 мс (native)**; wasm не измерен |
| `openXlsx` 1M ячеек | < 2 с | < 5 с | **0,38 с** |
| `renderPage` DOCX A4 | < 30 мс | < 30 мс | — |
| `measure_text` 100 симв. | < 1 мкс | < 5 мкс | **278 нс (native)** |
| `buildDisplayList` 100k | < 100 мс | < 100 мс | 1–3 мс на экран |
| `buildDisplayList` 10k ячеек с условным форматированием | — | — | база ≈5,2–5,6 мс; шкала ≈9,2–9,6; гистограмма ≈7,1–7,4; значки **8,34** (было 14,4) |
| `paintDisplayList` 1200×800 | < 16 мс | < 16 мс | 0–5 мс |
| `hitTest` | < 2 мс | < 2 мс | **48 мкс на 5000×50** |
| `exportPdf` 100 страниц | — | < 1.5 с | **≈ 60 мс (экстраполяция)** |
| `exportPdf` 500 страниц | — | < 3 с | **≈ 300 мс (экстраполяция)** |
| Память модели | ≤ 48 байт/ячейку | — | **48 байт** |
| Память узлов DOCX (абзац / run / ячейка) | ≤ 256 / 128 / 192 Б | — | **88 / 64 / 128 Б** |
| Пиковая память `openDocx` 50 МБ с изображениями | ≤ 200 МиБ | — | **156,78 МиБ** |

Замер условного форматирования (`cargo bench -p doc-converter-xlsx --bench
conditional`) — худший случай: окно покрывает весь лист, в кадр попадают все
10k ячеек. В реальном кадре на экран попадают сотни ячеек. Накладные расходы
правил: шкала ≈ 4 мс, гистограмма ≈ 2 мс, значки ≈ 3 мс — все укладываются
в бюджет «< 5 мс». Спринт 5 ускорил значки: форма набора определялась поиском
подстроки на каждом вызове и съедала ~47% времени кадра — классификация
переехала в разбор правила.

`exportPdf` измерен criterion (`crates/pdf/benches/pdf.rs`): 13 страниц —
7,95 мс (медиана), 1000 ячеек — 1,14–1,18 мс. Для 100 и 500 страниц замеров
нет: линейная экстраполяция от 0,61 мс на страницу даёт ≈ 60 и ≈ 300 мс.
Память экспорта линейна — ≈ 0,44 МиБ на страницу
(`docs/sprint-6/memory-report.md`).

**DOCX (Спринт 8).** `openDocx` на плотном профиле (`profile_50mib.docx`:
100 000 абзацев тела, 500 таблиц, 50 изображений, вход 55,5 МиБ) — медиана
**356 мс** при бюджете 1500 мс (`cargo bench -p doc-converter-docx`, нативный
прогон). Wasm-нога не измерена: wasm-бенч-каркаса в репозитории нет, да и самого
`openDocx` в `crates/wasm` пока нет — DOCX-API появится вместе с просмотрщиком.
Это зафиксированное отклонение, а не забытое число.

Бюджеты памяти DOCX выполнены: 88 / 64 / 128 Б на абзац / run / ячейку при
256 / 128 / 192, пик RSS на `memory_50mib` (50 МБ с изображениями) — **156,78
МиБ** при бюджете 200 МиБ (`docs/sprint-8/memory-report.md`). Отклонение — на
плотном профиле пик **874,6 МиБ**: бюджет 200 МиБ недостижим без потокового
разбора, который в спринт 8 не входит. Поэтому гейт памяти в CI стоит на
фикстуре «50 МБ с изображениями» (`memory_50mib`), а плотный профиль идёт
информационной строкой без порога; потоковый разбор — задел Спринта 9.

**Бюджеты для пути без SAB (замер в Спринте 11):**
- RPC round-trip < 3 мс (vs < 1 мс с SAB)
- `buildDisplayList` — без деградации (transferables)

Замеры — Apple Silicon, релизная сборка, синтетическая книга (см. `cargo bench
-p doc-converter-xlsx` и `examples/viewer-xlsx/e2e/`).

### Бюджеты размера (gzip)

| Пакет | Бюджет v1 | Бюджет v2 | Факт |
|---|---|---|---|
| `@doc-converter/core` | 200 КБ | **30 КБ** | 2,3 КБ |
| `@doc-converter/docx` | 450 КБ | 450 КБ | пакета нет |
| `@doc-converter/xlsx` | 500 КБ | 500 КБ | пакета нет |
| `@doc-converter/react` | 25 КБ | 25 КБ | пакета нет |
| Worker bundle | 600 КБ | 600 КБ | 5,4 КБ JS + 275 КБ wasm (просмотрщик); модуль экспорта PDF 1,76 МБ грузится по требованию |

`size-limit` меряет JS-бандлы и отдельной строкой `wasm (viewer)` — основной
wasm просмотрщика (бюджет 300 КБ gzip); модуль экспорта PDF бюджета не имеет:
приезжает отдельным ассетом и грузится по требованию.

CI-гейт: `pnpm size-limit` (шаг на Node 20) блокирует merge при превышении
бюджета.

---

## 10. Тесты

| Уровень | Инструмент | Спринт ввода | Состояние |
|---|---|---|---|
| Unit Rust | `cargo test` | 1 | 889 тестов |
| Property | `proptest` | 1 | `cellref`, `strings`, `model` |
| **Snapshot** | **`insta`** | **5.5** | **20 фикстур** |
| **Fuzz** | **`cargo-fuzz`** | **8** | **введён: `crates/fuzz/`, 8 целей; ночной прогон и ручные длинные (`workflow_dispatch`: `target`/`minutes`, до 330 мин на цель); корпус кэшируется между прогонами** |
| WASM | `wasm-bindgen-test` | 5.5 | painter пока не покрыт |
| **E2E** | **Playwright** | 6 | **5 тестов, только Chromium, по пикселям; расширяется на 3 браузера** |
| Stress | 1M ячеек, 30 мин scroll | 5.5 | вводится |
| Differential | mammoth (DOCX), exceljs (XLSX) | 8 / 4 | 110 фикстур XLSX; DOCX — 76 кандидатов: 74 совпадения, 2 non-goal, 0 неклассифицированных (`crates/docx/tests/differential.rs`) |
| Размер | `size-limit` | 5.5 | 3 бюджета, включая `wasm (viewer)` |
| Производительность | `criterion` | 3 | открытие книги |
| **Покрытие** | **`cargo-llvm-cov` + TS** | **8** | **джоба `coverage` в CI: `crates/docx` ≥ 85 % строк (факт 92,75 %), `crates/core` ≥ 80 % (факт 97,68 %); TS-гейта нет** |

Покрытие: в CI есть джоба `coverage` — два прогона `cargo llvm-cov` с
`--fail-under-lines`, по одному на крейт (общий прогон двух сюит смешал бы
числа): ≥ 85 % строк для `crates/docx` (факт **92,75 %**) и ≥ 80 % для нового
кода `crates/core` (факт **97,68 %**). Вне гейта осознанно: `crates/fuzz` и
`vendor/printpdf` — отдельные workspace'ы, `crates/wasm` на нативном таргете
пуст, у `render`/`xlsx`/`pdf` порога в §9 нет. TS-порог (≥ 80 %) гейтом не
закрыт: требование `CLAUDE.md` остаётся без проверки в CI.

`cargo-fuzz` введён: `crates/fuzz/` — отдельный workspace (nightly и
sanitizer не задевают stable), 8 целей, ночной прогон в
`.github/workflows/fuzz.yml` (15 мин на цель, на PR — 60 с). Плюс ручные
длинные прогоны: `workflow_dispatch` с параметрами `target` и `minutes`, до
330 минут на цель (лимит job'а GitHub — 6 ч), корпус ездит между прогонами
через кэш, чтобы длинная сессия не начиналась с нуля.

Differential vs mammoth введён: 76 кандидатов из 97 фикстур, **74 совпадения**,
2 non-goal, **0 неклассифицированных расхождений**; гейт —
`crates/docx/tests/differential.rs` (`match / (кандидаты − non-goal) ≥ 0.95`).
DOCX: 97 коммитируемых фикстур, ещё 5 «больших» (11–15 МиБ) генерируются по
требованию в gitignored `target/fixtures/docx-large/`. TS: 28 тестов vitest
в `@doc-converter/core`.

---

## 11. Риски и митигация

### Три системных риска и их закрытие

#### Риск A. Метрики текста — фундамент, а не задача

**Суть.** `FontRegistry` не подключён в путь рисования. Ширина оценивается по
таблице Calibri. Для чисел работает, для произвольного текста — нет.

**Что зависит от метрик:**
- Перенос по словам (DOCX layout, Спринт 9)
- `#####` для узких колонок (XLSX, перенесено в Спринт 5.5)
- PDF-экспорт: `pdf/src/text.rs` (Спринт 6)
- Hit-testing по тексту (выделение, курсор)
- Пагинация (Спринт 7)

**Закрытие:** Спринт 5.5. ADR-0002 (шрифты) — до Спринта 5.5.
**Состояние:** ✅ закрыт в Спринте 5.5 — `FontRegistry` на `skrifa` подключён в
`xlsx::paint`, `estimate_width` удалён, `render::text_measure` даёт единые
ширины для canvas и будущего PDF.

#### Риск B. Граница `crates/render`

**Суть.** В `render` живут `DisplayList` + painter, но `viewport.rs`,
`canvas.rs`, `hit_test.rs` не появились — их работу делают `xlsx::layout` и
`xlsx::paint`. При появлении DOCX (Спринт 9) граница сломается.

**Правило границы (ADR-0003):**

| В `render` | В `xlsx` / `docx` |
|---|---|
| `DisplayList`, `DrawCommand` | Специфика ячеек, параграфов, runs |
| Painter (OffscreenCanvas 2D) | Раскладка листа / потока текста |
| `FontRegistry`, метрики | `cellAtPoint` семантически (через геометрию) |
| Геометрия: `Rect`, `Point`, clip | Модель: `Cell`, `Merge`, `Para` |
| Viewport: скролл, видимый диапазон | Семантика скролла (freeze panes) |
| `hitTest` по геометрии DisplayList | Маппинг геометрия → ячейка/слово |

**Закрытие:** Спринт 5.5 — `render::{viewport, canvas, hit_test}` выносятся как
чистая геометрия. `xlsx::layout` и `xlsx::paint` остаются как адаптеры.

**Состояние:** ✅ закрыт в Спринте 5.5 — модули `viewport`, `canvas`, `hit_test`
существуют, `Viewport` переехал в `render`, `xlsx::paint::hit_test` остался
раскладочным адаптером (DisplayList не несёт идентичности ячеек).

#### Риск C. SAB и COOP/COEP

**Суть.** В v1 записано «SAB работает при COOP/COEP, fallback без них».
Фактически: embed-сценарий (iframe в чужом сайте, extension, Electron, webview)
**не контролирует заголовки**, SAB отваливается. Это не edge case.

**Переформулировка:**

> **SAB — ускорение, не обязательное условие.**
> Все пути (RPC, DisplayList, PDF/PNG transfer) должны работать без SAB
> с приемлемой производительностью. SAB даёт ×N, а не «включает».

**Требования:**
1. Путь без SAB — first-class, не fallback.
2. COOP/COEP — ответственность потребителя при embed; задокументировано в README.
3. Тесты гоняют **оба пути**: с заголовками и без.
4. Бюджеты производительности фиксируются для обоих путей отдельно.

**Закрытие:** Спринт 11 — оба пути, оба в CI.

### Принятые ADR

| ADR | Тема | Решение | Файл |
|---|---|---|---|
| 0002 | Парсинг шрифтов | `skrifa`; `ttf-parser` unmaintained (RUSTSEC-2026-0192) | `docs/adr/0002-font-parsing.md` |
| 0003 | Граница `crates/render` | Геометрия, viewport, hit-test и метрики — в `render`; модель формата — в `xlsx`/`docx` | `docs/adr/0003-render-boundary.md` |
| 0004 | SAB как ускорение | Путь без SAB — first-class, оба пути в CI | `docs/adr/0004-sab-acceleration.md` |
| 0005 | Метрики текста | `skrifa` + таблицы шрифта + LRU-кэш; API `render::text_measure` | `docs/adr/0005-text-metrics.md` |
| 0006 | Раскладка DOCX и перенос текста | чужая работа, в план v2 не входит | `docs/adr/0006-docx-layout-and-text-wrapping.md` |
| 0013 | Каскад стилей DOCX | Каскад резолвится на layout (Спринт 9), модель несёт `RawPPr`/`RawRPr` + ссылки; полный порядок из ECMA-376 §17.7 | `docs/adr/0013-docx-style-cascade.md` |
| 0014 | `mc:AlternateContent` | Берём `mc:Choice` (первая поддержанная ветка), `mc:Fallback` игнорируем; неподдержанное — `Unknown` с XML | `docs/adr/0014-alternate-content.md` |
| 0015 | Лимиты ZIP для OOXML | Per-part 64 МиБ, per-archive 256 МиБ, ratio ≤ 200:1, запрет path traversal | `docs/adr/0015-ooxml-zip-limits.md` |
| 0016 | Политика ошибок DOCX | Recover-and-continue: `Vec<ParseWarning>`, частичная модель; фатально только ZIP и отсутствие `document.xml` | `docs/adr/0016-docx-error-policy.md` |
| 0017 | Strict vs Transitional OOXML | Transitional — единственный поддерживаемый; Strict — non-goal v1 | `docs/adr/0017-strict-vs-transitional.md` |
| 0018 | Границы редактирования в v1 | v1 — read-only; модель только в Rust/WASM; Jotai — только view state; `NodeId` без editing-инфраструктуры | `docs/adr/0018-editing-boundaries.md` |
| 0019 | NodeId | `NodeId(u64)`, присваивается при парсинге, детерминирован, не переживает переоткрытие, не публичный API | `docs/adr/0019-node-id.md` |
| 0020 | Границы парсера и writer | Парсер = десериализатор; `serde` в JSON — да; writer в DOCX — non-goal v1 | `docs/adr/0020-parser-writer-boundary.md` |

ADR 0002–0005 приняты 05.10.2026 (0002 пересмотрен на `skrifa`). Реализация
0002, 0003 и 0005 — Спринт 5.5, 0004 — Спринт 11. ADR-0006 — чужая работа про
раскладку DOCX, к нашему плану не относится.

### Таблица рисков

| Риск | Митигация | Спринт |
|---|---|---|
| OffscreenCanvas в Safari < 16.4 | Fallback на main-thread рендер | 2 |
| **SAB требует COOP/COEP при embed** | **First-class путь без SAB, README** | **11** |
| **Метрики текста — фундамент для 4 спринтов** | **Спринт 5.5, ADR-0002/0005** | **5.5** |
| **Граница `render` сломается при DOCX** | **ADR-0003, вынос модулей** | **5.5** |
| Subsetting ломает CJK | Тесты на 20 шрифтах, fallback на full embed | 6 |
| Расхождение с MS Word | Differential testing, SSIM в CI | 6 |
| Большие файлы (>500 МБ) | Streaming-парсинг, lazy-read частей | 8 |
| Регрессия производительности | `github-action-benchmark`, гейт 10% | 5.5 |
| Утечки памяти в воркере | 30-мин stress, `performance.memory` | 5.5 |
| PDF не открывается в старых viewer | `qpdf --check`, PDF/A валидация | 7 |
| Fuzz-находки в XML-парсерах | `cargo-fuzz` с Спринта 8, фиксы в первую очередь | 8 |

---

## 12. Definition of Done v1.0

- [ ] DOCX и XLSX открывают 100% фикстур без потери данных.
- [ ] SSIM ≥ 0.95 vs MS Office на 50 эталонах.
- [ ] **Единая логика метрик текста** для canvas и PDF (ADR-0005).
- [ ] **Граница `render` зафиксирована** (ADR-0003).
- [ ] PDF векторный, шрифты встроены, subsetting работает.
- [ ] Все рендер-операции в воркере, main FPS ≥ 60.
- [ ] `OffscreenCanvas` передан один раз.
- [ ] PDF/PNG через Transferables, zero-copy.
- [ ] **SAB работает при COOP/COEP; путь без SAB — first-class** (ADR-0004).
- [ ] **COOP/COEP задокументированы в README**.
- [ ] Все бюджеты размера и производительности соблюдены.
- [ ] Покрытие ≥ 85% / 80% (замер).
- [ ] Fuzz 24 ч без находок (замер).
- [ ] Аудит безопасности чистый.
- [ ] Chrome 90+, Firefox 96+, Safari 16.4+; fallback везде.
- [ ] **Примеры работают в 3 браузерах (Playwright Chromium + Firefox + WebKit)**.
- [ ] Модель DOCX несёт `RawPPr`/`RawRPr` + ссылки; каскад резолвится в layout (ADR-0013).
- [ ] `NodeId` на всех узлах модели, детерминирован, не публичный (ADR-0019).
- [ ] ZIP-лимиты из ADR-0015 покрыты тестами.
- [ ] Политика ошибок из ADR-0016 покрыта тестами.
- [ ] Differential vs mammoth ≥ 95% на 50 фикстурах DOCX.
- [ ] Round-trip `Model → JSON → Model` lossless.
- [ ] Writer в DOCX не реализован (ADR-0020).
- [ ] Все ADR в `docs/adr/`.
- [ ] Опубликовано в npm + crates.io под Apache-2.0.

---

## 13. Правила кода

- Rust: `rustfmt`, `clippy::pedantic -D warnings`.
- `#![forbid(unsafe_code)]` вне WASM-слоя.
- TS: `strict`, `noUncheckedIndexedAccess`, без `any`.
- JSDoc на всех экспортируемых функциях.
- Conventional Commits, trunk-based.
- Feature-flags для незавершённого.
- 2 approver'а на публичный API.
- Никаких `postMessage` без учёта transferables.
- Все тяжёлые вычисления — в воркере.
- Новые WASM-функции покрываются `wasm-bindgen-test`.
- Лицензия Apache-2.0, CLA.

---

## 14. Чек-лист старта

- [ ] Команда: 2 Rust, 1 TS, 1 QA, 1 DevOps.
- [ ] Публичный репозиторий на GitHub.
- [ ] GitHub Projects с 13 спринтами.
- [ ] CI-матрица: native + wasm + 3 браузера.
- [ ] Линтеры и pre-commit.
- [ ] Согласованы бюджеты v2.
- [x] **ADR-0002, 0003, 0005 — приняты до Спринта 5.5 и реализованы в нём.**
- [ ] Kick-off с разбором roadmap v2.
- [ ] `docs/architecture.md` с C4.
- [ ] Первый PR: workspace + CI + пустой `core@0.1.0`.

---

## 🎓 Обучение команды

| Перед фазой | Тема | Длительность |
|---|---|---|
| 1 | Rust + WASM basics | 2 дня |
| 2 | OOXML + Canvas API | 2 дня |
| 3 | PDF спецификация | 2 дня |
| 4 | WordprocessingML + line breaking | 3 дня |
| 5 | SAB, COOP/COEP, Worker Threads | 1 день |

Воркшопы каждые 2 недели: 1 ч разбор кода + 30 мин демо.

---

**Итог: 25 недель, 13 спринтов, 12 вех, 6 пакетов, 3 формата экспорта, 3 браузера.**

_Конец ROADMAP.md_
