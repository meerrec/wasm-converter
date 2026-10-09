# Покрытие `crates/core` и `crates/docx`: замер, добор, гейт

Пороги ROADMAP §9 (DoD спринта 8): **≥ 85 % строк для `crates/docx`, ≥ 80 %
строк для нового кода `crates/core`**. Документ прошёл три состояния и хранит
все три — первое объясняет, откуда взялся долг, второе показывает, чем он
закрыт:

- **E1 — замер до добора**: docx порог не проходил (82,31 %), не хватало
  269 строк; разделы «Что именно не покрыто», «Непокрытые ветви ошибок» и
  «Сколько не хватало» — снимок того состояния, он объясняет, куда целился E2;
- **E2a–E2c — добор тестами**: юнит-тесты в `src` трёх больших файлов
  (`document.rs`, `numbering.rs`, `styles.rs`);
- **E3 — гейт в CI**: джоба `coverage` с жёсткими `--fail-under-lines`.

Разделы «Числа», «Цена одного процента», «Худшие файлы», «Время прогона» и
«Вывод» обновлены до финального замера (E3); раздел «Гейт в CI» добавлен;
разделы-снимки E1 («Что именно не покрыто», «Непокрытые ветви ошибок»,
«Сколько не хватало») помечены и оставлены как есть — они объясняют добор.

Дата замера E1: 09.10.2026; дата финального замера E3: 09.10.2026, ветка
`sprint/8-docx-parse-fuzz`, HEAD `bfe9e94`.

## Протокол

Условия ниже — от замера E1; финальный прогон E3 отличался только состоянием
дерева и прогретым каталогом сборки (см. абзац в конце раздела).

- **Машина:** Apple M3 Pro, 11 потоков, macOS 26.6.1, arm64. Замер нативный,
  не wasm.
- **Инструмент:** `cargo llvm-cov` 0.9.1 (llvm-tools к тулчейну 1.98.0),
  rustc 1.98.0. `sccache`/`RUSTC_WRAPPER` не используются — числа сборки ниже
  честные, без внешнего кэша.
- **Каталоги сборки:** `target/llvm-cov` для основных прогонов и
  `target/llvm-cov-cold` для замера холодной сборки. В дереве параллельно шли
  четыре fuzz-прогона и другие слайсы, общий `target/` они держат сами.
- **Состояние дерева:** HEAD `861c46b`…`f9b950a` (коммиты параллельных слайсов
  шли во время замера), плюс незакоммиченные правки в `crates/docx/tests/`.
  Исходники крейтов в замере не двигались: свёртка sha256 по сортированному
  списку `*.rs` — `crates/core/src` `4df696e468b7b847`, `crates/docx/src`
  `b363f9b1a7ac2fb7`. Правки тестов на числа этого отчёта не влияют: мерились
  строки `src`, их состав и разметка за замер не менялись.
- **Тесты:** `cargo llvm-cov -p <крейт>` гоняет юнит-тесты и интеграционные
  (`tests/*.rs`). Доктесты не гоняются: `--doctests` не передавался.
- **Красных тестов нет.** Обе сюиты зелёные: core — 54 теста, docx — 227
  в прогоне E1, из которого взяты числа (196 lib + 3 fixtures + 4 memory +
  8 properties + 7 roundtrip + 9 warnings); в холодном прогоне `memory.rs`
  уже давал 5 тестов — 228, на покрытие `src` это не влияет. Параллельный
  слайс C2 успел починить `fixtures.rs` до замера.

Замер E3 (финальные числа) сделан тем же инструментом на **чистом дереве**
(HEAD `bfe9e94`, незакоммиченных правок в `crates/**` нет — параллельные слайсы
успели закоммититься до прогона), в прогретом каталоге `target/llvm-cov`:
сборка инструментированного дерева была только инкрементальной, поэтому
времена ниже — тёплые, а не «как в CI». Сюита docx выросла до 280 тестов
(247 lib + 1 differential + 3 fixtures + 5 memory + 8 properties + 7 roundtrip
+ 9 warnings), core не менялся — 54.

## Числа

Из `--summary-only` (команды — в разделе «Как воспроизвести»):

«До добора» — замер E1, «после добора» — финальный замер E3:

