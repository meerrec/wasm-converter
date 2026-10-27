# Архитектурный аудит doc-converter: v1.4.3, рефакторинг и оптимизация

**Дата:** 23.10.2026
**Коммит:** `f0fd13c` (`v1.4.3`)
**Объём:** 14 крейтов workspace, 5 wasm-модулей, 10 npm-пакетов, 234 Rust-файла,
174 TS-файла, 46 ADR на момент аудита (ADR-0043…0046 добавлены по его итогам)
**Метод:** статический аудит по срезам (Rust-крейты, wasm-мосты, TS-ядро,
вьюеры, CLI, сборка). Сборка и тесты не запускались; ключевые находки
проверены повторным чтением кода. Оценки эффекта, не подтверждённые замером,
помечены словом «оценка».

## TL;DR

Архитектура слоистая и в целом соответствует ADR-0003/0030: `core` → `render`
→ форматы → `pdf` → wasm-мосты → TS. Качество кода высокое: почти нет
`unwrap` в продакшн-коде, 1 300+ Rust-тестов, fuzz, дифференциальные оракулы,
SSIM и жёсткие бюджеты. Главная проблема — **стоимость изменений**, а не
корректность слоёв.

- **Монолиты.** 9 файлов ≥ 2 000 строк держат 28% кода; примерно половина
  этих строк — inline-тесты. Разрезание почти механическое.
- **Дублирование разошлось.** У DOCX три реализации схлопывания границ
  таблиц (canvas, PDF, HTML) с разной семантикой vMerge; три копии разбора
  `pPr/rPr`; три вьюера дублируют ~300 строк; PPTX-воркер — ~600 из 820 строк
  копия `WorkerCore`.
- **Контракты ручные.** Раскладка SAB описана независимо в Rust и TS;
  размеры команд DisplayList продублированы в писателе и читателе; `push_tag`
  игнорирует длину; `from_bytes` может паниковать на невыровненном буфере.
- **Реальные баги.** Висящие промисы в TS-вьюерах, потеря значимых пробелов
  в XLSX (`t="str"`/`t="d"`), SAB-кадр больше 1 МиБ молча теряется, `batch`
  может зависнуть, PPTX даёт `inf`-координаты при `wrap="none"`, парсер
  нумерации DOCX обрывает уровень на парных тегах.
- **Крупнейший запас производительности.** XLSX пересчитывает раскладку,
  условное форматирование и формат числа на каждый кадр; DOCX глубоко
  клонирует страницу на каждом разрыве и дважды меряет сегменты строки; PDF
  не освобождает декодированные изображения; painter аллоцирует строки на
  каждую команду.

Решения, требующие смены архитектуры, вынесены в ADR-0043…0046:
контракты кадра и SAB, шрифты как ленивый ассет, граница PDF-бэкенда,
единый runtime вьюеров.

## 1. Карта системы

### 1.1. Слои

```text
TS:  examples / react / vue / svelte
         │
     create*Viewer → protocol.ts → worker (WorkerCore) → transports (SAB/bytes)
         │                                                  │
         └────────────────── wasm-bindgen ──────────────────┘
Rust: core → render → xlsx / docx / pptx (+ pptx-layout, pptx-render)
                             └────────────→ pdf
                                              │
                          wasm, docx-wasm, pdf-wasm, pptx-wasm, pptx-pdf-wasm
```

`core` не знает про DOCX/XLSX; `render` — общая граница форматов (ADR-0003);
`pdf` сегодня зависит от `docx` и `xlsx` (ADR-0007, пересматривается в
ADR-0045); wasm-мосты — единственное место с `unsafe` (фактически его там нет).

### 1.2. Поток данных

- **Просмотр:** байты → `Archive` (лимиты ZIP, ADR-0015/0031) → `XmlReader`
  → модель → раскладка → **DisplayList** (`DLST`, `DL_VERSION = 4`) → painter
  в воркере или на main → Canvas2D.
- **PDF:** модель → пагинация → собственный PDF-бэкенд (ADR-0022); кадр
  DisplayList в PDF не переиспользуется.
- **Экспорт Markdown/HTML/текст:** модель → независимые экспортёры
  (`crates/docx/src/export/*`), writer — non-goal (ADR-0020).

### 1.3. Объём подсистем

| Подсистема | LOC | Что внутри |
|---|---:|---|
| `crates/docx` | 45 475 | разбор 9,7K, стили/нумерация 5,5K, модель 5,2K, layout 12,4K, render 2,8K, export 2,3K |
| `crates/pdf` | 22 858 | backend 4,6K, общий слой 7,9K, DOCX-мост 1,5K, остальное тесты/бенчи |
| `crates/xlsx` | 20 943 | модель/разбор 11,2K, paint/layout 7,0K |
| `crates/render` + `core` | 7 857 | DisplayList, painter, SAB, шрифты, ZIP/XML |
| PPTX-семейство | 5 076 | `pptx`, `pptx-layout`, `pptx-render`, `pptx-pdf` |
| wasm-мосты | 3 247 | 5 крейтов, 67 `#[wasm_bindgen]`-экспортов, 14 `thread_local!` |
| `packages/core` + `cli` | 13 726 | воркеры, протокол, вьюеры, CLI |
| `examples/viewer` | 7 484 | демо и e2e |

