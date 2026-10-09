# E1: покрытие `crates/core` и `crates/docx`

Замер перед слайсом E2. Цель — числа под пороги ROADMAP §9: **≥ 85 % строк для
`crates/docx`, ≥ 80 % строк для нового кода `crates/core`** (план спринта,
R1). Тесты в этом слайсе не добавлялись, CI не трогался: только замер и список
того, что покрыто не полностью.

Дата замера: 09.10.2026, ветка `sprint/8-docx-parse-fuzz`.

## Протокол

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
  в прогоне, из которого взяты числа (196 lib + 3 fixtures + 4 memory +
  8 properties + 7 roundtrip + 9 warnings); в холодном прогоне `memory.rs`
  уже давал 5 тестов — 228, на покрытие `src` это не влияет. Параллельный
  слайс C2 успел починить `fixtures.rs` до замера.

## Числа

Из `--summary-only` (команды — в разделе «Как воспроизвести»):

| Крейт | Строк (непокрыто) | Строк % | Функций (непокрыто) | Функций % | Регионов % | Порог | Вердикт |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `doc-converter-core` | 947 (22) | **97,68 %** | 133 (6) | 95,49 % | 96,50 % | 80 % | **взят**, код возврата 0 |
| `doc-converter-docx` | 9988 (1767) | **82,31 %** | 790 (116) | 85,32 % | 78,42 % | 85 % | **не взят**, код возврата 1 |

Дополнительно: прогон обоих крейтов в одном `cargo llvm-cov` (оба сюита в одном
профиле) даёт по core **98,63 %** (13 непокрытых строк), по docx — те же
82,31 %. То есть docx-тесты исполняют часть `crates/core`, которую не исполняют
юнит-тесты core (`XmlReader::preserving`, путь `Error::Xml` в
`next_significant`); `--workspace`-прогон покажет по core не меньше 97,68 %.

## Цена одного процента

| Крейт | Исполняемых строк | 1 п.п. | Функций | 1 п.п. |
| --- | ---: | ---: | ---: | ---: |
| core | 947 | 9,5 строки | 133 | 1,3 функции |
| docx | 9988 | 99,9 строки | 790 | 7,9 функции |

Дефицит docx до 85 %: `0,85 × 9988 = 8490` покрытых строк, сейчас 8221, —
**не хватает 269 исполняемых строк** (2,69 п.п.). За счёт одного лишь удаления
мёртвого кода планку не взять: непокрыто 1767 строк, но они лежат в живых
ветвях парсеров.

## Худшие файлы

docx (колонки — из summary; всего 18 файлов, у пяти из них 0 непокрытых строк):

| Файл | Строк | Непокрыто | Строк % | Функций (непокрыто) |
| --- | ---: | ---: | ---: | ---: |
| `src/document.rs` | 3764 | **749** | 80,10 % | 234 (33) |
| `src/numbering.rs` | 1355 | **446** | 67,08 % | 98 (30) |
| `src/styles.rs` | 1362 | **372** | 72,69 % | 86 (12) |
| `src/xml.rs` | 771 | 44 | 94,29 % | 78 (9) |
| `src/parse.rs` | 657 | 40 | 93,91 % | 69 (5) |
| `src/footnotes.rs` | 384 | 28 | 92,71 % | 35 (6) |
| `src/comments.rs` | 315 | 23 | 92,70 % | 31 (6) |
| `src/settings.rs` | 371 | 19 | 94,88 % | 33 (1) |
| `src/metadata.rs` | 191 | 17 | 91,10 % | 24 (6) |
| `src/model/raw.rs` | 73 | 15 | 79,45 % | 18 (5) |
| `src/rels.rs` | 183 | 6 | 96,72 % | 24 (2) |
| `src/context.rs` | 137 | 4 | 97,08 % | 16 (1) |
| `src/model/validate.rs` | 256 | 4 | 98,44 % | 19 (0) |
| `src/error.rs`, `src/lib.rs`, `src/model/{mod,numbering,style}.rs` | 169 | 0 | 100 % | 20 (0) |

**Три файла держат 1567 из 1767 непокрытых строк (89 %):** `document.rs`,
`numbering.rs`, `styles.rs`. Только на них и стоит целиться, чтобы добрать 269.