| Крейт | Строк | Непокрыто | Строк % | Функций (непокрыто) | Функций % | Регионов % | Тестов | Порог | Вердикт |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `doc-converter-core`, до добора | 947 | 22 | 97,68 % | 133 (6) | 95,49 % | 96,50 % | 54 | 80 % | взят |
| `doc-converter-core`, после | 947 | 22 | **97,68 %** | 133 (6) | 95,49 % | 96,50 % | 54 | 80 % | **взят**, код 0 |
| `doc-converter-docx`, до добора | 9988 | 1767 | 82,31 % | 790 (116) | 85,32 % | 78,42 % | 227 | 85 % | не взят, код 1 |
| `doc-converter-docx`, после | 11546 | 837 | **92,75 %** | 867 (87) | 89,97 % | 90,09 % | 280 | 85 % | **взят**, код 0 |

Запас до порога после добора: core — 17,68 п.п. (167 строк), docx — 7,75 п.п.
(895 строк). Запас большой и по построению устойчивый (числа покрытия
детерминированы), поэтому пороги стоят жёстко, без «дежурной» правки при
каждой новой ветке.

Знаменатель docx вырос с 9988 до 11546 строк (+1558) — это юнит-тесты E2,
легшие внутрь `src` (lib-тестов 196 → 247): как и всё в `src`, они входят в
счёт. Непокрытых строк при этом стало 837 против 1767 — добор перебил рост
знаменателя.

Дополнительно (замер E1): прогон обоих крейтов в одном `cargo llvm-cov` (оба
сюита в одном профиле) давал по core **98,63 %** (13 непокрытых строк) против
97,68 % в одиночном прогоне, по docx — те же 82,31 %. То есть docx-тесты
исполняют часть `crates/core`, которую не исполняют юнит-тесты core
(`XmlReader::preserving`, путь `Error::Xml` в `next_significant`). Поэтому
гейты в CI разведены по отдельным прогонам: порог привязан к крейту, а общий
прогон смешал бы числа — для core в сторону завышения.

## Гейт в CI

Джоба `coverage` в `.github/workflows/ci.yml`; оба порога жёсткие, каждый
крейт мерится своим прогоном:

```yaml
- name: core ≥ 80% lines
  run: cargo llvm-cov -p doc-converter-core --fail-under-lines 80
- name: docx ≥ 85% lines
  run: cargo llvm-cov -p doc-converter-docx --fail-under-lines 85
```

- **Почему жёстко.** Числа покрытия, в отличие от времён и памяти, не шумят:
  один и тот же набор тестов даёт один и тот же процент. Красный шаг здесь —
  регрессия (упавший или удалённый тест, новая непокрытая ветка), а не
  капризы раннера, поэтому мягкий режим, как у временных гейтов `perf`, тут
  только маскировал бы регрессии.
- **`components: llvm-tools-preview`** в шаге `dtolnay/rust-toolchain` —
  требование `cargo-llvm-cov` (`llvm-profdata`/`llvm-cov` из sysroot).
  Компонент продублирован в `rust-toolchain.toml` для локальных прогонов:
  в CI rustup идёт от `RUSTUP_TOOLCHAIN`, который выставляет action, и файл
  проекта не читает — одного места мало.
- **`--target-dir` в джобе нет** — в cargo-llvm-cov 0.9.1 такой опции не
  существует (`error: invalid option`), а на своём раннере дефолтный `target/`
  ни с кем не делится. Локально каталог задаётся через `CARGO_TARGET_DIR`.
- **`timeout-minutes: 40`** — с запасом: локально тёплый прогон обоих крейтов
  занимает 26–34 с (см. «Время прогона»), но в CI к этому добавляются установка
  `cargo-llvm-cov` из исходников и холодная инструментированная сборка на
  2–4 vCPU.
- **Вне гейта осознанно, а не по недосмотру:**
  - `crates/fuzz` и `vendor/printpdf` — отдельные workspace'ы (nightly +
    санитайзер и вендоренный форк), не библиотечный код, ROADMAP §9 их не
    адресует;
  - `crates/wasm` — целиком под `#![cfg(target_arch = "wasm32")]`, на нативном
    таргете пуст, мерить нечего;
  - `crates/render`, `crates/xlsx`, `crates/pdf` — порога в ROADMAP §9 не
    имеют;
  - доктесты — `--doctests` в cargo-llvm-cov помечен unstable и не передаётся.

## Цена одного процента

| Крейт | Исполняемых строк | 1 п.п. | Функций | 1 п.п. |
| --- | ---: | ---: | ---: | ---: |
| core | 947 | 9,5 строки | 133 | 1,3 функции |
| docx | 11546 | 115,5 строки | 867 | 8,7 функции |

