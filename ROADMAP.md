# 🗺️ ROADMAP: doc-converter

> Полный план разработки проекта **doc-converter** — движков просмотра и конвертации
> DOCX/XLSX в браузере и Node.js на Rust/WASM + Web Worker + OffscreenCanvas + PDF.
>
> **Длительность:** 24 недели (12 спринтов по 2 недели).
> **Команда:** 2 Rust-инженера, 1 TS-инженер, 1 QA, 1 DevOps.
> **Лицензия:** Apache-2.0.
> **Релиз v1.0:** неделя 24.
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
| Zero-copy через SAB и Transferables | Real-time синхронизация |

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
4. Zero-copy: `SharedArrayBuffer` для DisplayList, `Transferable` для PDF/PNG.
5. Коалесцирование кадров через `requestAnimationFrame` в воркере.
6. Fallback на main-thread рендер при отсутствии OffscreenCanvas.

### 3.4 COOP/COEP заголовки

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Нужны для `SharedArrayBuffer`. При отсутствии — fallback на `ArrayBuffer` + transfer.

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
- `ttf-parser` снят: в коде не использовался, а с 2026 года помечен
  `unmaintained` (RUSTSEC-2026-0192). Возвращать его (с исключением в аудите) или
  брать рекомендованный `skrifa` — решение Фазы 4, см. `docs/adr/0002-font-parsing.md`.
- `printpdf` 0.7 → 0.8: версия из манифеста, 0.7 в дереве не было.

### Frontend

| Технология | Назначение |
|---|---|
| TypeScript 5.5 | Типы |
| tsup 8 | ESM + CJS + d.ts |
| Vite 5 | Dev и сборка примеров |
| React 18 | Обёртки |
| OffscreenCanvas | Рендер в воркере |
| SharedArrayBuffer | Zero-copy DisplayList |
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
- `rustybuzz`, `unicode-bidi`, `unicode-linebreak`; разбор шрифтов — `skrifa` или
  `ttf-parser` (`docs/adr/0002-font-parsing.md`).
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

> **Состояние на 05.10.2026.** Закрыты Фаза 1 (workspace, CI, core-крейты),
> Фаза 2 (RPC Main↔Worker, OffscreenCanvas, SAB ring) и Спринты 3–4: XLSX
> разбирается целиком и **показывается в браузере**.
>
> **Что работает.** `crates/xlsx` разбирает книгу `open()` — каталог листов,
> общие строки, стили, форматы чисел, геометрию, объединения, гиперссылки.
> `paint::build` раскладывает лист в пикселях и собирает `DisplayList`, который
> painter рисует на `OffscreenCanvas`. `createXlsxViewer` из
> `@doc-converter/core` даёт прокрутку, зум, переключение листов и определение
> ячейки под курсором; пример — `examples/viewer-xlsx/`.
>
> **Чем подтверждено.** 181 нативный тест; 100 фикстур, собранных exceljs, и
> дифф-тест нашего разбора против эталона того же exceljs — в обе стороны;
> пять сквозных тестов в настоящем Chromium, которые декодируют PNG из воркера
> и смотрят на пиксели; criterion-замеры открытия книги.
>
> **Замеры** (Apple Silicon, релиз): миллион ячеек разбирается за **0,38 с** при
> бюджете 2 с; модель занимает **48 байт на ячейку** (≈45 МБ на миллион);
> экран таблицы собирается за **1–3 мс** и рисуется за **0–5 мс**; wasm весит
> **168 КБ gzip** при бюджете воркера 600 КБ.
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
> 30 минут стабильности) не гонялись; тёмная тема и `theme1.xml` — не начаты.
>
> **Дальше — Спринт 5.** По порядку пользы: тема оформления (`theme1.xml`),
> без которой цвета `theme` рисуются чёрным; границы ячеек (в модели
> разбираются, в кадр не попадают); метрики текста, чтобы появились перенос и
> «#####» для произвольного текста; условное форматирование; изображения и
> диаграммы.

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
- ✅ `wasm-pack build` < 200 KB gzip — 168 КБ.
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

### Спринт 5 (недели 9–10): Стили, диаграммы, изображения

**Цель:** полноценный визуал.

**Задачи:**
1. Conditional formatting → финальный `Style`.
2. Диаграммы: парсинг `chart*.xml`, рендер в DisplayList.
3. Изображения: `ImageRegistry`, `DrawCommand::Image`, anchors.
4. Гиперссылки: подчёркивание, обработка клика.
5. Frozen panes через `PushClip`.
6. Заголовки строк/столбцов.
7. Тёмная тема.

**DoD:**
- ✅ 5 типов диаграмм.
- ✅ SSIM ≥ 0.95 vs MS Office на 30 эталонах.
- ✅ DPR=2 без потери качества.
- ✅ Условное форматирование < 5 мс на 10k ячеек.

---

### Спринт 6 (недели 11–12): Базовый PDF

**Цель:** экспорт простого XLSX-листа в PDF.