Цифры включают inline-тесты; в разрезе «прод / тесты» крупнейшие файлы
показаны в §3.2.

## 2. Что держит качество

Это нужно сохранить при любом рефакторинге:

- **Ошибки:** `thiserror` 2 + `miette` 7, `#[non_exhaustive]` у публичных
  ошибок; 0 `unwrap` в продакшн-коде `core`/`xlsx`/`docx`/`pdf`, 2 в
  `render`.
- **Тесты:** ~1 400 Rust-тестов, 30 тыс. строк тестов (40% от продакшн-кода),
  `wasm-bindgen-test`, Playwright, 8 fuzz-таргетов, differential vs
  mammoth/exceljs, SSIM против LibreOffice.
- **Бюджеты:** `Cell == 48 B`, `Paragraph/Run/Cell` 88/64/128 Б при
  256/128/192, память PDF 28 МиБ при 40, размеры модулей в gzip.
- **Контракты:** wire-типы генерируются из Rust (ADR-0028) с CI-гейтом дрейфа;
  SAB и bytes — first-class; main-thread fallback; ленивые wasm-модули.
- **Безопасность:** лимиты ZIP, отбрасывание symlink, `#![forbid(unsafe_code)]`
  в четырёх крейтах, `cargo deny`/`cargo audit`, fuzz.
- **Дисциплина решений:** 46 ADR, пересмотр — новым ADR, а не правкой
  принятого.

## 3. Находки

### 3.1. P0 — корректность и надёжность

1. **Висящие промисы в TS-вьюерах.** `pendingHit` и `pendingHyperlink` не
   имеют `reject` и не чистятся при `destroy`/глобальной ошибке воркера
   (`packages/core/src/render/xlsx_viewer.ts:196-197, 428-452, 702-719`;
   то же в DOCX/PPTX). Ошибка теряет `code`: `new Error(msg.message)`. Нет
   `worker.onerror`/`messageerror`. Клик по гиперссылке после `destroy` или
   падение воркера оставляют промис навсегда.
2. **XLSX теряет значимые пробелы.** `value_of` делает `raw.trim()` до
   развилки по типу (`crates/xlsx/src/worksheet.rs:709-735`), поэтому
   строковый результат формулы `t="str"` и ISO-дата `t="d"` обрезаются. Это
   тихая порча данных.
3. **SAB-кадр больше слота.** Слот по умолчанию 1 МиБ; `try_write` возвращает
   `false`, кадр молча дропается — визуально «замерший экран»
   (`crates/docx/src/render.rs:55, 202-238`;
   `packages/core/src/worker/worker_core.ts:760-763`). На bytes-пути буфер
   растёт, на SAB — нет. ROADMAP это признаёт, фолбэка нет.
4. **PPTX: `inf`-координаты и потерянные разрывы.** При `wrap="none"` и
   выравнивании Center/Right `max_width=∞` даёт `shift=∞`
   (`crates/pptx-layout/src/layout.rs:176, 346-351`). Разбор текста смотрит
   только `a:r`, поэтому `a:br`/`a:fld` уровня абзаца (разрывы, номера
   слайдов, даты) теряются (`crates/pptx/src/parse.rs:1019`).
5. **Парсер нумерации DOCX обрывает уровень.** `read_children` считает любой
   `End` концом родителя, а листовые ветки (`start`, `numFmt`, `suff` и т.п.)
   не дочитывают поддерево (`crates/docx/src/numbering.rs:1244, 328-380,
   434-456, 534-537`). На парном `<w:start>…</w:start>` остаток уровня
   теряется.
6. **CLI `batch` может зависнуть.** В пуле нет таймаута/heartbeat
   (`packages/cli/src/pool.ts:67-76`), Deno-воркер не наблюдает `exit`
   (`runtime-deno.ts:92`). Молча умерший воркер вешает `Promise.all`.
7. **PDF-изображения не освобождаются.** `Registry` хранит декодированные
   изображения и полные байты как ключи `BTreeMap` весь экспорт
   (`crates/pdf/src/image.rs:116-175`); DOCX декодирует все media заранее и
   всегда буферизует весь PDF в `Vec` (`crates/pdf/src/docx/mod.rs:84-85`).
   Бюджет 40 МиБ этого не покрывает.