Числа — уже после добора. Запас в строках до порога: core — 80 % от 947 это
758 покрытых при 925 фактических, то есть **ещё 167 непокрытых строк** можно
позволить; docx — 85 % от 11546 это 9814 при 10709, то есть **ещё 895 строк**.
Столько нового кода без тестов помещается в крейт, прежде чем гейт покраснеет.

Дефицит E1 (269 строк до порога) закрыт добором тестов, а не удалением мёртвого
кода: непокрытые строки лежали в живых ветвях парсеров.

## Худшие файлы

docx **после добора** (колонки — из summary; всего 18 файлов, у пяти из них
0 непокрытых строк):

| Файл | Строк | Непокрыто | Строк % | Было в E1 | Функций (непокрыто) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `src/document.rs` | 4256 | **453** | 89,36 % | 80,10 % | 262 (29) |
| `src/styles.rs` | 1927 | **114** | 94,08 % | 72,69 % | 106 (4) |
| `src/numbering.rs` | 1856 | **70** | 96,23 % | 67,08 % | 127 (13) |
| `src/xml.rs` | 771 | 44 | 94,29 % | 94,29 % | 78 (9) |
| `src/parse.rs` | 657 | 40 | 93,91 % | 93,91 % | 69 (5) |
| `src/footnotes.rs` | 384 | 28 | 92,71 % | 92,71 % | 35 (6) |
| `src/comments.rs` | 315 | 23 | 92,70 % | 92,70 % | 31 (6) |
| `src/settings.rs` | 371 | 19 | 94,88 % | 94,88 % | 33 (1) |
| `src/metadata.rs` | 191 | 17 | 91,10 % | 91,10 % | 24 (6) |
| `src/model/raw.rs` | 73 | 15 | 79,45 % | 79,45 % | 18 (5) |
| `src/rels.rs` | 183 | 6 | 96,72 % | 96,72 % | 24 (2) |
| `src/context.rs` | 137 | 4 | 97,08 % | 97,08 % | 16 (1) |
| `src/model/validate.rs` | 256 | 4 | 98,44 % | 98,44 % | 19 (0) |
| `src/error.rs`, `src/lib.rs`, `src/model/{mod,numbering,style}.rs` | 169 | 0 | 100 % | 100 % | 25 (0) |

Добор E2 виден по колонке «Было в E1»: три файла, в которые легли юнит-тесты,
поднялись на 9–29 п.п. (`numbering.rs` 67,08 → 96,23, `styles.rs` 72,69 →
94,08, `document.rs` 80,10 → 89,36), остальные не двигались — и не должны были.
Знаменатели этих трёх выросли (3764 → 4256, 1362 → 1927, 1355 → 1856 строк):
в `src` легли тестовые модули, они входят в счёт наравне с кодом.

**Три файла держат 637 из 837 непокрытых строк (76 %):** `document.rs`,
`styles.rs`, `numbering.rs`. Долг после добора — почти весь `document.rs`
(453 строки: ветви ошибок и предупреждений парсеров, см. снимок E1 ниже).

core (тоже из summary, для полноты; числа те же, что в E1: `crates/core/src`
между замерами не менялся):

| Файл | Строк | Непокрыто | Строк % |
| --- | ---: | ---: | ---: |
| `src/xml.rs` | 31 | 8 | 74,19 % |
| `src/archive.rs` | 277 | 9 | 96,75 % |
| `src/rels.rs` | 187 | 5 | 97,33 % |
| `src/{error,node_id,warning,zip_limits}.rs` | 452 | 0 | 100 % |

## Что именно не покрыто: docx

> **Снимок E1** (состояние 82,31 %, до добора E2a–E2c). Разделы ниже
> объясняют, куда целился добор и почему он был дешёвым; текущие числа — в
> таблицах выше. Разбор не переписывался: после E2 адреса непокрытых строк
> сместились, а часть перечисленного закрыта тестами.

Ниже — непокрытые строки по функциям (из `--show-missing-lines`; в скобках —
число вызовов функции: `0` — функция не вызывалась ни разу).

### `document.rs`, 697 строк вне тестового модуля (из 749 в summary)

