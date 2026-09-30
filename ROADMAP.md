# doc-converter — Roadmap (Фазы 2–8)

> Продолжение Фазы 1 (`bootstrap-phase1.sh` создал workspace, CI, RPC Main↔Worker).
> Этот документ — исполняемый план до v1.0. Каждая фаза самодостаточна: после
> неё можно мержить в `main` и релизить промежуточный минор.

## Оглавление

- [Общая карта](#общая-карта)
- [Фаза 2 — Painter на OffscreenCanvas + двойная буферизация SAB](#фаза-2)
- [Фаза 3 — XLSX: парсер + buildDisplayList + paint](#фаза-3)
- [Фаза 4 — PDF: ручной painter + шрифты + subsetting + пагинация](#фаза-4)
- [Фаза 5 — SAB-путь + COOP/COEP + fallback + transferables](#фаза-5)
- [Фаза 6 — DOCX: парсер + pagination + line-breaking](#фаза-6)
- [Фаза 7 — CLI + WorkerPool + batch-конвертация](#фаза-7)
- [Фаза 8 — Playwright + бенчмарки + релиз v1.0](#фаза-8)
- [Сквозные артефакты (живут между фазами)](#сквозные-артефакты)
- [Матрица рисков](#матрица-рисков)

---

## Общая карта

| Фаза | Недели | Ключевой результат | Релиз |
|------|--------|--------------------|-------|
| 1 ✔  | 1–3    | workspace, CI, RPC Main↔Worker, крейты-заглушки | `@doc-converter/core@0.1.0` |
| 2    | 4–6    | Painter на OffscreenCanvas, SAB ring, resize/DPR, exportPng | `@doc-converter/core@0.2.0` |
| 3    | 7–9    | XLSX парсинг, buildDisplayList, paint, hit-test, скролл 1M ячеек | `@doc-converter/xlsx@0.1.0` |
| 4    | 10–12  | PDF-экспорт XLSX: векторный painter, шрифты, subsetting, пагинация | `@doc-converter/pdf@0.1.0` |
| 5    | 13–14  | COOP/COEP, SAB в проде, fallback без SAB, transferables везде | `@doc-converter/core@0.3.0` |
| 6    | 15–18  | DOCX парсинг, layout, pagination, line-breaking, numbering | `@doc-converter/docx@0.1.0` |
| 7    | 19–20  | CLI, WorkerPool, batch-конвертация, прогресс | `@doc-converter/cli@0.1.0` |
| 8    | 21–22  | Playwright, differential-тесты, бенчмарки, публикация v1.0 | всё `@1.0.0` |

**Правило перехода между фазами.** Фаза не закрывается, пока:
1. `cargo test --workspace` зелёный на x86_64 и wasm32.
2. `pnpm turbo run typecheck test build` зелёный.
3. Бюджеты `size-limit` не превышены.
4. `docs/` обновлён (архитектура + API).
5. `CHANGELOG.md` сгенерирован через changesets.

---

<a id="фаза-2"></a>
## Фаза 2 — Painter на OffscreenCanvas + двойная буферизация SAB

**Цель.** Main не рисует. Воркер получает `OffscreenCanvas`, paints `DisplayList` в `requestAnimationFrame`, отдаёт `tick` с таймингами. DisplayList передаётся через SAB с ring-буфером из 2 слотов — без сериализации в JSON.

**Продолжительность.** 3 недели.

### 2.1 Структура файлов

```
crates/render/src/
├── painter/
│   ├── mod.rs              # pub use painter_2d::*;
│   ├── painter_2d.rs       # Painter2D для OffscreenCanvasRenderingContext2D
│   ├── state.rs            # стек clip/transform, fillStyle/strokeStyle dedup
│   ├── text.rs             # измерения, align/baseline, ellipsis
│   └── bitmap_cache.rs     # HashMap<u32, web_sys::ImageBitmap>
├── sab/
│   ├── mod.rs
│   ├── writer.rs           # SabWriter (Rust-сторона, пишет в линейный буфер)
│   ├── reader.rs           # SabReader для тестов
│   └── ring.rs             # ring из 2 слотов + atomic state
└── display_list.rs         # + to_bytes()/from_bytes() для zero-copy

crates/wasm/src/
├── painter_api.rs          # paint_display_list_sab(ptr, len), resize_canvas(dpr)
├── bitmap_api.rs           # register_bitmap(id, ImageBitmap), drop_bitmap(id)
└── sab_api.rs              # alloc_sab(bytes) -> SharedArrayBuffer

packages/core/src/
├── worker/
│   ├── worker.ts           # обновлён: paint через SAB, tick с таймингами
│   └── frame_loop.ts       # rAF-цикл, drop кадров при backpressure
├── sab/
│   ├── reader.ts           # SabReader в TS (для тестов/дебага)
│   └── protocol.ts         # SlotHeader: seq, len, generation
└── render/
    ├── offscreen.ts        # + bitmap upload, + dpr
    └── resize_observer.ts  # debounce resize, aspect-lock
```

### 2.2 Ключевые контракты

**`crates/render/src/sab/ring.rs`:**

```rust
/// Ring из двух слотов. Писатель кладёт в N+1, читатель берёт N.
/// Atomic-состояние в первых 16 байтах:
///   [0..4)  seq_writer: u32   — сколько кадров записано
///   [4..8)  seq_reader: u32   — сколько кадров прочитано
///   [8..12) slot_len[0]: u32  — длина DisplayList в слоте 0
///   [12..16) slot_len[1]: u32 — длина DisplayList в слоте 1
pub struct SabRing {
    buf: js_sys::SharedArrayBuffer,
    header: js_sys::Int32Array,
    slot0: js_sys::Uint8Array,
    slot1: js_sys::Uint8Array,
    slot_capacity: usize,
}

impl SabRing {
    pub fn new(slot_capacity: usize) -> Self;

    /// true — можно писать в следующий слот (читатель отстаёт < 2).
    pub fn has_free_slot(&self) -> bool;

    /// Пишет байты DisplayList в следующий слот, обновляет seq_writer.
    /// Возвращает false, если слот ещё занят читателем.
    pub fn try_write(&mut self, bytes: &[u8]) -> bool;

    /// Возвращает срез байтов активного слота для чтения.
    pub fn read_current(&self) -> Option<&[u8]>;

    /// Помечает текущий слот прочитанным (seq_reader += 1).
    pub fn release_current(&mut self);
}
```

**`crates/wasm/src/painter_api.rs`:**

```rust
#[wasm_bindgen]
pub fn paint_display_list_sab(
    ctx: web_sys::OffscreenCanvasRenderingContext2D,
    sab: js_sys::SharedArrayBuffer,
    slot_capacity: usize,
) -> Result<PaintStats, JsValue>;
// Возвращает { cmds: u32, dropped: bool, paintMs: f64 }.

#[wasm_bindgen]
pub fn resize_canvas(ctx: web_sys::OffscreenCanvasRenderingContext2D, dpr: f32);
```

**`packages/core/src/worker/frame_loop.ts`:**

```ts
export interface FrameLoopCallbacks {
  build(): SharedArrayBuffer;        // Rust строит DisplayList в SAB
  paint(sab: SharedArrayBuffer): PaintStats;
  onTick(stats: PaintStats & { frameId: number }): void;
}

/**
 * Коалесцирует вызовы render() в один rAF. Drop кадров при отставании.
 * Никаких аллокаций в hot-path.
 */
export function startFrameLoop(cb: FrameLoopCallbacks): {
  request(): void;
  stop(): void;
};
```

### 2.3 Алгоритм painter'а

Порядок обработки `DrawCommand` (пакетный, чтобы минимизировать смены `fillStyle`/`strokeStyle`):

1. Пройти по командам, сгруппировать соседние `Rect`/`Line` по `(fill, stroke)`.
2. Для каждой группы — установить стиль один раз, отрисовать все.
3. `PushClip`/`PopClip` → `ctx.save()` + `ctx.rect()` + `ctx.clip()` / `ctx.restore()`.
4. `PushTransform`/`PopTransform` → `ctx.setTransform(a,b,c,d,e,f)` со стеком.
5. `Text` → `ctx.font` (кэш по `(fontId, sizePx)`), `ctx.textAlign`, `ctx.textBaseline`, `measureText` только при `overflow: Ellipsis`.
6. `Image` → `bitmap_cache.get(bitmap_id)` → `ctx.drawImage`.
7. `Rect` с `radius != [0;4]` → `ctx.roundRect` (fallback: `arcTo` вручную, Safari < 16.4).

**Кэши:**
- `font_cache: HashMap<(FontId, u32 /*size_bits*/), String>` — собирает `"${size}px fontN"`.
- `bitmap_cache: HashMap<u32, ImageBitmap>` — заполняется через `register_bitmap`.
- `style_cache: HashMap<Color, String>` — `to_css()` один раз.
- `clip_depth: usize` — счётчик для отладки утечек `save/restore`.

### 2.4 Обработка `resize` + DPR

```ts
// packages/core/src/render/resize_observer.ts
export function attachResize(
  canvas: HTMLCanvasElement,
  rpc: RpcHandle,
  opts: { debounceMs?: number; maxDpr?: number } = {},
): () => void;
```

Правила:
- DPR клампится `min(window.devicePixelRatio, opts.maxDpr ?? 3)` — на 4K Retina без клампа `canvas.width * height` > 16M → `OffscreenCanvas` падает.
- Debounce 100 мс по умолчанию, trailing edge.
- При изменении DPR (перенос окна на другой монитор) — форсированный repaint.
- `resize_canvas` в Rust сбрасывает `clip_depth` и `bitmap_cache` (bitmap'ы нужны в новых размерах).

### 2.5 Экспорт PNG

```ts
// packages/core/src/render/export_png.ts
export async function exportPng(
  canvas: OffscreenCanvas,
  dpr: number,
): Promise<Uint8Array>;
```

Реализация:
1. `const blob = await canvas.convertToBlob({ type: 'image/png' })`.
2. `const ab = await blob.arrayBuffer()`.
3. `new Uint8Array(ab)` — transferable в main.

Работает из воркера начиная с Chrome 108, Firefox 116, Safari 16.4.

### 2.6 Тесты

| Тип | Что | Где |
|-----|-----|-----|
| Unit | `SabRing::has_free_slot/try_write/read/release` циклически на 1000 итераций | `crates/render/src/sab/ring.rs::tests` |
| Unit | `DisplayList::to_bytes` → `from_bytes` round-trip | `crates/render/src/display_list.rs::tests` |
| Integration | Painter отрисовывает пустой `DisplayList` без паники | `crates/wasm/tests/painter_empty.rs` |
| WASM | `wasm_bindgen_test`: 1000 `Rect` → `paint` < 16 мс | `crates/wasm/tests/painter_perf.rs` |
| Playwright | Ресайз окна → 200 мс → `canvas.width` пересчитан | `packages/core/test/resize.spec.ts` |
| Playwright | Pointer по canvas → `tick` получен, `paintMs < 16` | `packages/core/test/tick.spec.ts` |
| Playwright | `exportPng` возвращает валидный PNG (magic bytes) | `packages/core/test/png.spec.ts` |

### 2.7 Бенчмарки (criterion)

```rust
// benches/render.rs
fn bench_paint_1k_rects(c: &mut Criterion);
fn bench_paint_10k_text(c: &mut Criterion);
fn bench_sab_write_read_cycle(c: &mut Criterion);
```

Цели:
- `paint` 1000 `Rect` в 1200×800 viewport: **< 4 мс**.
- `paint` 1000 `Text`: **< 8 мс**.
- `sab_write_read_cycle`: **< 50 мкс** на кадр.

### 2.8 Definition of Done

- [ ] `@doc-converter/core@0.2.0` опубликован.
- [ ] `OffscreenCanvas` передаётся один раз; повторный `init` → `error`.
- [ ] `exportPng` работает в Chrome/Firefox/Safari.
- [ ] `tick` приходит каждый кадр, `paintMs` < 16 мс на 1080p.
- [ ] `size-limit` worker-бандла ≤ 550 КБ gzip.
- [ ] Тесты Фазы 2 зелёные в CI (все 6 пунктов таблицы).

### 2.9 Риски

| Риск | Митигация |
|------|-----------|
| Safari 16.4 не поддерживает `desynchronized: true` | fallback: игнорировать опцию, но всё равно работает |
| `convertToBlob` медленный на больших canvas | ограничить размер PNG-экспорта: `dpr <= 2` для > 4K |
| `measureText` дёргает layout | кэш `TextMetrics` + отказ от ellipsis, если ширина известна |
| RACE: `resize` приходит в середине `paint` | `resize` обрабатывается вне rAF, `paint` читает snapshot `(w,h,dpr)` |

---

<a id="фаза-3"></a>
## Фаза 3 — XLSX: парсер + buildDisplayList + paint

**Цель.** Открыть `.xlsx` в 100 МБ / 1M ячеек за < 5 с WASM, отрендерить видимую область 1200×800 за < 16 мс, скроллить 1M ячеек на 60 FPS.

**Продолжительность.** 3 недели.

### 3.1 Структура файлов

```
crates/xlsx/src/
├── lib.rs
├── workbook.rs         # Workbook: sheets, named ranges, calc chain
├── sheet.rs            # Sheet: cells, merged, dimensions, freeze
├── cell.rs             # Cell, CellValue, CellRef, CellRange
├── shared_strings.rs   # SST с индексом + sparse lookup
├── styles.rs           # StyleTable: fonts, fills, borders, numFmt
├── numfmt.rs           # FormatCode → форматированная строка (числа/даты/проценты)
├── column.rs           # ColumnLayout: widths, hidden, custom
├── row.rs              # RowLayout: heights, hidden, custom
├── merge.rs            # MergedRegions: lookup O(log n) по row
├── conditional.rs      # Условное форматирование → применяет final style
├── hyperlinks.rs       # Гиперссылки на ячейках
├── drawings.rs         # Изображения (anchored), без диаграмм пока
├── tables.rs           # Table regions (для повторения header)
├── parse/
│   ├── mod.rs
│   ├── workbook_xml.rs
│   ├── sheet_xml.rs    # streaming <row>/<c> → cell iterator
│   ├── shared_strings.rs
│   └── styles_xml.rs
├── layout/
│   ├── mod.rs
│   ├── build.rs        # build_display_list(viewport) → DisplayList
│   ├── columns.rs      # видимый диапазон колонок по X
│   ├── rows.rs         # видимый диапазон строк по Y
│   ├── grid.rs         # сетка только для видимой области
│   └── headers.rs      # заголовки строк/столбцов
└── hit_test.rs         # (x, y) → CellRef

crates/render/src/
├── build_context.rs    # BuildContext: style table, string pool, font registry
└── text_layout.rs      # wrap/ellipsize через FontRegistry
```

### 3.2 Модель данных (память ≤ 3× размера файла)

```rust
// crates/xlsx/src/cell.rs
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    Number(f64),
    Boolean(bool),
    Error(ErrorKind),         // #DIV/0!, #N/A, #REF! ...
    InlineString(Box<str>),
    SharedString(u32),        // индекс в SST
    Formula { cached: Box<CellValue>, formula: Box<str> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRef { pub row: u32, pub col: u32 }  // 0-based

// crates/xlsx/src/sheet.rs — разреженное хранение
pub struct Sheet {
    pub name: String,
    pub dimension: CellRange,                     // A1:Z1000
    cells: FxHashMap<CellRef, Cell>,              // разреженно
    pub merged: MergedRegions,                    // отсортированные, для O(log n)
    pub cols: Vec<ColumnInfo>,                    // по индексу = col (default width при отсутствии)
    pub rows: FxHashMap<u32, RowInfo>,            // высоты, скрытые
    pub freeze: Option<(u32, u32)>,               // (frozen_rows, frozen_cols)
    pub hyperlinks: FxHashMap<CellRef, String>,
    pub tables: Vec<TableRegion>,
}
```

**Почему `FxHashMap`, а не `Vec<Vec<Cell>>`:** файл с 1M ячеек имеет ≤ 5% заполненных. Плотная матрица 1M × 16 байт = 16 МБ на пустоту. Разреженное — ~500 КБ.

### 3.3 Streaming-парсер `sheet.xml`

Проблема: `sheet.xml` 100 МБ → держать DOM в памяти нельзя.

```rust
// crates/xlsx/src/parse/sheet_xml.rs
pub fn parse_sheet<R: Read>(
    reader: R,
    styles: &StyleTable,
    sst: &SharedStrings,
) -> Result<Sheet, ParseError>;

// Внутри: quick_xml Reader с buf 64 КБ, читает поток тегов:
//   <dimension ref="A1:Z1000"/>
//   <cols><col min="1" max="1" width="12.5"/></cols>
//   <sheetData>
//     <row r="1" ht="15" hidden="1">
//       <c r="A1" t="s" s="3"><v>42</v></c>
//       <c r="B1" t="str"><f>SUM(A:A)</f><v>3.14</v></c>
//     </row>
//   </sheetData>
//   <mergeCells>...</mergeCells>
//   <conditionalFormatting>...</conditionalFormatting>
//   <hyperlinks>...</hyperlinks>
// Всё остальное игнорируется через fast skip (`Reader::read_to_end` внутри `tag`).
```

Ключевая оптимизация: не создавать `String` на каждый текстовый узел. Использовать `quick_xml::events::BytesText::unescape` только для SST и inline strings; числа парсить `bytes_to_f64` без intermediate `String`.

### 3.4 Shared Strings Table

```rust
// crates/xlsx/src/shared_strings.rs
pub struct SharedStrings {
    /// Индекс: shared string id → (offset, len) в `data`.
    index: Vec<(u32, u32)>,
    data: String,                 // все строки подряд
    unique_count: u32,
    total_count: u32,
}

impl SharedStrings {
    pub fn get(&self, id: u32) -> Option<&str> {
        let (off, len) = *self.index.get(id as usize)?;
        let start = off as usize;
        self.data.get(start..start + len as usize)
    }
}
```

Для XLSX с 500k уникальных строк и 2M использований: `index` 4 МБ, `data` 20 МБ. Lookup O(1). SST строится один раз при `open_xlsx`.

### 3.5 `StyleTable` и `numFmt`

```rust
// crates/xlsx/src/styles.rs
pub struct StyleTable {
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    num_fmts: Vec<NumFmt>,
    cell_xfs: Vec<CellXf>,       // index → (fontId, fillId, borderId, numFmtId, align, ...)
}

pub struct CellXf {
    pub font_id: u32,
    pub fill_id: u32,
    pub border_id: u32,
    pub num_fmt_id: u32,
    pub align: Alignment,
    pub protection: Protection,
}
```

```rust
// crates/xlsx/src/numfmt.rs
pub fn format_value(
    value: &CellValue,
    fmt: &FormatCode,
    date1904: bool,
) -> String;

/// Поддерживается:
/// - числа: `0`, `0.00`, `#,##0.00`, `0%`, `0.00E+00`
/// - даты: `yyyy-mm-dd`, `dd.mm.yyyy`, `h:mm:ss`, `[$-409]ddd`
/// - текст: `@`, `"prefix"@`
/// - валюта: `"$"#,##0.00`, `€#.##0,00`
/// - условия: `[Red][>100]0;0;[Blue]0`
pub fn parse_format(code: &str) -> FormatCode;
```

**Тонкость:** формат содержит **до 4 секций**, разделённых `;`: `positive;negative;zero;text`. Реализация — простая state machine, ~400 строк.

### 3.6 `build_display_list` — горячий путь

```rust
// crates/xlsx/src/layout/build.rs
pub fn build_display_list(
    sheet: &Sheet,
    styles: &StyleTable,
    sst: &SharedStrings,
    fonts: &mut FontRegistry,
    viewport: Viewport,
    cfg: &RenderConfig,
) -> DisplayList;
```

Алгоритм:
1. **Видимый диапазон колонок.** Бинарный поиск по prefix-sum ширин колонок: `[col_start, col_end)`.
2. **Видимый диапазон строк.** Аналогично, но с учётом скрытых и custom heights.
3. **Сетка** (если `cfg.show_grid`): для каждой `(row, col)` в диапазоне — линия. Пропуск ячеек с border'ами.
4. **Заливки**: только ячейки с `fill_id != 0`.
5. **Borders**: 4 линии на ячейку; thin/medium/thick/dashed/dotted/double → `StrokeStyle`. Двойные границы рисуются как две линии с gap 0.5 pt.
6. **Текст**: измерение через `FontRegistry::measure`, при `wrap` — `break_lines`, при overflow — `ellipsize`.
7. **Merged**: сначала рисуются merged regions (одна команда `Rect` + один `Text` на регион), потом всё остальное. Сетка под merged не рисуется.
8. **Freeze panes**: `PushClip` для frozen top/left + повторная отрисовка.
9. **Headers**: заголовки столбцов (A, B, C, ..., AA) и строк (1, 2, 3...).
10. **Hyperlinks**: подчёркивание + цвет из темы; hit-test возвращает URL.

**Оптимизации:**
- Батчинг по цвету заливки: команды `Rect` с одинаковым `fill` группируются.
- Ленивая подстановка текста: только если пересекается с viewport.
- Переиспользование `Vec<DrawCommand>` между кадрами (capacity = 8k команд).
- Пропуск заведомо пустых range'ов через `Sheet::dimension` и `Sheet::cells.is_empty()`.

### 3.7 Hit-test

```rust
// crates/xlsx/src/hit_test.rs
pub fn hit_test(sheet: &Sheet, x: f32, y: f32) -> Option<CellRef>;
```

Prefix-sum по колонкам/строкам, бинарный поиск O(log n). Возвращает `None`, если точка вне `dimension`.

### 3.8 Публичный TS API

```ts
// packages/xlsx/src/index.ts
export async function openXlsx(bytes: Uint8Array): Promise<XlsxWorkbook>;
export function sheetNames(wb: XlsxWorkbook): string[];
export function displayList(
  wb: XlsxWorkbook,
  sheet: number,
  viewport: Viewport,
): DisplayList;
export function paintDisplayList(
  ctx: OffscreenCanvasRenderingContext2D,
  dl: DisplayList,
): void;
export function cellAtPoint(wb: XlsxWorkbook, sheet: number, x: number, y: number): CellRef | null;
export function exportSheetToCsv(wb: XlsxWorkbook, sheet: number): string;
export function exportSheetToJson(wb: XlsxWorkbook, sheet: number): unknown;
```

### 3.9 Тесты

| Тип | Что | Покрытие |
|-----|-----|----------|
| Unit | `parse_shared_strings` на 20 реальных SST | corner cases: XML entities, unicode, empty |
| Unit | `numfmt::format_value` — 300 кейсов (даты, валюта, условия) | 100% ECMA-376 секции 18.8.30 |
| Unit | `MergedRegions::at(cell)` — O(log n) | 500 merged регионов |
| Proptest | `CellRef::parse("A1")` → `to_string` round-trip | 10k итераций |
| Insta | Snapshot XML-модели для 30 фикстур | golden |
| Integration | `open_xlsx` на 50 реальных файлах без ошибок | 100% |
| Integration | `build_display_list` viewport 1200×800 → < 8k команд | размер |
| wasm-bindgen-test | `open_xlsx` 100 МБ → < 5 с | perf |
| Stress | Scroll 1M ячеек 30 минут без утечек | memory |
| Playwright | Скролл 1M ячеек на 60 FPS | FPS |

### 3.10 Бенчмарки

```rust
// benches/parse.rs
fn bench_open_xlsx_1m_cells(c);
fn bench_open_xlsx_100mb(c);

// benches/render.rs
fn bench_build_display_list_100k_visible(c);
fn bench_hit_test_1m(c);
```

Цели:
- `open_xlsx` 100 МБ: **< 5 с WASM**, < 2 с native.
- `build_display_list` 100k видимых ячеек: **< 100 мс**.
- `hit_test`: **< 2 мс**.

### 3.11 Definition of Done

- [ ] `@doc-converter/xlsx@0.1.0` опубликован.
- [ ] 100% фикстур `test-fixtures/xlsx/*.xlsx` открываются.
- [ ] 1M ячеек открываются < 5 с, память ≤ 3× файла.
- [ ] Скролл 1M ячеек 60 FPS (Playwright trace).
- [ ] `numfmt` проходит 300 тестов.
- [ ] CSV/JSON export работает.
- [ ] `size-limit` xlsx-бандла ≤ 500 КБ gzip.

### 3.12 Риски

| Риск | Митигация |
|------|-----------|
| `sheet.xml` 200 МБ не влезает в WASM heap | стриминг + отказ от DOM; увеличить max memory через `wasm-opt` |
| `SST` 500k строк — долгий построение индекса | two-pass: сначала count, потом однопроходное построение |
| Формулы не вычисляются (только cached) | явно документировать: `#VALUE!` если cached отсутствует |
| Условное форматирование может дублировать стили | применять **последним**, поверх базового |

---

<a id="фаза-4"></a>
## Фаза 4 — PDF: ручной painter + шрифты + subsetting + пагинация

**Цель.** XLSX-лист → векторный PDF с встроенными subsetted шрифтами. A4/A3, portrait/landscape, repeat header rows, fit-to-width, watermark. 500 страниц < 3 с.

**Продолжительность.** 3 недели.

### 4.1 Структура файлов

```
crates/pdf/src/
├── lib.rs
├── painter.rs         # PdfExporter — основной цикл
├── layer.rs           # обёртка над printpdf::PdfLayerReference
├── fonts.rs           # FontRegistry → embedded subsets
├── subset.rs          # subsetting через allsorts
├── encoding.rs        # ToUnicode CMap, CID для CJK
├── pagination.rs      # разбиение sheet → страницы
├── layout.rs          # PageConfig, страницы, отступы
├── styles.rs          # CellStyle для PDF (mm-единицы)
├── text.rs            # измерение, перенос, align/valign
├── border.rs          # thin/medium/thick/dashed/dotted/double
├── background.rs      # заливка ячейки
├── grid.rs            # сетка + заголовки строк/столбцов
├── hyperlink.rs       # /Link аннотации
├── image.rs           # XObject (PNG/JPEG), обрезка
├── watermark.rs       # диагональный текст или PNG
├── metadata.rs        # title/author/subject/keywords
├── wasm_api.rs        # wasm-bindgen exports
└── tests.rs
```

### 4.2 Основной цикл

```rust
// crates/pdf/src/painter.rs
pub struct PdfExporter {
    opts: PdfOptions,
    doc: PdfDocumentReference,
    fonts: PdfFontRegistry,
}

impl PdfExporter {
    pub fn new(opts: PdfOptions) -> Self;

    pub fn export_xlsx_sheet(
        &mut self,
        wb: &Workbook,
        sheet_idx: usize,
    ) -> Result<Vec<u8>>;

    pub fn export_xlsx_workbook(&mut self, wb: &Workbook) -> Result<Vec<u8>>;

    fn paint_page(
        &mut self,
        page: &PageLayout,
        sheet: &Sheet,
        styles: &StyleTable,
    ) -> Result<()>;

    fn paint_cell(
        &self,
        layer: &PdfLayerReference,
        cell: &Cell,
        x_mm: f32, y_mm: f32, w_mm: f32, h_mm: f32,
        style: &CellStyle,
    ) -> Result<()>;
}
```

**Порядок отрисовки ячейки:**
1. Background (Rect с fill).
2. Диагонали (если заданы `diagonalUp`/`diagonalDown`).
3. Границы (4 линии; Double — две параллельные с зазором 0.5 pt).
4. Текст построчно с учётом `wrap`, `align`, `valign`, `indent`, `rotation` (0/90/-90/45).

### 4.3 Пагинация

```rust
// crates/pdf/src/pagination.rs
pub struct PageLayout {
    pub rows: Range<u32>,
    pub cols: Range<u32>,
    pub scale: f32,
    pub offset_x_mm: f32,
    pub offset_y_mm: f32,
}

pub fn paginate(sheet: &Sheet, cfg: &PageConfig) -> Vec<PageLayout>;
```

Алгоритм:
1. Эффективная площадь = `page.size - margins`.
2. Пройти строки сверху вниз, накапливая высоту. Разрыв при превышении.
3. `repeat_header_rows` — на каждой новой странице сверху.
4. Аналогично по столбцам для широких таблиц с `repeat_first_columns`.
5. `avoid_row_break` (по умолчанию true): строка выше оставшегося места → целиком на новую страницу.
6. `orphan_rows` / `widow_rows`: не оставлять 1–2 строки в одиночестве.
7. `fit_to_width`: бинарный поиск `scale ∈ [0.1, 1.0]`, чтобы влезло в N страниц по ширине.

**Edge cases (обязательные тесты):**
- Пустой лист → одна страница.
- Ячейки за пределами `dimension` → игнор.
- Merged cells на границе страницы → разрыв или перенос целиком.
- Скрытые строки/столбцы → пропуск.
- Условное форматирование → применять финальный стиль.
- Изображения и диаграммы → XObject, обрезка на границе.
- Комментарии → аннотации `/Text`.
- Гиперссылки → аннотации `/Link`.
- Очень высокие строки (> 500 мм) → разбить или ужать.

### 4.4 Шрифты и subsetting

```rust
// crates/pdf/src/fonts.rs
pub struct FontSpec {
    pub id: FontId,
    pub data: Option<Vec<u8>>,       // TTF/OTF bytes
    pub builtin_fallback: BuiltinFont,
    pub fallback_chain: Vec<FontId>,
}

pub struct PdfFontRegistry {
    embedded: HashMap<FontId, EmbeddedFont>,
    used_glyphs: HashMap<FontId, HashSet<u16>>,  // собираем во время paint
}

impl PdfFontRegistry {
    pub fn register(&mut self, spec: FontSpec) -> Result<()>;
    pub fn measure(&self, id: FontId, size_pt: f32, text: &str) -> TextMetrics;
    pub fn mark_used(&mut self, id: FontId, glyph_ids: &[u16]);
    pub fn finalize(&mut self) -> Result<()>;  // subset всех использованных
}
```

Требования:
- `printpdf::add_external_font` для TTF/OTF.
- **Subsetting** — встраивать только используемые глифы (`allsorts` / `subsetter`).
- Кириллица, греческий, CJK, арабский, иврит.
- Метрики ascent/descent из `hhea`/`OS/2`.
- Экономия: subsetting уменьшает PDF на ~91% (пример: Noto Sans 400 КБ → 36 КБ).

**Алгоритм subsetting (двухпроходный):**
1. Первый проход paint: собираем `used_glyphs: HashMap<FontId, HashSet<u16>>`, попутно пишем placeholder в PDF.
2. Второй проход: subset каждого шрифта через `allsorts::subset::subset`.
3. Обновляем PDF: подменяем шрифтовые объекты на subsetted.

Альтернатива (однопроходная): зарезервировать шрифтовые объекты в PDF, после paint — вставить subsetted данные.

### 4.5 Encoding / ToUnicode

Для корректного copy-paste из PDF нужны ToUnicode CMaps:

```rust
// crates/pdf/src/encoding.rs
pub fn build_tounicode_cmap(glyph_to_unicode: &HashMap<u16, char>) -> String;
```

Для CJK — CID fonts с Identity-H encoding. Для латиницы/кириллицы — WinAnsiEncoding + ToUnicode.

### 4.6 Watermark

```rust
pub struct Watermark {
    pub text: String,
    pub opacity: f32,
    pub angle_deg: f32,
    pub font_size_pt: f32,
    pub color: Color,
}
```

Рисуется первым слоем (под содержимым), полупрозрачным через `ExtGState` с `CA`/`ca`.

### 4.7 WASM API

```rust
// crates/pdf/src/wasm_api.rs
#[wasm_bindgen]
pub fn export_xlsx_sheet_to_pdf(opts_json: &str, xlsx_bytes: &[u8]) -> Result<Vec<u8>, JsValue>;

#[wasm_bindgen]
pub fn export_xlsx_workbook_to_pdf(opts_json: &str, xlsx_bytes: &[u8]) -> Result<Vec<u8>, JsValue>;
```

`opts_json` парсится в `PdfOptions`.

### 4.8 Тесты

| Тип | Что | Покрытие |
|-----|-----|----------|
| Unit | `paginate` — 40 синтетических sheet'ов (пустые, широкие, высокие, merged на границе) | 100% ветвей |
| Unit | `border::draw_double` — точный gap 0.5 pt | golden |
| Unit | `subset` — сравнение с `pyftsubset` (внешний oracle) | точность |
| Insta | Snapshot PDF-потоков (без метаданных) для 20 фикстур | golden |
| Integration | `export_xlsx_sheet` на 50 фикстурах → валидный PDF (`lopdf::Document::load`) | 100% |
| Integration | PDF открывается в `pdfium-render` и рендерит те же размеры | размеры ±1 pt |
| Differential | PDF vs MS Office Print → SSIM ≥ 0.95 | 50 эталонов |
| wasm-bindgen-test | 500 страниц < 3 с | perf |

### 4.9 Бенчмарки

```rust
// benches/export_pdf.rs
fn bench_export_pdf_100_pages(c);
fn bench_export_pdf_500_pages(c);
fn bench_subset_10k_glyphs(c);
```

Цели:
- 100 страниц: **< 1.5 с**.
- 500 страниц: **< 3 с**.
- Subset 10k глифов: **< 200 мс**.

### 4.10 Definition of Done

- [ ] `@doc-converter/pdf@0.1.0` опубликован.
- [ ] 50 фикстур XLSX → валидный PDF (проверка через `lopdf` + `pdfium`).
- [ ] SSIM ≥ 0.95 vs MS Office Print на 50 эталонах.
- [ ] Шрифты subsetted: PDF ≤ 40 КБ на 10 страниц с текстом.
- [ ] Copy-paste текста из PDF работает (ToUnicode).
- [ ] Watermark, header/footer, hyperlinks работают.
- [ ] `size-limit` pdf-бандла ≤ 400 КБ gzip.

### 4.11 Риски

| Риск | Митигация |
|------|-----------|
| CJK subsetting сложен (CID, Identity-H) | Фаза 6, отложить; пока — embed full font |
| Двойные границы визуально шире | тест на gap 0.5 pt |
| MS Office рисует диагонали иначе | differential-тест ловит |
| PDF > 2 ГБ на огромных листах | отказ после 10k страниц с явной ошибкой |

---

<a id="фаза-5"></a>
## Фаза 5 — SAB-путь + COOP/COEP + fallback + transferables

**Цель.** Zero-copy везде, где возможно. SAB работает при COOP/COEP; при их отсутствии — graceful fallback на `ArrayBuffer` + transferables.

**Продолжительность.** 2 недели.

### 5.1 Структура файлов

```
packages/core/src/
├── sab/
│   ├── capabilities.ts      # detectSharedArrayBuffer(), detectOffscreen()
│   ├── fallback.ts          # ArrayBufferWriter/Reader API-совместимые с SAB
│   ├── reader.ts            # SabReader (TS)
│   └── ring.ts              # SabRing (TS) — симметрично Rust
├── transport/
│   ├── mod.ts
│   ├── transferable.ts      # helpers: toTransferable(bytes)
│   └── sab_transport.ts     # передача SAB через postMessage (без копии)
├── render/
│   └── offscreen.ts         # + выбор пути SAB/AB
└── docs/
    └── coop_coep.md         # инструкция для хостинга

docs/workers.md              # дополнен
scripts/
├── setup-coop-coep.md
└── dev-server-headers.md
```

### 5.2 Feature detection

```ts
// packages/core/src/sab/capabilities.ts
export interface Capabilities {
  offscreenCanvas: boolean;
  transferControlToOffscreen: boolean;
  sharedArrayBuffer: boolean;
  crossOriginIsolated: boolean;
  wasmSimd: boolean;
  wasmThreads: boolean;
  path: 'sab' | 'transferable';
}

export function detectCapabilities(): Capabilities;
```

**Правила:**
- `path: 'sab'` требует `crossOriginIsolated === true` **и** `sharedArrayBuffer !== undefined` **и** `offscreenCanvas`.
- Fallback: `ArrayBuffer` + transferables, рендер на main-thread, если OffscreenCanvas недоступен.
- Safari < 16.4: `path: 'transferable'`, рендер на main-thread.

### 5.3 SAB Transport (zero-copy)

```ts
// packages/core/src/transport/sab_transport.ts
export class SabTransport {
  private sab: SharedArrayBuffer;
  private writer: SabRing;
  private reader: SabRing;

  constructor(slotCapacity: number, slots = 2);

  /** Возвращает SAB для передачи воркеру. Не копирует. */
  getBuffer(): SharedArrayBuffer;

  /** main → worker: положить DisplayList. */
  send(bytes: Uint8Array): boolean;

  /** main ← worker: получить tick. */
  onTick(cb: (stats: PaintStats) => void): () => void;
}
```

### 5.4 Fallback Transport

```ts
// packages/core/src/transport/transferable.ts
export class TransferableTransport {
  send(bytes: Uint8Array): void;   // postMessage(bytes, [bytes.buffer])
  onTick(cb: (stats: PaintStats) => void): () => void;
}
```

Использует тот же API. Разница — копирование через structured clone исчезает за счёт transfer of ownership.

### 5.5 Заголовки COOP/COEP

**Vite:**
```ts
// vite.config.ts
export default defineConfig({
  server: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
});
```

**Nginx:**
```nginx
add_header Cross-Origin-Opener-Policy same-origin always;
add_header Cross-Origin-Embedder-Policy require-corp always;
```

**Netlify (`_headers`):**
```
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
```

**Vercel (`vercel.json`):**
```json
{
  "headers": [{
    "source": "/(.*)",
    "headers": [
      { "key": "Cross-Origin-Opener-Policy", "value": "same-origin" },
      { "key": "Cross-Origin-Embedder-Policy", "value": "require-corp" }
    ]
  }]
}
```

### 5.6 Двойная буферизация DisplayList

Ring из 2 слотов. Гарантия: писатель ждёт, читатель не блокирует.

```
slot[0]: [header: seq_writer=5, seq_reader=4, len=1024] [payload]
slot[1]: [header: seq_writer=5, seq_reader=5, len=980]  [payload]
```

Писатель (Rust) выбирает слот `seq_writer % 2`, проверяет `seq_writer - seq_reader < 2`, пишет payload, инкрементит `seq_writer` атомарно.

Читатель (painter) читает `seq_writer`, если `seq_reader < seq_writer`, берёт слот `seq_reader % 2`, рисует, инкрементит `seq_reader`.

Backpressure: если writer видит `seq_writer - seq_reader >= 2`, **дропает старый кадр** (читатель ещё не забрал).

### 5.7 Тесты

| Тип | Что |
|-----|-----|
| Unit | `detectCapabilities` в 4 симулированных окружениях (SAB on/off, OC on/off) |
| Unit | `SabRing` TS ↔ Rust: 10k циклов без рассинхрона |
| Playwright | COOP/COEP on → `crossOriginIsolated === true`, `path === 'sab'` |
| Playwright | COOP/COEP off → `path === 'transferable'`, всё работает |
| Playwright | 1000 рендеров через SAB vs transferable → одинаковый результат |
| Playwright | Safari 16.4 (WebKit) — оба пути |

### 5.8 Definition of Done

- [ ] `@doc-converter/core@0.3.0` опубликован.
- [ ] SAB-путь: 0 копирований DisplayList между воркером и main (проверка через Chrome DevTools Memory).
- [ ] Fallback на transferable работает во всех поддерживаемых браузерах.
- [ ] `docs/workers.md` содержит инструкцию для Nginx/Netlify/Vercel/Vite.
- [ ] Тесты COOP/COEP on/off зелёные.

### 5.9 Риски

| Риск | Митигация |
|------|-----------|
| SAB замедляет на маленьких DisplayList (< 4 КБ) | порог: `payload_size < 4KB` → transferable |
| Гонка seq_writer/seq_reader | Atomics.wait/notify — но только в worker, main не должен блокироваться |
| Safari 16.4 не поддерживает `Atomics.wait` в main | не используем `Atomics.wait` в main |

---

<a id="фаза-6"></a>
## Фаза 6 — DOCX: парсер + pagination + line-breaking

**Цель.** Открыть `.docx` 50 МБ за < 4 с WASM. Отрендерить A4-страницу за < 30 мс. Совпадение с MS Word SSIM ≥ 0.95.

**Продолжительность.** 4 недели.

### 6.1 Структура файлов

```
crates/docx/src/
├── lib.rs
├── document.rs          # Document: body, sections, styles
├── styles.rs            # StyleTable: paragraph, character, table, numbering
├── numbering.rs         # <w:numbering>: numId, ilvl, lvlText, start
├── theme.rs             # <w:theme>: fonts, colors
├── settings.rs          # <w:settings>: defaultTabStop, evenAndOddHeaders
├── sections.rs          # <w:sectPr>: page, columns, headers, footers
├── runs.rs              # Run, RunProperties (b/i/u/sz/color/font)
├── paragraphs.rs        # Paragraph, ParagraphProperties
├── tables.rs            # Table, Row, Cell, gridSpan, vMerge
├── images.rs            # Inline + floating, wrap modes
├── header_footer.rs     # Header/Footer parts, first/even/default
├── footnotes.rs         # Footnotes/endnotes
├── hyperlinks.rs        # Hyperlinks + bookmarks
├── fields.rs            # Simple fields (PAGE, NUMPAGES, TOC cache)
├── layout/
│   ├── mod.rs
│   ├── line_break.rs    # break_lines(run) → Lines
│   ├── paragraph.rs     # layout_paragraph → Vec<Line>
│   ├── table_layout.rs  # grid, borders, cell padding
│   ├── page.rs          # pagination по секциям
│   ├── inline.rs        # inline runs → text+formatting
│   └── image_place.rs   # inline/floating placement
├── parse/
│   ├── mod.rs
│   ├── document_xml.rs
│   ├── styles_xml.rs
│   ├── numbering_xml.rs
│   ├── theme_xml.rs
│   ├── settings_xml.rs
│   └── rels_resolve.rs
└── render/
    ├── mod.rs
    ├── build.rs         # build_display_list(page_idx) → DisplayList
    └── hit_test.rs      # (x, y) → TextPos { page, offset }

crates/render/src/
└── text_layout.rs       # общий для XLSX и DOCX: wrap, ellipsize, measure
```

### 6.2 Модель

```rust
// crates/docx/src/paragraphs.rs
pub struct Paragraph {
    pub props: ParagraphProperties,
    pub runs: Vec<RunOrBreak>,
}

pub enum RunOrBreak {
    Run(Run),
    Break { kind: BreakKind },          // Line / Page / Column
    Tab,
    FieldBegin(Field),
    FieldEnd,
    BookmarkStart(String),
    BookmarkEnd(String),
}

pub struct Run {
    pub props: RunProperties,
    pub content: Vec<RunContent>,
}

pub enum RunContent {
    Text(String),
    Tab,
    Break,
    Drawing(DrawingRef),                 // изображение
    Footnote(u32),
    Endnote(u32),
}
```

**`ParagraphProperties`:** align, indentation, spacing, line-height, tabs, numbering, borders, shading, keepNext, keepLines, pageBreakBefore.

### 6.3 Line-breaking

Ключевой алгоритм. Требования:
- Unicode line-break opportunities через `unicode-linebreak`.
- Kerning через `rustybuzz`.
- Ligatures (ffi, ffl) — только внутри слова.
- Hyphenation (опционально, `hyphenation` крейт).
- Justify — растягивание пробелов между словами.

```rust
// crates/docx/src/layout/line_break.rs
pub struct Line {
    pub runs: Vec<LineRun>,
    pub width_px: f32,
    pub y_offset_px: f32,
    pub is_last_of_paragraph: bool,
    pub align: TextAlign,
}

pub fn break_lines(
    paragraph: &Paragraph,
    fonts: &mut FontRegistry,
    styles: &StyleTable,
    available_width_px: f32,
) -> Vec<Line>;
```

Алгоритм:
1. Склеить все runs в один `text` + `Vec<(Range, RunStyle)>`.
2. Найти break opportunities через `unicode_linebreak::linebreaks`.
3. Итеративно жадно упаковывать слова в строку: если не влезает — break, применять `overflow`.
4. Для justify — распределить лишнее пространство между пробелами (кроме последней строки абзаца).
5. Для разрывов страниц — вернуть `Vec<Line>` с флагом `is_page_break_before`.

**Fallback для сложных случаев:** если слово шире доступной ширины — обрезать по code point'ам с `ellipsis`.

### 6.4 Pagination

```rust
// crates/docx/src/layout/page.rs
pub struct PageLayout {
    pub section: usize,
    pub page_in_section: usize,
    pub elements: Vec<LayoutElement>,   // параграфы, таблицы, изображения
    pub header: Option<HeaderContent>,
    pub footer: Option<FooterContent>,
}

pub fn paginate(doc: &Document, styles: &StyleTable) -> Vec<PageLayout>;
```

Алгоритм:
1. Для каждой секции (из `sections.rs`) — начать со `SectionStart::NewPage`.
2. Пройти body: параграфы, таблицы, разрывы.
3. Параграф: `break_lines` → сложить в страницу, пока `accumulated_height <= page_height - margins`.
4. `keepNext` — не разрывать между этим и следующим абзацем.
5. `keepLines` — весь параграф на одной странице.
6. Таблицы: раскладка по строкам, разрыв между строками.
7. Разрывы страниц (`<w:br w:type="page"/>`) — форсированный разрыв.
8. Колонтитулы: first/even/default — подставляются после пагинации.

### 6.5 Tables

```rust
// crates/docx/src/tables.rs
pub struct Table {
    pub props: TableProperties,
    pub rows: Vec<Row>,
    pub grid: Vec<ColumnWidth>,      // tck → px
}

pub struct Row {
    pub props: RowProperties,
    pub cells: Vec<Cell>,
    pub is_header: bool,             // <w:tblHeader/>
}

pub struct Cell {
    pub props: CellProperties,       // width, vAlign, shading, borders, margins
    pub grid_span: u32,              // <w:gridSpan/>
    pub v_merge: VMerge,             // None / Restart / Continue
    pub content: Vec<BlockElement>,  // paragraphs, nested tables
}
```

Раскладка таблицы:
1. Прочитать `tblGrid` → `ColumnWidth` в tck (1/20 pt).
2. `width` из `tblW` (px = tck / 20 * 96/72).
3. Для каждой строки: распределить колонки с учётом `gridSpan`.
4. `vMerge::Continue` — ячейка продолжается с предыдущей.
5. Borders: `tblBorders` ∩ `tcBorders` — рисуется более приоритетный.
6. `<w:tblHeader/>` — повтор строки на каждой новой странице при разрыве.

### 6.6 Numbering

```rust
// crates/docx/src/numbering.rs
pub struct Numbering {
    pub abstracts: HashMap<u32, AbstractNum>,
    pub nums: HashMap<u32, Num>,       // numId → abstractNumId
}

pub struct AbstractNum {
    pub levels: Vec<Level>,             // ilvl 0..8
}

pub struct Level {
    pub start: u32,
    pub num_fmt: NumFmt,                // decimal, lowerLetter, upperRoman, bullet, ...
    pub lvl_text: String,               // "%1.", "%1.%2", "•", ...
    pub suff: Suffix,                   // tab / space / nothing
    pub indent: f32,
    pub font: Option<String>,
    pub color: Option<Color>,
}

pub struct Num { pub abstract_id: u32 }

/// Резолвит номер для (numId, ilvl, counters) → "1.2.3".
pub fn resolve_number(
    numbering: &Numbering,
    num_id: u32,
    ilvl: u8,
    counters: &[u32; 9],
) -> Option<String>;
```

Счётчики обновляются при проходе по body: `counters[ilvl] += 1`, все `counters[i>ilvl] = start[i]`.

### 6.7 Изображения

```rust
// crates/docx/src/images.rs
pub enum ImageAnchor {
    Inline,
    Floating {
        wrap: WrapMode,             // None / Square / Tight / Through / TopAndBottom
        h_pos: HPosition,           // Absolute / Relative
        v_pos: VPosition,
        behind_text: bool,
    },
}

pub struct ImageRef {
    pub rel_id: String,
    pub anchor: ImageAnchor,
    pub width_px: f32,
    pub height_px: f32,
    pub crop: Option<Crop>,
}
```

Bitmap-и: конвертируются в `ImageBitmap` через `createImageBitmap(blob)` в main-thread и передаются в воркер через transferable (или `BitmapImage` в `web_sys::ImageBitmap`).

### 6.8 `build_display_list(page_idx)`

```rust
// crates/docx/src/render/build.rs
pub fn build_display_list(
    doc: &Document,
    page: &PageLayout,
    styles: &StyleTable,
    numbering: &Numbering,
    fonts: &mut FontRegistry,
    cfg: &RenderConfig,
) -> DisplayList;
```

Отличия от XLSX:
- **Секции** — разные размеры страниц, отступы, колонтитулы.
- **Многострочный текст** — каждая `Line` в `BreakLines` → команда `Text` на строку + `Rect` для shading/borders.
- **Таблицы** — `Rect` для cell background + `Line` × 4 для borders + `Text` внутри.
- **Изображения** — `DrawCommand::Image` с bitmap_id.
- **Колонтитулы** — рендерятся до/после body.
- **Сноски** — отдельный layout внизу страницы.

### 6.9 Публичный API

```ts
// packages/docx/src/index.ts
export async function openDocx(bytes: Uint8Array): Promise<DocxDocument>;
export function paginate(doc: DocxDocument, opts?: LayoutOpts): PageLayout[];
export function renderPage(
  doc: DocxDocument,
  page: number,
  ctx: OffscreenCanvasRenderingContext2D,
): void;
export function exportToPdf(doc: DocxDocument, opts?: PdfOpts): Promise<Uint8Array>;
export function exportToMarkdown(doc: DocxDocument): string;
export function exportToHtml(doc: DocxDocument): string;
export function exportToText(doc: DocxDocument): string;
```

### 6.10 Тесты

| Тип | Что | Покрытие |
|-----|-----|----------|
| Unit | `numbering::resolve_number` для 9 уровней × 5 форматов | 100% |
| Unit | `break_lines` для 100 строк разной длины и языков | golden |
| Unit | `paginate` — 40 фикстур (пустые, таблицы, изображения, разрывы) | 100% ветвей |
| Insta | Snapshot `PageLayout` для 30 фикстур | golden |
| Integration | `open_docx` на 50 реальных `.docx` без ошибок | 100% |
| Differential | PDF vs MS Word Print → SSIM ≥ 0.95 на 50 эталонах | 50 |
| wasm-bindgen-test | `open_docx` 50 МБ < 4 с | perf |
| wasm-bindgen-test | `render_page` A4 < 30 мс | perf |

### 6.11 Бенчмарки

```rust
// benches/render.rs
fn bench_open_docx_50mb(c);
fn bench_paginate_100_pages(c);
fn bench_render_page_a4(c);
fn bench_line_break_10k(c);
```

Цели:
- `open_docx` 50 МБ: **< 4 с WASM**, < 1.5 с native.
- `paginate` 100 страниц: **< 200 мс**.
- `render_page` A4: **< 30 мс**.
- `line_break` 10k строк: **< 50 мс**.

### 6.12 Definition of Done

- [ ] `@doc-converter/docx@0.1.0` опубликован.
- [ ] 100% фикстур `test-fixtures/docx/*.docx` открываются.
- [ ] SSIM ≥ 0.95 vs MS Word на 50 эталонах.
- [ ] Pagination совпадает с MS Word на 30 эталонах (номера страниц).
- [ ] Экспорт в Markdown/HTML/Text работает.
- [ ] `size-limit` docx-бандла ≤ 450 КБ gzip.

### 6.13 Риски

| Риск | Митигация |
|------|-----------|
| Line-breaking не совпадает с Word (Word использует свои правила) | differential-тесты ловят; допускается ±1 слово на строку |
| Секции с колонками (2–3 колонки) | MVP — только одноcolon; multi-col в v1.1 |
| Floating images с wrap Tight/Through | MVP — Square + TopAndBottom; Tight/Through в v1.1 |
| Footnote layout сложен | MVP — только inline footnotes, без wrap |

---

<a id="фаза-7"></a>
## Фаза 7 — CLI + WorkerPool + batch-конвертация

**Цель.** `doc-converter` в npm: конвертация папок с XLSX/DOCX в PDF на Node.js с прогресс-баром, N воркерами, retry на ошибках.

**Продолжительность.** 2 недели.

### 7.1 Структура файлов

```
packages/cli/
├── package.json
├── tsup.config.ts
├── bin/
│   └── doc-converter.js        # executable, шебанг
├── src/
│   ├── index.ts                # main()
│   ├── args.ts                 # commander/yargs wrapper
│   ├── commands/
│   │   ├── convert.ts          # convert <input> -o <output>
│   │   ├── batch.ts            # batch <glob> --out <dir>
│   │   ├── info.ts             # info <file> (метаданные)
│   │   └── version.ts
│   ├── pool.ts                 # WorkerPool
│   ├── worker.ts               # node worker (без canvas, только WASM)
│   ├── progress.ts             # cli-progress wrapper
│   ├── retry.ts                # exponential backoff
│   ├── glob.ts                 # input discovery
│   └── format.ts               # output format detection (pdf/png/md/html/csv/json)
└── tests/
    ├── convert.test.ts
    ├── batch.test.ts
    └── fixtures/
```

### 7.2 WorkerPool

```ts
// packages/cli/src/pool.ts
export interface WorkerTask<T> {
  id: string;
  input: Uint8Array;
  format: 'docx' | 'xlsx';
  output: 'pdf' | 'png' | 'md' | 'html' | 'csv' | 'json';
  opts: Record<string, unknown>;
}

export interface WorkerResult {
  id: string;
  ok: boolean;
  bytes?: Uint8Array;
  error?: Error;
}

export class WorkerPool {
  constructor(opts: { workers: number; wasmUrl: string });
  run(tasks: AsyncIterable<WorkerTask>): AsyncIterable<WorkerResult>;
  close(): Promise<void>;
}
```

Реализация:
- Пул воркеров на `node:worker_threads`.
- Round-robin с учётом занятости (не ждать, а отдавать следующему).
- Backpressure через `AsyncIterator` — не более `2 * workers` задач в очереди.
- `retry` — 2 попытки с экспоненциальной задержкой (100 мс, 400 мс).
- `close()` — `terminate()` всех воркеров, дождаться завершения.

### 7.3 CLI

```bash
doc-converter convert input.docx -o out.pdf
doc-converter convert input.xlsx -o out/ --sheet 0 --dpi 300
doc-converter convert input.xlsx -o out/ --all-sheets
doc-converter batch ./in/*.xlsx --out ./pdf --workers 4 --progress
doc-converter batch ./in/ --out ./pdf --recursive --format pdf
doc-converter info input.docx
doc-converter --version
doc-converter --help
```

### 7.4 Прогресс-бар

```
Converting 47 files with 8 workers
[████████████░░░░░░░] 63% | 30/47 | 2.4 MB/s | ETA 12s | 0 errors
```

- `cli-progress` для отрисовки.
- Обновление раз в 100 мс (throttle).
- В тихом режиме (`--quiet`) — только финальная строка.
- В JSON-режиме (`--json`) — построчный JSONL.

### 7.5 Тесты

| Тип | Что |
|-----|-----|
| Unit | `WorkerPool` — 100 задач на 4 воркерах, все завершены |
| Unit | `retry` — 3 попытки при ошибке |
| Integration | `convert input.docx -o out.pdf` → файл создан, валидный PDF |
| Integration | `batch ./fixtures/*.xlsx --out ./tmp` → 10 PDF, 0 ошибок |
| Integration | `--workers 1` vs `--workers 8` → одинаковый результат |
| Stress | 100 файлов × 10 МБ → без OOM за 5 минут |
| CLI | `--help`, `--version`, невалидные args → exit codes |

### 7.6 Definition of Done

- [ ] `@doc-converter/cli@0.1.0` опубликован.
- [ ] `npx @doc-converter/cli convert input.xlsx -o out.pdf` работает.
- [ ] Batch 100 файлов < 60 с на 8 воркерах.
- [ ] Прогресс-бар работает во всех терминалах (Windows/macOS/Linux).
- [ ] `--json` выдаёт machine-readable вывод.
- [ ] `size-limit` CLI-бандла ≤ 150 КБ gzip.

### 7.7 Риски

| Риск | Митигация |
|------|-----------|
| Node.js 20 не поддерживает `OffscreenCanvas` (нет canvas) | WASM без canvas: только PDF/md/html/csv/json |
| Много воркеров → OOM | `--workers` по умолчанию `min(4, hardwareConcurrency)` |
| Большие файлы → медленный transfer между потоками | `SharedArrayBuffer` в Node 20+ |

---

<a id="фаза-8"></a>
## Фаза 8 — Playwright + бенчмарки + релиз v1.0

**Цель.** Полное тестовое покрытие, бенчмарки с CI-гейтами, публикация v1.0 во всех реестрах.

**Продолжительность.** 2 недели.

### 8.1 Playwright

```
tests/e2e/
├── playwright.config.ts
├── fixtures/
│   ├── docx/
│   ├── xlsx/
│   └── corrupted/
├── specs/
│   ├── docx-viewer.spec.ts
│   ├── xlsx-viewer.spec.ts
│   ├── scroll.spec.ts
│   ├── zoom.spec.ts
│   ├── export-pdf.spec.ts
│   ├── export-png.spec.ts
│   ├── hit-test.spec.ts
│   ├── resize.spec.ts
│   ├── dpr.spec.ts
│   ├── coop-coep.spec.ts
│   ├── fallback.spec.ts
│   ├── a11y.spec.ts
│   └── stress.spec.ts
└── utils/
    ├── compare-screenshots.ts       # SSIM
    ├── measure-fps.ts
    └── measure-memory.ts
```

**Ключевые сценарии:**

1. **Scroll 1M ячеек** — 30 секунд, FPS ≥ 60, никаких `Long Task` > 50 мс.
2. **Zoom** 25% → 400% — без переинициализации воркера, только `render`.
3. **Resize** окна — canvas пересчитан, DPR учтён.
4. **DPR=2** — retina, пиксели чёткие (SSIM vs 1x эталона).
5. **Export PDF** — файл создан, `pdfium` открывает, страницы совпадают.
6. **Export PNG** — валидный PNG, размеры соответствуют.
7. **Hit-test** — клик по ячейке возвращает правильный `CellRef`.
8. **COOP/COEP on** — `crossOriginIsolated === true`, `path === 'sab'`.
9. **COOP/COEP off** — `path === 'transferable'`, всё работает.
10. **A11y** — `axe-core` без violations, canvas имеет `aria-label` + `role="img"`.
11. **Stress** — 100 файлов подряд, память воркера не растёт (после `dispose` → возврат к baseline).

### 8.2 Differential-тесты (SSIM)

```ts
// tests/e2e/utils/compare-screenshots.ts
export async function compareWithOffice(
  actual: Buffer,
  expected: Buffer,
  threshold = 0.95,
): Promise<{ ssim: number; pass: boolean }>;
```

Эталоны: 50 документов, отрендеренных в MS Office (Print to PDF) → PNG через `pdfium`. Сравнение с нашим рендером через `pixelmatch` + SSIM.

**Гейт:** SSIM < 0.95 → fail.

### 8.3 Бенчмарки в CI

```yaml
# .github/workflows/benchmarks.yml
name: Benchmarks
on:
  pull_request:
  push: { branches: [main] }
jobs:
  criterion:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with: { toolchain: nightly-2024-12-01 }
      - run: cargo bench --workspace -- --save-baseline current
      - uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: cargo
          output-file-path: target/criterion/*/new/estimates.json
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
          alert-threshold: '110%'
          fail-on-alert: true
```

**Гейт:** регрессия > 10% блокирует merge.

### 8.4 Бюджеты размера

```json
// .size-limit.json
[
  { "path": "packages/core/dist/index.js", "limit": "200 KB", "gzip": true },
  { "path": "packages/docx/dist/index.js", "limit": "450 KB", "gzip": true },
  { "path": "packages/xlsx/dist/index.js", "limit": "500 KB", "gzip": true },
  { "path": "packages/react/dist/index.js", "limit": "25 KB", "gzip": true },
  { "path": "packages/cli/dist/index.js", "limit": "150 KB", "gzip": true },
  { "path": "packages/core/dist/worker.js", "limit": "600 KB", "gzip": true }
]
```

### 8.5 Релиз v1.0

```bash
# 1. Обновить версии через changesets
pnpm changeset version

# 2. Проверить CHANGELOG
git diff CHANGELOG.md

# 3. Коммит
git add .
git commit -m "chore(release): v1.0.0"
git push origin main

# 4. Тег
git tag v1.0.0
git push origin v1.0.0

# 5. CI (release.yml) публикует:
#    - npm: @doc-converter/core, /docx, /xlsx, /pdf, /react, /cli, /wasm
#    - crates.io: doc-converter-core, /render, /docx, /xlsx, /pdf, /wasm
#    - GitHub Release с changelog

# 6. Verify
npm view @doc-converter/core@1.0.0
cargo search doc-converter
```

### 8.6 Финальная приёмка (Definition of Done v1.0)

1. [ ] DOCX и XLSX открывают 100% фикстур без потери данных.
2. [ ] Рендер совпадает с MS Office (SSIM ≥ 0.95) на 50 эталонах.
3. [ ] PDF-экспорт векторный, со встроенными subsetted шрифтами.
4. [ ] Все рендер-операции в воркере; main-thread FPS ≥ 60 при scroll.
5. [ ] OffscreenCanvas передан один раз; повторный вызов — ошибка.
6. [ ] PDF/PNG через transferables, без копирования.
7. [ ] SAB-путь работает при COOP/COEP; fallback — без них.
8. [ ] Бюджеты размера и производительности соблюдены в CI.
9. [ ] Покрытие тестами ≥ 85% (Rust), ≥ 80% (TS).
10. [ ] `cargo-deny`, `cargo-audit`, `npm audit` — чисто.
11. [ ] Chrome 90+, Firefox 96+, Safari 16.4+; fallback для всех.
12. [ ] Примеры работают в 3 браузерах (2 последние версии).
13. [ ] Опубликовано в npm + crates.io под Apache-2.0.

### 8.7 Демо и отчёт

Артефакты приёмки:

- **Репозиторий** с полной историей (main + tags).
- **Бенчмарки** vs `exceljs`, `xlsx`, `mammoth`, `docx` — графики в `docs/benchmarks/`.
- **Отчёт о совместимости** с OOXML — таблица фикстур + процент успеха.
- **Демо-приложение** Vite + React: `<DocxViewer/>`, `<XlsxViewer/>`, `<ExportButton/>`.
- **Видео** 30 с: scroll 1M ячеек на 60 FPS в OffscreenCanvas.
- **Отчёт о памяти** Chrome DevTools Memory для 100 МБ XLSX.
- **Аудит размера** `size-limit --json` + `cargo bloat`.
- **Проверка COOP/COEP + SAB + fallback** — скриншоты + логи.

### 8.8 Риски

| Риск | Митигация |
|------|-----------|
| SSIM < 0.95 на сложных документах | расхождения документируются; v1.1 фиксит |
| Playwright flaky на CI | `--retries=2`, `--workers=2`, trace on failure |
| Бенчмарки шумят на shared runners | baseline на 5 прогонах, alert-threshold 10% |
| Safari WebKit не поддерживает `Atomics.wait` в main | не используем в main |

---

<a id="сквозные-артефакты"></a>
## Сквозные артефакты (живут между фазами)

### `docs/` — обновляется каждую фазу

```
docs/
├── architecture.md          # C4, потоки данных, воркеры (обновляется в Ф2, Ф5)
├── workers.md               # COOP/COEP, SAB, transferables (Ф2, Ф5)
├── api/
│   ├── typedoc/             # генерируется автоматически
│   └── rustdoc/             # cargo doc --workspace --no-deps
├── migration-from-mammoth.md
├── migration-from-exceljs.md
├── cookbook/
│   ├── 01-render-docx.md
│   ├── 02-render-xlsx.md
│   ├── 03-export-pdf.md
│   ├── 04-worker-pool.md
│   └── ...40+ рецептов
├── benchmarks/
│   └── 2025-XX-XX-v1.0.md
└── compat/
    └── ooxml-coverage.md
```

### `CHANGELOG.md` — через changesets

```bash
pnpm changeset          # после каждой фичи/фикса
pnpm changeset version  # перед релизом
```

### `benches/` — расширяется каждую фазу

```
benches/
├── parse.rs            # open_docx, open_xlsx (Ф3, Ф6)
├── render.rs           # build_display_list, paint (Ф2, Ф3, Ф6)
├── export_pdf.rs       # (Ф4)
└── measure_font.rs     # (Ф4)
```

### `test-fixtures/` — пополняется ежемесячно

```
test-fixtures/
├── docx/               # 150+ реальных документов (Ф6)
├── xlsx/               # 150+ реальных книг (Ф3)
├── corrupted/          # 20+ битых файлов (Ф3, Ф6)
└── README.md           # источник + лицензия каждого файла
```

**Правило:** все фикстуры — под лицензией, допускающей использование в тестах. Источник — `<https://file-examples.com>` + собственные + opensource-документы.

---

<a id="матрица-рисков"></a>
## Матрица рисков (сквозная)

| Риск | Вероятность | Влияние | Митигация |
|------|-------------|---------|-----------|
| OOXML слишком разнообразен | Высокая | Средн. | 300+ фикстур, differential-тесты |
| WASM heap ограничен 4 ГБ | Средн. | Высок. | streaming-парсеры, sparse-структуры |
| Word ломает line-breaking | Высокая | Средн. | SSIM ≥ 0.95, расхождения документируются |
| Safari отстаёт по OffscreenCanvas | Средн. | Средн. | fallback на main-thread |
| COOP/COEP усложняют деплой | Средн. | Низк. | fallback на transferables |
| Бенчмарки шумят в CI | Высокая | Низк. | baseline 5 прогонов, threshold 10% |
| Размер WASM-бандла растёт | Средн. | Средн. | `wasm-opt -O4`, `size-limit` гейт |
| Fuzzing находит панику | Низк. | Высок. | `cargo-fuzz` ночью, fix в течение 24 ч |
| Ключевой разработчик выпадает | Средн. | Высок. | документация, bus factor ≥ 2 на крейт |

---

## Итоговое расписание

```
Week  1–3   Фаза 1 ✔  workspace, CI, RPC
Week  4–6   Фаза 2    Painter + SAB ring
Week  7–9   Фаза 3    XLSX
Week 10–12  Фаза 4    PDF
Week 13–14  Фаза 5    SAB + COOP/COEP + fallback
Week 15–18  Фаза 6    DOCX
Week 19–20  Фаза 7    CLI
Week 21–22  Фаза 8    E2E + бенчмарки + v1.0
```

**Итого:** 22 недели (~5.5 месяцев) от старта до v1.0 при одном full-time инженере. С командой из 2–3 человек — 12–14 недель.

---

## Как пользоваться этим документом

1. Открыть фазу, прочитать **Цель** и **Definition of Done**.
2. Скопировать **Структуру файлов** — создать скелеты с `todo!()` / `throw`.
3. Реализовать **Ключевые контракты** (типы и сигнатуры — раньше, чем тела).
4. Прогнать **Тесты** — они определяют готовность.
5. Прогнать **Бенчмарки** — они определяют производительность.
6. Обновить `CHANGELOG.md` через `pnpm changeset`.
7. Опубликовать пакет фазы (`pnpm publish -r`).
8. Перейти к следующей фазе.

Каждая фаза заканчивается **рабочим релизом на npm/crates.io**, а не merge'ом в `main`. Это позволяет ловить регрессии рано и давать пользователям incremental value.