**Задачи:**
1. `pdf/src/styles.rs` — `Color`, `BorderStyle`, `CellStyle`, `TextAlign`.
2. `pdf/src/layout.rs` — `PageSize`, `PageOrientation`, `Margins`, `PageConfig`.
3. `pdf/src/fonts.rs` — `FontRegistry`, TTF/OTF.
4. `pdf/src/text.rs` — измерение + перенос.
5. `pdf/src/border.rs` — границы, включая `Double`.
6. `pdf/src/background.rs`.
7. `pdf/src/painter.rs` — `export_xlsx_sheet`.
8. Тесты: 10×10 лист → PDF открывается.

**DoD:**
- ✅ PDF открывается в Chrome/Firefox/Adobe.
- ✅ Кириллица корректна.
- ✅ 1000 ячеек → PDF < 200 KB с subsetting.
- ✅ 10 страниц < 300 мс.

---

### Спринт 7 (недели 13–14): Пагинация + продвинутый PDF

**Задачи:**
1. `pdf/src/pagination.rs`:
   - разбиение по строкам и столбцам;
   - `repeat_header_rows`, `repeat_first_columns`;
   - `avoid_row_break`, `orphan_rows`, `widow_rows`;
   - `fit_to_width`, `fit_to_height`.
2. Диаграммы → векторные PDF-примитивы.
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

---

### Спринт 8 (недели 15–16): Парсинг DOCX

**Задачи:**
1. Модель: `Document`, `Body`, `Paragraph`, `Run`, `Text`, `Table`, `Row`, `Cell`, `Style`, `Numbering`, `Section`, `HeaderFooter`, `Image`, `Hyperlink`, `Footnote`, `Comment`, `Bookmark`.
2. Парсеры: `document.rs`, `styles.rs`, `numbering.rs`, `settings.rs`, `rels.rs`, `footnotes.rs`, `comments.rs`.
3. Resolve стилей: paragraph → character → direct.
4. Тесты: 100 фикстур → 100%.

**DoD:**
- ✅ 100/100 фикстур.
- ✅ 50 МБ DOCX < 1.5 с.
- ✅ Покрытие ≥ 85%.

---

### Спринт 9 (недели 17–18): Layout + Pagination DOCX

**Задачи:**
1. `docx/src/layout/`:
   - `engine.rs`;
   - `line_break.rs`;
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

---

### Спринт 10 (недели 19–20): Рендер + PDF DOCX

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

---

### Спринт 11 (недели 21–22): SAB + CLI

**Задачи:**
1. SAB-путь для DisplayList.
2. Двойная буферизация (2 SAB).
3. Transferable везде.
4. `WorkerPool` для batch.
5. `@doc-converter/cli`:
   - `convert input.docx -o out.pdf`;
   - `batch ./in/*.xlsx --out ./pdf --workers 4`;
   - прогресс-бар, сжатие, опции.
6. Бенчмарки vs `exceljs`, `xlsx`, `mammoth`.

**DoD:**
- ✅ SAB работает при COOP/COEP.
- ✅ Fallback без SAB.
- ✅ CLI конвертирует 100 файлов < 30 с.
- ✅ Отчёт ×10–300 vs JS-аналоги.

---

### Спринт 12 (недели 23–24): Релиз v1.0

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
- ✅ Примеры в 3 браузерах.
- ✅ Покрытие ≥ 85% / 80%.
- ✅ Бюджеты размера соблюдены.

---

## 7. Сводная таблица

| Недели | Фаза | Артефакт | Состояние |
|---|---|---|---|
| 0 | Подготовка | Окружение | сделано |
| 1–2 | Фундамент | `core@0.1.0`, CI | сделано |
| 3–4 | Фундамент | RPC + Worker | сделано |
| 5–6 | XLSX | Парсер | сделано |
| 7–8 | XLSX | DisplayList + рендер | сделано, кроме стресс-тестов |
| 9–10 | XLSX | Стили, диаграммы | не начато |
| 11–12 | PDF | Базовый | не начато |
| 13–14 | PDF | Пагинация | не начато |
| 15–16 | DOCX | Парсинг | не начато |
| 17–18 | DOCX | Layout | не начато |
| 19–20 | DOCX | Рендер + PDF | не начато |
| 21–22 | Полировка | SAB + CLI | SAB сделана в Фазе 2, CLI нет |
| 23–24 | Релиз | v1.0.0 | не начато |

---

## 8. Ключевые вехи

| Веха | Неделя | Критерий | Состояние |
|---|---|---|---|
| M1: Workspace + CI | 2 | Зелёный CI, `core@0.1.0` | достигнута |
| M2: RPC + Worker | 4 | Round-trip < 1 мс | достигнута, кроме замера |
| M3: XLSX парсер | 6 | 100% фикстур | достигнута |
| M4: XLSX viewer | 8 | 60 FPS scroll | достигнута, кроме 60 FPS на 1M |
| M5: XLSX полный | 10 | SSIM ≥ 0.95 | не начата |
| M6: PDF базовый | 12 | 3 браузера | не начата |
| M7: PDF полный | 14 | 500 страниц < 3 с | не начата |
| M8: DOCX парсер | 16 | 100% фикстур | не начата |
| M9: DOCX layout | 18 | ±1 страница vs MS Word | не начата |
| M10: DOCX viewer | 20 | 200 страниц < 2 с | не начата |
| M11: Оптимизация | 22 | SAB + CLI | SAB есть, CLI нет |
| M12: v1.0.0 | 24 | Публикация | не начата |