| Функция (строка) | Непокрыто | Вызовов |
| --- | ---: | ---: |
| `parse_shading_pattern` (3583) | 36 | 6 |
| `parse_table_children` (769) | 34 | 18 |
| `parse_border_style` (3548) | 22 | 66 |
| `parse_inline_element` (1868) | 21 | 14 |
| `parse_position_value` (2641) | 21 | 0 |
| `parse_borders` (3137) | 17 | 1 |
| `parse_highlight` (3402) | 16 | 1 |
| `parse_underline` (3439) | 16 | 3 |
| `parse_cell_borders` (1427) | 15 | 2 |
| `parse_cell_margins` (1475) | 15 | 2 |
| `parse_columns` (454), `parse_tbl_grid` (911), `parse_row` (956), `parse_num_pr` (3058) | по 14 | 1 / 10 / 63 / 2 |
| `scan_drawing_element` (2486), `read_number` (2814), `parse_rel_from_h/v` (2694/2720) | 8–12 | 0 |
| `parse_ppr` (2965), `parse_run_content` (2184), `parse_tc_pr` (1125), `parse_color` (3376) | 6–12 | ≥46 |

Оставшееся — по 1–6 строк: `InvalidAttribute`-ветки `attr_i64`, `parse_row_height`,
`parse_cell_v_align`, `parse_v_merge`, `parse_table_layout/width`, `parse_tab_stop`,
`parse_shading`, `parse_symbol`, `parse_justification`, `parse_vert_align`,
`parse_border`, `parse_sect_pr`, `parse_orientation`, `parse_section_type`,
`parse_part_ref` и `UnknownElement`-ветки парсеров свойств.

### `numbering.rs`, 419 строк вне тестов

| Функция (строка) | Непокрыто | Вызовов |
| --- | ---: | ---: |
| `shading_pattern` (1119) | 42 | 0 |
| `border_style` (1085) | 31 | 0 |
| замыкание `read_children` в `parse_abstract_num` (872) | 30 | 0 |
| замыкание `read_children` в `parse_lvl` (701) | 22 | 0 |
| `highlight` (1018) | 21 | 0 |
| `parse_vert_align` (1048) | 19 | 0 |
| замыкания `read_children` (802, 583, 431, 651) | 13–18 | 0–23 |
| `tab_stop_kind` (1164) | 13 | 0 |
| `attr_outline_lvl` (774), `parse_border` (832) | по 12 | 0 |
| `parse_override` (493) | 18 | 7 |

### `styles.rs`, 363 строки вне тестов

| Функция (строка) | Непокрыто | Вызовов |
| --- | ---: | ---: |
| `shading_pattern` (1430) | 41 | 0 |
| `parse_p_bdr` (1002) | 25 | 0 |
| `parse_p_pr` (747) | 24 | 79 |
| `border_style` (1397) | 21 | 14 |
| `highlight` (1498) | 20 | 0 |
| `underline` (1474) | 17 | 1 |
| `parse_tabs` (978) | 15 | 0 |
| `parse_shading` (1258) | 13 | 0 |
| `tab_stop_kind` (1539) | 12 | 0 |
| `parse_conditional` (545) | 12 | 4 |
| `parse_r_pr` (831), `parse_tbl_pr` (896) | по 11 | 136 / 4 |
| `table_style_condition` (1566) | 11 | 3 |
| `parse_style` (425) | 9 | 192 |

**Главный вывод по трём файлам:** основная масса непокрытого — не ветки ошибок,
а таблицы «значение атрибута → enum», продублированные в `document.rs`,
`numbering.rs` и `styles.rs`: `shading_pattern`, `border_style`, `highlight`,
`underline`, `tab_stop_kind`, `parse_vert_align`, `table_style_condition`.
Их 15 экземпляров дают ≈ 338 непокрытых строк — этого одного хватает на
дефицит в 269. Ни одна фикстура не приносит эти значения (Shading pattern,
цвета подсветки, виды подчёркивания, tab stop kinds), поэтому функции
`shading_pattern`/`border_style`/`highlight` в `numbering.rs` и `styles.rs`
имеют **0 вызовов**. Дешевле всего E2 закрывается фикстурами с этими
атрибутами (или юнит-тестами самих маппингов), а не разбором `UnknownElement`
по всему `document.rs`.

### Прочие файлы docx

- `xml.rs` (21 строка вне тестов): непокрыты ветви `attr_toggle` (169),
  `attr_f32` (222), `parse_attr` (245) — все три `InvalidAttribute`; ветвь
  `DeepNesting` в `capture_element` (289/291) и `resolve_block` (453);
  `skip_element` → `_ => {}` (321); `push_event`: `CData` (338–342),
  `Comment` (344–348), `Decl|PI|DocType|Eof` (357); ошибка «unclosed
  `mc:Choice`» (472); `matching_end` → `None` (558).