core (тоже из summary, для полноты):

| Файл | Строк | Непокрыто | Строк % |
| --- | ---: | ---: | ---: |
| `src/xml.rs` | 31 | 8 | 74,19 % |
| `src/archive.rs` | 277 | 9 | 96,75 % |
| `src/rels.rs` | 187 | 5 | 97,33 % |
| `src/{error,node_id,warning,zip_limits}.rs` | 452 | 0 | 100 % |

## Что именно не покрыто: docx

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

## Сколько не хватает и за счёт чего добирать

До порога docx не хватает **269 исполняемых строк**. Ориентиры для E2, по
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

## Время прогона

Для решения о таймаутах (слайс E3), локально:

| Этап | Время |
| --- | ---: |
| Сборка инструментированного core (пустой `target-dir`) + 54 теста | 19 с |
| Сборка инструментированного docx поверх + 227 тестов | 30 с |
| **Холодная сборка обоих крейтов в чистом каталоге + 282 теста** | **40 с** |
| Отчёты `report` без прогона тестов (сводка, таблица, missing lines, JSON) | 1–2 с каждый |
| Весь сценарий замера целиком (2 прогона + 7 отчётов + 2 JSON) | 54 с |

Это локальные числа M3 Pro с прогретым `~/.cargo`; на CI-раннере (2–4 vCPU)
сборка будет заметно дольше, но порядок — минуты, не десятки минут.
Инструментированное дерево занимает ≈ 350 МБ на оба крейта.

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
cargo llvm-cov -p doc-converter-docx  --fail-under-lines 85; echo "код=$?"   # 1
```

Разделение прогона и отчёта (так считался объединённый профиль двух сюитов):

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

Числа этого отчёта — из прогонов в `target/llvm-cov` (09.10.2026, 20:44–20:47
по локальному времени) и `target/llvm-cov-cold` (20:49); JSON-выгрузки и логи
лежат локально, в репозиторий не добавлялись.

## Чего в замере нет

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
  (`-p` фильтрует по файлам крейта); юнит-тесты внутри `src` входят и дают
  70 непокрытых строк из 1653.

## Сомнения и ограничения

- Число в колонке `Missed Lines` (1767 у docx) чуть больше числа строк в списке
  `--show-missing-lines` (1653): llvm-cov считает эти величины по-разному.
  Процент — от summary, адреса для правки — из списка.
- Прогон идёт по `debug`-профилю с `-C instrument-coverage` — на разметку
  строк это не влияет, на время сборки влияет.
- `--fail-under-lines` в этом замере применялся к `cargo llvm-cov report`;
  код возврата 1 у docx — это именно недобор порога, тесты при этом зелёные.
- Один прогон на крейт, без повторов: llvm-cov детерминирован на одном и том же
  наборе тестов, разброса, как у времён/памяти в других отчётах, здесь нет.
- Дерево во время замера двигали параллельные слайсы (тесты и CI-скрипты, не
  `src`); воспроизведение на текущем HEAD может дать другие числа по тестовым
  файлам — на покрытие `src` это не влияет.

## Вывод

- **`crates/core` порог 80 % проходит с большим запасом: 97,68 % строк**
  (в объединённом прогоне с docx — 98,63 %). Слайс по core не нужен.
- **`crates/docx` порог 85 % не проходит: 82,31 % строк**, не хватает 269
  исполняемых строк. Долг сконцентрирован в трёх файлах (89 % непокрытого):
  `document.rs`, `numbering.rs`, `styles.rs`; самая крупная однородная группа —
  дублированные таблицы «атрибут → enum» (≈ 338 строк), и её одной достаточно,
  чтобы взять порог.
- Ветви ошибок и предупреждений непокрыты почти по всему `document.rs`
  (≈ 111 строк), но это не главный резерв: строки короткие, и добирать их
  выгодно теми же фикстурами, что и пункт выше.
- Цена порога в docx: 269 строк ≈ 2,7 п.п. при знаменателе 9988 строк
  (1 п.п. ≈ 100 строк), то есть E2 без новых фикстур с перечисленными
  атрибутами не закрывается.