8. **Tagged PDF отсутствует.** В `crates/pdf` нет `StructTreeRoot`,
   `MarkInfo`, `StructElem`, `/Lang`, `ParentTree` (grep пуст). ADR-0012
   описывает объём Спринта 7 (K1–K6), задачи не закрыты. Нужно либо
   реализовать, либо честно пометить ADR нереализованным.
9. **Гонка `open`/`close` в воркере.** `onmessage` асинхронный, `close` может
   выполниться, пока `open` ждёт загрузки модуля; продолжение `open` затем
   снова выставит `format`/`source` (`worker_core.ts:369-375` против
   `:544-555`). После неудачного `init` флаг `initialized` остаётся `true`
   (`:253`) — повторный init невозможен.
10. **Отмена PDF-экспорта не реализована.** В `crates/pdf-wasm` нет
    `set_cancel`; `cancel` лишь реджектит промис после завершения
    синхронного экспорта. Если это осознанно — зафиксировать в ADR-0032;
    иначе контракт отмены неполон.
11. **PPTX: флаг отмены «съедает» следующее открытие.** `set_cancel`
    проверяется только между слайдами (`pptx-wasm/src/pptx_api.rs:73-78`);
    флаг не сбрасывается в начале операции и не гасится после успешной
    раскладки, поэтому `cancel`, пришедший после последней проверки,
    отменит следующее открытие. DOCX гасит флаг всегда
    (`docx-wasm/src/docx_api.rs:88-93`), PPTX — нет.

### 3.2. P1 — структура и дублирование

#### Бог-файлы

| Файл | Прод | Тесты | Всего |
|---|---:|---:|---:|
| `crates/docx/src/document.rs` | 3 794 | 2 374 | 6 168 |
| `crates/docx/src/layout/engine.rs` | 1 831 | 2 166 | 3 997 |
| `crates/xlsx/src/paint.rs` | 1 812 | 1 996 | 3 808 |
| `crates/docx/src/layout/tables.rs` | 2 025 | 877 | 2 902 |
| `crates/docx/src/styles.rs` | 1 585 | 1 160 | 2 745 |
| `crates/docx/src/numbering.rs` | 1 362 | 1 383 | 2 745 |
| `crates/xlsx/src/model.rs` | 1 613 | 708 | 2 321 |
| `crates/xlsx/src/conditional.rs` | 1 555 | 705 | 2 260 |
| `crates/pdf/src/pagination.rs` | 701 | 1 515 | 2 216 |

Разрезание почти механическое: швы намечены разделителями (`parse_blocks`,
таблицы, рисунки, свойства; `build`/`draw_region`/границы/текст). Перенос
inline-тестов в `#[cfg(test)] mod tests;` уже уменьшает файлы вдвое.

#### Разошедшееся дублирование

- **DOCX-таблицы.** Canvas и PDF схлопывают рёбра по-разному; PDF
  дополнительно гасит рёбра внутри vMerge, canvas — нет
  (`crates/docx/src/render/tables.rs:124-250` против
  `crates/pdf/src/docx/tables.rs:96-104, 146-208`). Третий потребитель —
  HTML-экспорт. Это визуальное расхождение, а не только копипаста.
- **Три копии разбора `pPr/rPr`:** `styles.rs:747/832`,
  `numbering.rs:576/648`, `document.rs:2980/3312`, плюс ~20 пар enum-функций
  (~400 строк). Копии расходятся: `border` без `w:val` в `styles.rs` — Single
  без warning, в `numbering.rs` — warning + None; `shd/color` — разный
  регистр и предупреждения.
- **Три вьюера.** ≈280–319 одинаковых нормализованных строк; DOCX↔PPTX — 319.
  Нет общего `viewer_base`: каркас, worker handshake, pending-карты,
  resize/scroll/rAF, экспорт, destroy.
- **PPTX-воркер.** ~600 из 820 строк — структурная копия `WorkerCore`; плюс
  четыре копии ленивого загрузчика wasm.
- **wasm-мосты.** 5 крейтов, 67 экспортов, 14 `thread_local!`, одинаковые
  конверты `serde-wasm-bindgen`, error mapping, stream/export API. Общий
  крейт убирает ≈150–200 строк (12–15%), но не является главным выигрышем.
- **PDF — второй рисовальщик.** Диспетчер `LayoutItem` продублирован
  (`pdf/src/docx/mod.rs:341-399` и `docx/src/render.rs:242-333`);
  текст/подъём базовой линии/границы реализованы дважды.

### 3.3. P1 — межъязыковые контракты и документация

- **DisplayList.** Размеры payload'ов записаны трижды — в комментарии,
  писателе и читателе; `push_tag` принимает `_payload_len` и игнорирует его
  (`crates/render/src/display_list.rs:443-449`); `from_bytes` использует
  `bytemuck::from_bytes` и может паниковать на невыровненном срезе (`:423`).
  Валидатора формата нет.