- `parse.rs` (33): `optional_part` → `WarningKind::MissingPart` (226–230);
  `parse_frame` → `Err(malformed("`w:hdr`/`w:ftr` is missing"))` (501);
  `walk_paragraph` → `WarningKind::MissingNumId` (630–635); `walk_table` →
  `WarningKind::MissingStyleRef` (688–693); `walk_table` не вызывался ни разу;
  `log_warnings` (819–823).
- `footnotes.rs` (15): `parse` → `Err(malformed("unexpected end of input inside
  `{container}`"))` (61–64) и хвост `open_container` (96, 105); `find_container`
  → `Err(malformed("`w:footnotes` or `w:endnotes` is missing"))` (153–156);
  `note_kind` → `ContinuationNotice` (211); `skip_element` (175).
- `comments.rs` (10): `parse` → `Err(malformed("unexpected end of input inside
  `{CONTAINER}`"))` (49–52); `open_container` → `Err(malformed("`w:comments`
  is missing"))` (128); `Event::Empty` (56), `skip_element` (138/147).
- `settings.rs` (18): `xml_error` → `other => Error::from(other)` (46);
  `parse_note_props` → `Err(malformed("unexpected end of input"))` (163);
  `parse_compat` → то же (219) и ветки пропуска элементов (153–159, 179,
  207–215); `character_spacing_control` → `CompressPunctuationAndJapaneseKana`
  (317); `endnote_pos` → `Other` (339); `CompatFlag::DoNotBreakWrappedTables`
  (288).
- `metadata.rs` (11): `parse_core` → `b"category"` (79) и `_ => {}` (85);
  `element_text` → `Err(malformed("bad CDATA"))` (147), ветка `CData` (144–146),
  `depth`-ветвление (151, 153–154); `revision` (123).
- `model/raw.rs` (15, функция не покрыта целиком, 0 вызовов): `value()` и
  `From<i32>`/`From<String>` у двух newtype-типов (51–59, 670–678).
- `context.rs` (4): `ParseCtx::warn` → ветка `None => warning` (93),
  `impl Default` (133–135).
- `model/validate.rs` (4): хвосты `)?` четырёх проверок (88, 97, 114, 143).
- `rels.rs` (1 вне тестов): хвост `ctx.warn(WarningKind::MissingRels, …)?`
  (45) — предупреждение в тестах выдаётся, непокрыт путь ошибки `?`
  (`TooManyWarnings`).

Отдельно: **70 из 1653** строк списка лежат в `#[cfg(test)] mod tests` внутри
`src` (паникуют проверки, хелперы). Они входят в знаменатель покрытия, но
добирать их тестами бессмысленно — это цена того, что юнит-тесты живут в `src`.

## Непокрытые ветви ошибок

> **Снимок E1**, как и предыдущий раздел. После добора эти ветви частично
> покрыты; остаток по-прежнему лежит в основном здесь — в `document.rs` (453
> непокрытых строки из 837 по крейту).

Файл → функция → что не исполнялось ни разу (в скобках — строка):

- `crates/core/src/archive.rs`:
  - `Archive::validate_ooxml` — `Err(Error::MissingPart(ROOT_RELS))` (204):
    пакет без `_rels/.rels`; тест есть только на отсутствие
    `[Content_Types].xml` (тест `validate_ooxml_needs_content_types`);
  - строки 85–87 — `Archive::limits()`, аксессор не вызывается ни одним тестом.
- `crates/core/src/rels.rs`: `RelMap::parse` — `Err(Error::Malformed("incomplete
  <Relationship/>"))` (70); плюс хвосты 55 и 144.
- `crates/core/src/xml.rs` (в одиночном прогоне core): `XmlReader::preserving`
  (30–32) — юнит-тесты ядра им не пользуются; `next_significant` — замыкание
  `Error::Xml { .. }` (55–58). В объединённом профиле с docx обе ветви покрыты.
- `crates/docx/src/rels.rs`: `load` — хвост `ctx.warn(MissingRels, …)?` (45).
- `crates/docx/src/metadata.rs`: `element_text` — `Err(malformed("bad CDATA"))`
  (147).
- `crates/docx/src/settings.rs`: `xml_error` — `other => Error::from(other)`
  (46); `parse_note_props` — `Err(malformed("unexpected end of input"))` (163);
  `parse_compat` — то же (219).