---

## 9. Бенчмарки и бюджеты

### Производительность

| Операция | Цель | Факт |
|---|---|---|
| `openDocx` 50 МБ | < 1.5 с native / < 4 с WASM | — |
| `openXlsx` 100 МБ / 1M ячеек | < 2 с native / < 5 с WASM | **0,38 с** native |
| `renderPage` DOCX A4 | < 30 мс | — |
| `buildDisplayList` 100k ячеек | < 100 мс | 1–3 мс на экран в Chromium |
| `paintDisplayList` 1200×800 | < 16 мс (60 FPS) | 0–5 мс в Chromium |
| `hitTest` | < 2 мс | не замерено |
| `exportPdf` 100 страниц | < 1.5 с | — |
| `exportPdf` 500 страниц | < 3 с | — |
| Память модели | ≤ 48 байт на ячейку | **48 байт** (тест `cell_stays_small`) |

Замеры — Apple Silicon, релизная сборка, синтетическая книга (см. `cargo bench
-p doc-converter-xlsx` и `examples/viewer-xlsx/e2e/`).

### Бюджеты размера (gzip)

| Пакет | Бюджет | Факт |
|---|---|---|
| `@doc-converter/core` | 200 KB | 1,9 КБ (`dist/index.js`) |
| `@doc-converter/docx` | 450 KB | пакета нет |
| `@doc-converter/xlsx` | 500 KB | пакета нет |
| `@doc-converter/react` | 25 KB | пакета нет |
| Worker bundle | 600 KB | 4,7 КБ JS + **168 КБ** wasm |

`size-limit` меряет только JS: wasm приезжает отдельным ассетом и в бюджет
воркера входит отдельной строкой.

CI-гейт: регрессия > 10% блокирует merge.

---

## 10. Тесты

| Уровень | Инструмент | Состояние |
|---|---|---|
| Unit Rust | `cargo test` | 181 тест |
| Property | `proptest` | `cellref`, `strings`, `model` |
| Snapshot | `insta` | не используется |
| Fuzz | `cargo-fuzz` (15 мин ночью) | не настроен |
| WASM | `wasm-bindgen-test` | не используется |
| E2E | Playwright | 5 тестов, только Chromium, по пикселям |
| Stress | 1M ячеек, 30 мин scroll, memory | не гонялся |
| Differential | эталон третьей реализации | 100 фикстур против exceljs |
| Размер | `size-limit` | 2 бюджета, JS |
| Производительность | `criterion` | открытие книги |

Покрытие (Rust ≥ 85%, TS ≥ 80%) не измеряется: в CI нет ни `cargo-llvm-cov`,
ни порога по TS.

---

## 11. Риски и митигация

| Риск | Митигация |
|---|---|
| OffscreenCanvas в Safari < 16.4 | Fallback на main-thread рендер |
| SAB требует COOP/COEP | Fallback на ArrayBuffer + transfer |
| Subsetting ломает CJK | Тесты на 20 шрифтах, fallback на full embed |
| Расхождение с MS Word | Differential testing, SSIM в CI |
| Большие файлы (>500 МБ) | Streaming-парсинг, lazy-read частей |
| Регрессия производительности | `github-action-benchmark`, гейт 10% |
| Утечки памяти в воркере | 30-мин stress, `performance.memory` |
| PDF не открывается в старых viewer | `qpdf --check`, PDF/A валидация |
| Fuzz-находки в XML-парсерах | `cargo-fuzz` ночью, фиксы в первую очередь |

---

## 12. Definition of Done v1.0

- [ ] DOCX и XLSX открывают 100% фикстур без потери данных.
- [ ] SSIM ≥ 0.95 vs MS Office на 50 эталонах.
- [ ] PDF векторный, шрифты встроены, subsetting работает.
- [ ] Все рендер-операции в воркере, main FPS ≥ 60.
- [ ] `OffscreenCanvas` передан один раз.
- [ ] PDF/PNG через Transferables, zero-copy.
- [ ] SAB работает при COOP/COEP; fallback без них.
- [ ] Все бюджеты размера и производительности соблюдены.
- [ ] Покрытие ≥ 85% / 80%.
- [ ] Аудит безопасности чистый.
- [ ] Chrome 90+, Firefox 96+, Safari 16.4+; fallback везде.
- [ ] Примеры работают в 3 браузерах.
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
- [ ] GitHub Projects с 12 спринтами.
- [ ] CI-матрица: native + wasm + 3 браузера.
- [ ] Линтеры и pre-commit.
- [ ] Согласованы бюджеты.
- [ ] Kick-off с разбором roadmap.
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

**Итог: 24 недели, 12 спринтов, 12 вех, 6 пакетов, 3 формата экспорта, 3 браузера.**

_Конец ROADMAP.md_