- **SAB.** `HEADER_BYTES=16`, `SLOT_COUNT=2` совпадают, но семантика
  разошлась: Rust использует `u32` с `wrapping_sub`, TS — знаковые `number`
  без wrap (`packages/core/src/sab/protocol.ts:20-26`). Кросс-теста Rust↔TS
  нет: `frame_transport.spec.ts` проверяет TS сам себя, Rust — тоже.
- **Протокол.** Конверт `InMsg/OutMsg` написан руками; у XLSX/DOCX-воркеров
  нет `default`, поэтому неизвестное сообщение молча теряется и промис
  висит. `cancel` объявлен адресным по `id`, но оба пути ставят глобальный
  флаг. Ошибка отмены ходит строкой `[cancelled]`, хотя `WorkerErrorCode`
  уже типизирован.
- **Документация разошлась с кодом.** `AGENTS.md:20` говорит о версии 1.2.0
  и ждущем `NPM_TOKEN`, фактическая — 1.4.3; `VERSION = '0.1.0'` в
  `packages/core/src/index.ts:48`; README ядра — 1.0.0; ROADMAP утверждает,
  что React-пакета нет и main-fallback не реализован, хотя оба есть;
  ADR-0013 перечисляет numbering-level/table-style/`w:link`, которых в
  каскаде нет; tagged PDF заявлен, но не реализован.

### 3.4. P2 — производительность и размер

#### XLSX

- `SheetLayout::new` и `visible_used_range` вызываются на **каждый кадр** и
  сканируют ячейки; для 1M ячеек это O(cells) на кадр
  (`crates/xlsx/src/paint.rs:205, 329`; `model.rs:283-344`).
- `RuleIndex::new` строится на кадр, пороги CF пересчитываются заново
  (`conditional.rs:82-124, 203-221, 620-680`); правило на весь лист —
  O(cells) на кадр.
- `numfmt::format()` парсит код формата на **каждое число**, хотя модуль
  обещает разбор один раз (`numfmt.rs:170-172`; `paint.rs:1581`).
- `style_at` вызывается 3–4 раза на ячейку, `neighbor_side` — ещё 4 раза;
  `hyperlink_at`/`comment_at` — линейные поиски на ячейку; границы рисуются
  дважды; на каждую числовую ячейку открывается clip.
- `XmlReader::next_significant` делает `into_owned()` на каждое XML-событие
  (`crates/core/src/xml.rs:62`) — миллионы аллокаций на 1M ячеек.

#### DOCX

- `break_page` делает `finished.clone()` со всеми `LayoutItem` и строками на
  каждом разрыве (`crates/docx/src/layout/pagination.rs:442`).
- `table_item` клонирует все ячейки в плоский `cells`, оставляя `rows` —
  двойной контент на каждый фрагмент (`crates/docx/src/layout/tables.rs:194-207`).
- `resolve_run_styled` клонирует `ResolvedRPr` на каждый run
  (`crates/docx/src/layout/cascade.rs:460`); колонтитулы раскладываются
  трижды за проход; сегменты строки меряются дважды (`paragraph.rs:259-261`
  и `line_break.rs:236`).
- `cell_borders` — O(C²) на строку и вызывается дважды на ячейку
  (`tables.rs:1902-1925`); `merge_slots` — O(P·M·R).
- Экспорт текста: 4–6 копий каждого куска (clone → merge → escape →
  `format!`); Markdown-таблица O(R×C).
- Мёртвое поле `RawPPr::sect_pr` (~456 Б на абзац) никогда не заполняется
  (`crates/docx/src/model/raw.rs:791`; `document.rs:3048-3051`), но
  проверяется в `pagination.rs:501`; `Paragraph::mark_rpr` — дубль
  `ppr.r_pr` (~272 Б на абзац).

#### render / painter

- На каждую команду — `css_for(...).to_owned()` и клоны имени гарнитуры
  (`crates/render/src/painter/painter_2d.rs:130, 178, 269`;
  `painter/state.rs:44-50`).
- `DisplayList::intern` не дедуплицирует строки; продюсеры интернят
  «Calibri» на каждый текст.
- `FontRegistry` парсит `FontRef` на каждую строку и на каждый символ
  `break_word`; LRU проверяется после парсинга
  (`crates/render/src/font.rs:311-329, 360-376`).
- Диаграммы заново декодируются и раскладываются каждый кадр
  (`painter_2d.rs:224-227`).
- `hit_test` и `color.rs` в `render` — мёртвый код: `color.rs` не объявлен в
  `lib.rs`, `hit_test` не имеет потребителей; `ellipsize` вызывается только
  из собственных тестов.

#### Размер wasm