- `crates/docx/src/parse.rs`: `optional_part` — `WarningKind::MissingPart`
  (226–230); `parse_frame` — `Err(malformed("`w:hdr`/`w:ftr` is missing"))`
  (501); `walk_paragraph` — `WarningKind::MissingNumId` (630–635);
  `walk_table` — `WarningKind::MissingStyleRef` (688–693), сама функция
  не вызывалась.
- `crates/docx/src/xml.rs`: `InvalidAttribute` — `attr_toggle` (169),
  `attr_f32` (222), `parse_attr` (245); `DeepNesting` — `capture_element`
  (289/291), `resolve_block` (453); `Err(malformed("unclosed `mc:Choice`"))`
  (472).
- `crates/docx/src/document.rs` (111 строк-ветвей): `Err(malformed(...))` в
  `parse_blocks` (75), `parse_sect_pr` (321), `parse_columns` (463),
  `parse_table_children` (779), `parse_tbl_pr` (862), `parse_tbl_grid` (920),
  `parse_row` (976), `parse_tr_pr` (1025), `parse_cell` (1089), `parse_tc_pr`
  (1134), `parse_table_borders` (1388), `parse_cell_borders` (1436),
  `parse_cell_margins` (1484), `parse_inline_content` (1839), `parse_run` (2136),
  `parse_run_fragment` (2294), `scan_drawing` (2464), `scan_drawing_element`
  (2558), `parse_position_value` (2650), `read_text` (2907), `parse_ppr` (2976),
  `parse_num_pr` (3067), `parse_borders` (3146), `parse_tabs` (3215),
  `parse_rpr` (3305), `skip_element` (3746), `expect_fragment_root` (3762);
  `WarningKind::UnknownElement` в `parse_table_children` (824, 835),
  `parse_columns` (483), `parse_tbl_grid` (939), `parse_row` (998),
  `parse_run_content` (2240), `parse_num_pr` (3086), `parse_borders` (3167),
  `parse_cell_borders` (1457), `parse_cell_margins` (1504), `parse_table_borders`
  (1409), `parse_tabs` (3233); `WarningKind::DeepNesting` в `parse_inline_element`
  (1904) и `scan_drawing_element` (2497); `InvalidAttribute` — 25 мест
  (`parse_orientation` 544, `parse_section_type` 575, `parse_part_ref` 508,
  `parse_cell_v_align` 1330/1342, `parse_v_merge` 1203, `parse_row_height`
  1298/1310, `parse_table_layout` 1222, `parse_table_width` 1252,
  `parse_cell_width` 1278, `parse_color` 3388, `parse_highlight` 3408,
  `parse_vert_align` 3477, `parse_justification` 3501, `parse_border` 3187,
  `parse_tab_stop` 3252, `parse_shading` 3273, `parse_symbol` 2864,
  `attr_i64` 2801, `attr_range` 3680, `read_number` 2830,
  `parse_rel_from_h/v` 2696/2722, `parse_position_value` 2676,
  `parse_sect_pr` 377, `parse_tc_pr` 1150).

Замечание: у части строк в списке непокрыт не сам вызов, а **хвост `)?`** —
то есть путь ошибки у `ctx.warn(...)?` (`TooManyWarnings`). Это отдельный
сценарий: парсеры рассчитаны на превышение порога предупреждений ADR-0016 §6,
и ни один тест его не доводит.

## Сколько не хватало и за счёт чего добирали (план E1)

До порога docx не хватало **269 исполняемых строк**. Ориентиры для E2, по
убыванию отдачи на тест:

1. **Таблицы «атрибут → enum»** (≈ 338 строк, см. выше) — 15 дублированных
   функций в `document.rs`/`numbering.rs`/`styles.rs`. Покрываются либо
   фикстурами с этими значениями (Shading pattern, highlight, underline,
   tab stops, vert align), либо прямыми юнит-тестами маппингов. Одного этого
   пункта хватает на весь дефицит.
2. **Функции с нулём вызовов**: `parse_position_value` (21 строка),
   `scan_drawing_element` (12), `read_number` (12), `parse_rel_from_h/v` (по 8),
   `parse_p_bdr` (25), `parse_tabs` (15), `parse_shading` (13) в `styles.rs`,
   замыкания `read_children` в `numbering.rs` (до 30 строк). Это не ветки —
   тесты просто не заходят в эти элементы.
3. **Ветви ошибок и предупреждений** из списка выше (≈ 111 строк по
   `document.rs` + 40 по остальным) — дают меньше всего: строки короткие
   (`return Err(`), а тестов на каждый путь нужно по одному. Брать их стоит
   только вместе с пунктами 1–2 (те же фикстуры их и закроют).

Плата за порог в docx: 228 существующих тестов дают 82,31 %; для 85 % нужен
добор ≈ 269 строк — это, по порядку величины, покрытие одного из трёх больших
файлов на 20–25 %.

**Чем закончилось (E2a–E2c):** добор пошёл по пункту 1 — юнит-тесты на
таблицы «атрибут → enum» в `document.rs`, `numbering.rs` и `styles.rs`, — и
этого пункта хватило с запасом: docx 92,75 %, в резерве до порога 895
непокрытых строк. Новые фикстуры не понадобились.

## Время прогона

Числа E1 (холодная сборка — для оценки таймаута), локально:

| Этап | Время |
| --- | ---: |
| Сборка инструментированного core (пустой `target-dir`) + 54 теста | 19 с |
| Сборка инструментированного docx поверх + 227 тестов | 30 с |
| **Холодная сборка обоих крейтов в чистом каталоге + 282 теста** | **40 с** |
| Отчёты `report` без прогона тестов (сводка, таблица, missing lines, JSON) | 1–2 с каждый |
| Весь сценарий замера целиком (2 прогона + 7 отчётов + 2 JSON) | 54 с |

Числа E3 (финальный замер; дерево и `~/.cargo` прогреты E1/E2, сборка
инкрементальная — так гейт выглядит на повторном локальном прогоне):

| Этап | Время |
| --- | ---: |
| core: пересборка + 54 теста + отчёт | 3–5 с |
| docx: пересборка + 280 тестов + отчёт | 21–31 с |
| **Оба гейта подряд** | **26–34 с** |

Разброс — не случайный: два прогона подряд шли на машине, где параллельно
работали другие слайсы; оба дали одни и те же проценты (покрытие
детерминировано), разошлось только время.

Это локальные числа M3 Pro с прогретым `~/.cargo`; на CI-раннере (2–4 vCPU)
сборка будет заметно дольше, но порядок — минуты, не десятки минут: холодные
40 с локально плюс `cargo install cargo-llvm-cov`. Отсюда `timeout-minutes: 40`
в джобе — запас на порядок величины, а не впритык. Инструментированное дерево
занимает ≈ 350 МБ на оба крейта.

## Как воспроизвести

В `cargo-llvm-cov` 0.9.1 **нет опции `--target-dir`** (`error: invalid option`);
каталог задаётся переменной окружения. Рабочие команды:

```bash
export CARGO_TARGET_DIR=$PWD/target/llvm-cov

cargo llvm-cov -p doc-converter-core --summary-only
cargo llvm-cov -p doc-converter-docx --summary-only
cargo llvm-cov -p doc-converter-core            # таблица по файлам
cargo llvm-cov -p doc-converter-docx
cargo llvm-cov -p doc-converter-core --show-missing-lines   # адреса непокрытых строк
cargo llvm-cov -p doc-converter-docx  --show-missing-lines
cargo llvm-cov -p doc-converter-core --fail-under-lines 80; echo "код=$?"   # 0
cargo llvm-cov -p doc-converter-docx  --fail-under-lines 85; echo "код=$?"   # 0
```

Последние две команды — ровно то, что делает джоба `coverage` в CI; числа
таблиц выше сняты теми же командами с добавкой `--summary-only` (только
итоговые проценты, без таблицы по файлам). `CARGO_TARGET_DIR` нужен лишь
локально, чтобы не толкаться в общий `target/`; в CI дефолтного каталога
достаточно, и опции `--target-dir` у cargo-llvm-cov нет.

Разделение прогона и отчёта (замер E1; так считался объединённый профиль двух
сюитов):

```bash
cargo llvm-cov -p doc-converter-core -p doc-converter-docx --no-report
cargo llvm-cov report -p doc-converter-core --summary-only   # 98,63 %
cargo llvm-cov report -p doc-converter-docx --summary-only   # 82,31 %
```

Холодный замер (как это увидит CI):

```bash
CARGO_TARGET_DIR=$PWD/target/llvm-cov-cold \
  cargo llvm-cov -p doc-converter-core -p doc-converter-docx --no-report
```

Числа E1 — из прогонов в `target/llvm-cov` (09.10.2026, 20:44–20:47 по
локальному времени) и `target/llvm-cov-cold` (20:49); финальный замер E3 — там
же в `target/llvm-cov`, 09.10.2026, 21:07–21:08. JSON-выгрузки и логи лежат
локально, в репозиторий не добавлялись.