- Четыре сабсета Carlito (~370 КБ raw, ~177 КБ gzip по комментарию
  `.size-limit.mjs`) вкомпилированы в **каждый** из пяти модулей, потому что
  все зависят от `render`. Это до ~885 КБ gzip дублирования.
- `miette` с `fancy` тянет в wasm `backtrace`, `supports-color`,
  `terminal_size`, `textwrap`, второй `miniz_oxide`; fancy-рендеринг нигде не
  используется — только `derive(Diagnostic)`.
- `tracing` используется в одном месте (`crates/docx/src/parse.rs:950`), но
  тянет `tracing-core`/`tracing-attributes` в `docx-wasm`; `tracing-wasm` не
  используется вовсе.
- Пять модулей — 7,6 МБ wasm raw; бюджеты gzip в сумме 3,2 МБ.
  `pptx-pdf-wasm` (2,2 МБ raw / 800 КБ gzip) почти целиком дублирует
  PDF-бэкенд из `pdf-wasm` (2,8 МБ / 1 050 КБ).

### 3.5. PPTX

- `pptx/src/parse.rs` — 1 175 строк без unit-тестов, 5 фикстур; своя модель
  предупреждений `Vec<String>` вместо `core::Warnings`; ошибки chart/pic
  глотаются молча.
- `pptx-layout` создаёт новый `FontRegistry` на каждый слайд
  (`pptx-layout/src/layout.rs:102`), клонирует `RunProps` на каждое слово
  (`:287-306`), сериализует блоб диаграммы в каждом кадре
  (`pptx-render/src/render.rs:93`).
- `p:style`, scheme-фон, поворот/обрезка не читаются
  (`pptx/src/parse.rs:706-729, 417, 763-773`); незнакомый `prstGeom`
  молча рисуется прямоугольником (`:776-785`).
- **Экспорт парсит пакет заново.** `pptx-wasm/export_api.rs:52-56` открывает
  презентацию на каждый экспорт; воркер копирует весь `sourceBytes`
  (`pptx_worker.ts:580, 783+`), а `pptx-pdf-wasm` парсит пакет ещё раз в
  другом модуле (`pptx-pdf-wasm/src/lib.rs:42`). До трёх парсингов и двух
  копий файла на один экспорт.
- **Копии на границе.** `pptx_open_bytes` обещает «без копирования»
  (`pptx-wasm/src/pptx_api.rs:291`), но делает `to_vec()` (`:297`);
  `export_pptx_pdf` принимает `&[u8]` (копия на границе wasm-bindgen) и
  сразу делает `to_vec()` (`pptx-pdf-wasm/src/lib.rs:42`).
- **Лимиты и идентификаторы потоков.** `stream_append` не ограничен ни
  размером, ни числом (`pptx-wasm/src/stream_api.rs:42-50`); `NEXT_ID`
  переполняется и может затереть живой буфер (`:29-31`).
- **Panic-hook не ставит ни один wasm-крейт** (grep пуст): паника видна как
  `unreachable executed`, а не как диагностируемая ошибка.
- **Размер и зависимости.** `pptx-wasm` — 527 809 B raw при бюджете 480 КБ
  gzip; `pptx-pdf-wasm` — 2 171 010 B raw / ~782 КБ gzip, и он тянет
  `doc-converter-pdf`, а тот — `docx` и `xlsx` ради `PdfExporter`, хотя
  PPTX-пути нужен только бэкенд. Слияние `pptx-pdf-wasm` с `pdf-wasm`
  предусматривал ADR-0033 §7/§9; цена — рост `pdf-wasm` для XLSX/DOCX-only
  потребителей. Нужен замер (ADR-0045, фаза D).
- **Документация.** Комментарий `.size-limit.mjs:112-114` говорит, что
  `export_pptx_pdf` живёт в `@meerrec/wasm-pdf`, что противоречит строке
  `:159`; ROADMAP в бюджетах говорит про «два wasm-модуля» и «пять строк».
- **Тесты.** Покрыты open/slides, кадры bytes/SAB, close/out-of-range,
  картинки, md/html/text, chunked stream, PDF на мусоре. Нет тестов отмены
  реальной многослайдовой раскладки, «съедания» флага после успеха,
  экспорта из открытой презентации, битых options, `written:false`/малого
  слота, лимитов потоков и паники.
- **`pptx-render` тонкий, но плагин мёртв.** `render::build` собирает кадр
  слайда (`pptx-render/src/render.rs:25-133`), `to_blob` диаграммы
  аллоцируется на каждый кадр (`:93`), culling нет. `PptxPlugin::parse`
  раскладывает все слайды, а `build_display_list` рисует только первый
  (`plugin.rs:31-98`); в продакшене плагин не используется — wasm зовёт
  `render::build` напрямую (`pptx-wasm/src/pptx_api.rs:199`), плагин живёт
  только в тестах.
- **`pptx-pdf` дублирует каркас экспорта DOCX почти построчно**
  (`pptx-pdf/src/lib.rs:100-178` против `pdf/src/docx/mod.rs:62-158`):
  раскладка, `collect_faces`, `collect_glyphs`, `EmbeddedFonts`, регистрация
  шрифтов, имена XObject, цикл страниц, `translate`/`finish`. Базовая линия
  текста разная: DOCX — `typo_ascent_px` (`pdf/src/docx/mod.rs:509`), PPTX —
  `registry.ascent` (`pptx-pdf/src/lib.rs:425`); гарнитура тоже расходится:
  раскладка мерит Carlito, canvas рисует `run.font`/«Calibri», PDF встраивает
  Carlito. Это риск SSIM и расхождения переносов.
- **`export_pptx_pdf` теряет опции.** `pptx-pdf-wasm/src/lib.rs:36-43`
  валидирует `options` и сразу передаёт `PdfOptions::default()`, теряя
  `compress`/метаданные. `Item` не несёт rotation/flip/alpha/gradient/shadow
  (`pptx-layout/src/layout.rs:52-71`) — фичи PowerPoint молча теряются.
- **Тесты и бенчи.** У `pptx-pdf` два теста, у `pptx-render` четыре; бенчей
  нет. ADR-0033 §7/§9 предписывал `pdf/src/pptx/` и `export_pptx_pdf` в
  `pdf-wasm`, а код завёл пятый wasm-пакет; комментарий
  `.size-limit.mjs:112-114` устарел.
- Отдельный `pptx-layout` (5 публичных элементов) даёт логическую границу,
  но dependency-изоляции нет: `pptx` уже зависит от `render`. Решение о
  слиянии — в ADR-0045/0046.

## 4. План рефакторинга и оптимизации

### Волна 0 — «Надёжность и гигиена» (1 спринт, низкий риск)

Цель: убрать реальные баги, не меняя раскладку.

1. **Viewer RPC:** единый pending-registry с `reject` для всех запросов,
   `onError`, `worker.onerror/messageerror`, сохранение `code`,
   `hitTest → CellRef` (ADR-0046).
2. **XLSX `value_of`:** trim только для Number/SharedString/Bool/Error;
   фикстура с пробелами в `t="str"` и `t="d"`.
3. **DisplayList:** единая таблица tag→size, `validate`, `pod_read_unaligned`,
   proptest (ADR-0043).
4. **SAB:** генерировать `HEADER_BYTES`/`SLOT_COUNT` из Rust или отдавать
   через wasm; кросс-тест Rust↔TS (ADR-0043).
5. **Протокол:** исчерпывающий dispatch per-format
   (`default: postError('internal')`), честный `cancel` по `id`, типизированная
   ошибка отмены вместо `[cancelled]`, `xlsx release` → закрытие книги
   (ADR-0046).
6. **PPTX:** конечные координаты при `wrap="none"`, `a:br`/`a:fld`,
   `core::Warnings` вместо `Vec<String>`; сброс и гарантированное гашение
   флага отмены, отмена экспорта, экспорт из уже открытой презентации без
   повторного парсинга; лимиты потоков и защита `NEXT_ID` от переполнения.
7. **DOCX numbering:** дочитывать листья и завершать по имени родителя;
   фикстура с парными тегами.
8. **PDF:** освобождать изображения после `add_image`, `export_docx_to<W>`,
   метаданные XLSX, честный статус tagged PDF.
9. **Мёртвый код и память модели:** удалить `color.rs`, `hit_test`,
   `ellipsize`, `SabWriter`; убрать `RawPPr::sect_pr` и дубль `mark_rpr` — с
   замером memory-тестами.
10. **Копии на границе wasm:** `append_owned(Vec<u8>)`, приём `Vec<u8>` в
    PDF-экспортах, `pptx_open_bytes` без `to_vec`, checked-арифметика
    `alloc_sab`, лимиты потоков, panic-hook во всех пяти модулях.
11. **Документация и CI:** `AGENTS.md` → 1.4.3, `VERSION` из package.json,
    ROADMAP/ADR-0013; строгий `--frozen-lockfile`, кэш `wasm-pack`, e2e
    только на одной версии Node, path-фильтры.

**Приёмка:** новые тесты на каждый баг; ни один SSIM/дифф-тест не меняется;
размеры модулей не растут.

### Волна 1 — «Структура» (2 спринта, средний риск)

Цель: убрать дублирование, не меняя поведение.

1. **Тесты out of line:** перенести `mod tests` девяти крупнейших файлов в
   отдельные модули — файлы сразу уменьшаются вдвое.
2. **Разрезать по швам:** `document.rs` →
   `parse/{blocks,tables,inline,drawing,props,run}`; `paint.rs` →
   `paint/{mod,borders,text,colors,headers,objects}`; `engine.rs` →
   `chrome.rs`/`float.rs`; `pdf/painter.rs` → `painter/{xlsx,docx,common}`.