## Чего в замере нет

Ничто из перечисленного ниже не входит и в CI-гейт `coverage` — список
совпадает намеренно.

- **`crates/fuzz`** — отдельный workspace (nightly + sanitizer, свой
  `Cargo.lock` и `target/`), исключён из общего workspace в `Cargo.toml`. В
  замер не входит — и это осознанное решение, а не пропуск: fuzz-крейт не
  библиотечный код и в порог ROADMAP §9 не входит.
- **`vendor/printpdf`** — тоже исключён из workspace (вендоренный форк), в
  замер не входит.
- **wasm-нога**: `crates/wasm` целиком под `#![cfg(target_arch = "wasm32")]`, на
  нативном таргете он пуст → в замер не попадает. `crates/render`, `crates/xlsx`,
  `crates/pdf` в этом срезе не мерились (порог ROADMAP §9 задан только для
  `crates/docx` и нового кода `crates/core`).
- **Доктесты** — `--doctests` не передавался (в cargo-llvm-cov он помечен
  unstable).
- **`crates/docx/tests/*`** — строки интеграционных тестов не входят в отчёт
  (`-p` фильтрует по файлам крейта); юнит-тесты внутри `src` входят и в E1
  давали 70 непокрытых строк из 1653. После E2 тестовые модули в `src` —
  заметная доля знаменателя (см. рост 9988 → 11546); правкой их не убрать, не
  потеряв сами тесты, но и порог они держат с запасом.

## Сомнения и ограничения

- Число в колонке `Missed Lines` чуть больше числа строк в списке
  `--show-missing-lines` (в E1 у docx — 1767 против 1653): llvm-cov считает эти
  величины по-разному. Процент — от summary, адреса для правки — из списка.
  Для гейта это неважно: `--fail-under-lines` берёт процент из summary.
- Прогон идёт по `debug`-профилю с `-C instrument-coverage` — на разметку
  строк это не влияет, на время сборки влияет.
- `--fail-under-lines` в E1 применялся к `cargo llvm-cov report`; код возврата 1
  у docx — это именно недобор порога, тесты при этом зелёные. В E3 (и в CI) та
  же опция стоит в режиме прогона: обе команды вернули 0.
- Один прогон на крейт, без повторов: llvm-cov детерминирован на одном и том же
  наборе тестов, разброса, как у времён/памяти в других отчётах, здесь нет.
- Дерево во время замера E1 двигали параллельные слайсы (тесты и CI-скрипты, не
  `src`); воспроизведение на текущем HEAD может дать другие числа по тестовым
  файлам — на покрытие `src` это не влияет. Замер E3 сделан на чистом дереве
  (HEAD `bfe9e94`), поэтому от этого эффекта свободен.
- Времена E3 тёплые (инкрементальная сборка поверх `target/llvm-cov` от
  E1/E2) — для холодного прогона ориентир даёт таблица E1 выше, а `timeout-minutes`
  в джобе считается по ней.
- Гейт измеряет строки (`--fail-under-lines`), а не функции и регионы: так
  задан порог в ROADMAP §9. Функции по docx после добора — 89,97 %, регионы
  90,09 %; по core — 95,49 % и 96,50 %. Они печатаются в отчёте, но вердикт
  выносят не они.

## Вывод

- **Оба порога взяты, гейт в CI стоит жёстко:** core — 97,68 % строк (порог
  80 %), docx — 92,75 % (порог 85 %); `--fail-under-lines` вернул 0 в обоих
  прогонах, а джоба `coverage` повторяет эти команды один в один.
- Добор E2a–E2c поднял docx на 10,44 п.п. (82,31 → 92,75) и закрыл дефицит в
  269 строк: непокрытых стало 837 против 1767. Порог взят юнит-тестами на
  таблицы «атрибут → enum», без новых фикстур — как и планировал разбор E1
  (пункт 1 списка «за счёт чего добирать», ≈ 338 строк одним куском).
- Знаменатель при этом вырос с 9988 до 11546 строк: тестовые модули в `src`
  входят в счёт наравне с кодом. Запас это выдержал — до порога остаётся 895
  непокрытых строк.
- Остаток долга — `document.rs` (453 из 837 непокрытых строк: ветви ошибок и
  предупреждений парсеров), за ним `styles.rs` (114) и `numbering.rs` (70).
  Гейт держится и без них: это задел на будущее, а не незакрытый пункт DoD.