3. **Единый слой рисования таблиц и общий каркас PDF-экспорта:**
   `render::table_paint` (заливка, рёбра, текст) для canvas и PDF; перенести
   фильтр vMerge на canvas; общий `pdf::export` для DOCX/PPTX (регистрация
   шрифтов, картинок, цикл страниц), согласовать `typo_ascent_px` и имя
   гарнитуры; SSIM-оракул как приёмка.
4. **Единый разбор `pPr/rPr`:** `styles::parse_p_pr/r_pr` для
   styles/numbering/document, одна таблица enum'ов; решить судьбу
   theme-шрифтов.
5. **`viewer_base`:** pending-registry, worker lifecycle, resize/scroll/rAF,
   экспорт; мигрировать три вьюера; стабильные `options`, состояния
   error/loading, паритет обёрток (ADR-0046).
6. **PPTX → `WorkerCore`:** расширить `format` до `DocFormat`, добавить hooks;
   убрать ~600 строк копии; общий `LazyWasmLoader`.
7. **`wasm-common`:** макросы/крейт для `thread_local`, error mapping,
   serde-конвертов; сократить 5 мостов (умеренный выигрыш, после п. 5–6).
8. **Гигиена API:** `pub(crate)` + `unreachable_pub`, workspace-lints; удалить
   мёртвые публичные типы.
9. **ADR:** разделение `pdf` на backend и форматные адаптеры (ADR-0045);
   решение по `pptx-layout` и слиянию `pptx-pdf-wasm` с `pdf-wasm`.

**Приёмка:** ни один снапшот/SSIM/дифф-тест не меняется; размеры модулей не
растут; файлов > 1 500 строк не остаётся.

### Волна 2 — «Оптимизация и контракты» (3–4 спринта, высокий эффект)

1. **Шрифты — ленивый общий ассет (ADR-0044):** вынести четыре сабсета из
   `render`, регистрировать через `font_api`; снять ~177 КБ gzip с каждого из
   пяти модулей и получить общий кэш между форматами.
2. **XLSX-кэши:** `visible_used_range` в `Sheet`; разобранные `NumberFormat`
   в `StyleTable`; `Thresholds`/`RuleIndex` дольше кадра; индекс
   гиперссылок/примечаний; один проход разрешения стиля на ячейку;
   дедупликация рёбер; clip только для переполнения; дедупликация string
   pool.
3. **DOCX:** убрать `finished.clone()` в `break_page`; один мерёж сегментов
   строки; `Arc<ResolvedRPr>`; кэш колонтитулов; `cell_borders` один раз на
   ряд; `merge_slots` интервалами; индекс media; префиксные суммы страниц в
   `stack_range` и `text_layer` (сейчас O(N) и O(P²)); ленивое семантическое
   дерево TextLayer.
4. **PDF:** ленивая пагинация, владеющее сжатие без `Stream::clone`,
   освобождение изображений, `export_docx_to<W>` и `export_pptx` с
   `export_to<W>`, честная передача `compress`/метаданных, убрать 2–3 копии
   content stream на страницу; кэш `chart::Layout` на слайд и общий
   `FontRegistry` раскладки PPTX; оживить или удалить мёртвый `PptxPlugin`.
5. **render/painter:** не аллоцировать CSS-строки на команду, один `SabRing`,
   чтение `DisplayListReader` прямо из слота, API `advances(text)` для
   шрифтов, кэш диаграмм, корректный резерв `to_bytes_into`.
6. **XML/Archive:** заимствованные events вместо `Event<'static>`, индекс
   имён в `Archive`, префиксные смещения в `SegmentedBuffer`; обязательно с
   бенчмарком `openDocx`/`openXlsx` до/после.
7. **Слияние PDF-модулей:** оценить объединение `pptx-pdf-wasm` в
   `pdf-wasm` — минус ~800 КБ gzip, пакет и CI-джоба; цена — PPTX-код в
   PDF-модуле для тех, кто его не использует. Решение — через ADR-0045 и
   замер.
8. **Стресс-тесты:** wasm-бенч-каркас (ROADMAP признаёт его отсутствие),
   1M ячеек в браузере 30 минут, low-end harness (issue #27), memory-гейт на
   плотном DOCX.

## 5. Приоритеты

| # | Действие | Эффект | Риск | Труд |
|---|---|---|---|---|
| 1 | Viewer pending/onError/worker.onerror | Вечные промисы и тихие падения исчезают | S | S |
| 2 | XLSX trim `t="str"`/`t="d"` | Порча данных устраняется | S | S |
| 3 | SAB: фолбэк при кадре > слота | «Замерший экран» исчезает | S–M | S |
| 4 | PPTX `inf` + `a:br`/`a:fld` | Видимая корректность PPTX | S | S |
| 5 | DOCX numbering `read_children` | Уровень не теряется на валидном XML | S | S |
| 6 | PDF: изображения + честный tagged PDF | Память и достоверность документации | S–M | S–M |
| 7 | XLSX: used range + numfmt + CF кэши | Главный прирост FPS на больших листах | M | M |
| 8 | DOCX: page clone + двойной мерёж | Скорость и память раскладки | S–M | S–M |
| 9 | DL validate + SAB кросс-тест | Контракты не расходятся молча | S–M | S–M |
| 10 | Тесты out of line + разрез god-файлов | Стоимость изменений, параллельные слайсы | S | M |
| 11 | viewer_base + PPTX → WorkerCore | −300…−600 строк, единый lifecycle | M | M–L |
| 12 | Шрифты/`miette`/`tracing`/слияние модулей | −0,9 МБ gzip дублирования | M–L | M–L |

## 6. Что сознательно не трогать

- **ADR-0002/0005:** `skrifa`, одна система ширин, bold меряется как regular.
- **ADR-0006:** жадный перенос, без Knuth-Plass/kerning/ligatures.
- **ADR-0007/0025:** нет общего `layout-core`; XLSX-раскладка остаётся в
  `xlsx`.
- **ADR-0011:** раскладка диаграмм в `render`, PDF — только бэкенд
  примитивов.
- **ADR-0015/0016:** лимиты ZIP, recover-and-continue, first-wins стилей.
- **ADR-0018/0020:** read-only, writer — non-goal.
- **ADR-0021/0029:** SAB/bytes first-class, main-fallback и singleton painter.
- **ADR-0022:** собственный PDF-бэкенд на модели `lopdf`, возврат printpdf
  запрещён.
- **ADR-0030/0031/0032/0033/0034/0037/0040/0041/0042:** trait-слой,
  streaming ZIP, отмена раскладки, PPTX, Strict, OTel, bidi без rustybuzz,
  TextLayer, RTL-шрифты.
- **Cell 48 Б и CSR** (`crates/xlsx/src/model.rs`, бюджетный тест) — не
  пересобирать.
- **WebGL и ODF** — владелец пропустил; не предлагать как основной путь.

## 7. Как измерять

- **Rust-бенчи:** `cargo bench -p doc-converter-xlsx --bench conditional`,
  `-p doc-converter-docx --bench open`, `-p doc-converter-pdf --bench pdf` —
  до/после каждой волны.
- **Размер:** `pnpm size-limit`; для шрифтов и слияния модулей — отдельный
  замер gzip и cold start. Для оценки вклада зависимостей:
  `cargo tree -e features -i miette`, `twiggy top` или
  `cargo bloat --target wasm32-unknown-unknown`.
- **Качество вёрстки:** `bash scripts/oracle-ssim.sh`,
  `scripts/oracle-docx-pages.sh`, `pnpm diff:mammoth`, differential exceljs.
- **Память:** `scripts/check-pdf-memory.sh`, `check-docx-memory.sh`.
- **Контракты:** proptest на DisplayList, wasm-bindgen-тест SAB↔TS, drift
  `wire.ts`.
- **Чего не хватает:** wasm-бенч-каркаса, 1M-стресс-теста в браузере,
  low-end harness (issue #27), тестов `run.ts`/PPTX-воркера. Это стоит
  добавить до Волны 2, иначе оптимизации нечем мерить.

## 8. Открытые вопросы

1. **Tagged PDF:** реализовывать K1–K6 или официально закрыть ADR-0012 как
   нереализованный? Решение влияет на публичные заявления о доступности.
2. **Слияние `pptx-pdf-wasm` с `pdf-wasm`:** нужен замер объединённого
   модуля; решение продуктовое (кто платит за PPTX-код в PDF-модуле).
3. **`pptx-layout`:** оставить отдельным крейтом как логическую границу или
   слить в `pptx-render`; dependency-изоляции сейчас нет.
4. **Отмена PDF-экспорта:** кооперативные проверки в синхронном экспорте или
   явное ограничение в ADR-0032.
5. **Theme-шрифты DOCX:** читать `asciiTheme`/`hAnsiTheme` из `theme1.xml`
   или зафиксировать потерю предупреждением.

## 9. Связанные решения

- [ADR-0043](../adr/0043-cross-language-contracts.md) — контракты DisplayList
  и SAB.
- [ADR-0044](../adr/0044-font-assets.md) — шрифты как ленивый общий ассет.
- [ADR-0045](../adr/0045-pdf-backend-boundary.md) — граница PDF-бэкенда и
  форматных адаптеров.
- [ADR-0046](../adr/0046-viewer-runtime.md) — единый runtime вьюеров,
  типизированные ошибки и отмена.
