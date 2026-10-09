# Спринт 8: differential-тест DOCX против mammoth

Отчёт пишется `npx tsx scripts/diff-mammoth.ts --update-oracle` (сухой прогон —
без флагов, сверка с репозиторием — `--check`). Оракул —
[`test-fixtures/docx/mammoth-oracle.json`](../../test-fixtures/docx/mammoth-oracle.json):
абзацы и счётчики mammoth; список абзацев нашей стороны в него не кладётся,
Rust-тест гейта считает его сам — иначе проверка сравнивала бы разбор сам с собой.

Версия mammoth: 1.13.0.

## Что сравнивается

Нормализация — контракт из `crates/docx/examples/dump_model.rs`, дословно тот же с
обеих сторон: обход в порядке документа с рекурсией в таблицы (строки, ячейки, их
блоки — в один плоский список); текст абзаца = текст прогонов как есть, `w:tab` →
`"\t"`, разрывы/символы/рисунки/неизвестное → ничего, внутри гиперссылки и
результата поля — рекурсивно; `trim()`, пустые строки выброшены; сноски, концевые
сноски, комментарии и колонтитулы в список не входят.

Кандидаты — все 97 фикстур минус `broken/`, `alternate_content/` (mammoth читает
`mc:Fallback`, мы `mc:Choice`), `track_changes/` (наши non-goal) и `notes/`
(mammoth выносит их отдельно): 76.

## Оракул

[`test-fixtures/docx/mammoth-oracle.json`](../../test-fixtures/docx/mammoth-oracle.json)
пишет тот же скрипт: заголовок (`generator`, `mammoth`, `threshold`,
`min_comparable`, `candidates`) и `fixtures` — по записи на каждую из 97 фикстур,
ключи по возрастанию:

- `comparable` — фикстура входит в набор и mammoth её прочитал;
- `skip_reason` — почему нет (вне набора или отказ mammoth), иначе `null`;
- `classification` — класс, `null` у несопоставимых;
- `reason` — почему класс такой, пустая строка у `match`;
- `mammoth.paragraphs` / `mammoth.counts` — сторона оракула: абзацы и счётчики
  (`p`, `table`, `li`, `a`) из HTML;
- `model.counts` — наши счётчики (таблицы, пункты списка, ссылки, рисунки);
  абзацев нашей стороны в оракуле нет — их считает тест-гейт.

Классы расхождений:

- `match` — списки абзацев совпали полностью;
- `expected` — расхождение принято и объяснено (политика mammoth или наше решение);
  **остаётся в знаменателе метрики**;
- `non-goal` — фикстура вне области сравнения (элемента нет в модели v1 или его не
  читает mammoth); **выводится из знаменателя**, потолок — 8 из 76;
- `bug` — дефект нашего парсера;
- `unclassified` — решение не принято — значение по умолчанию.

Гейт: `unclassified = 0`, `bug = 0`, `non-goal ≤ 8`,
`match / (кандидаты − non-goal) ≥ 0.95` и сопоставимых не меньше 50.
Классификация — таблица `CLASSIFICATION` в `scripts/diff-mammoth.ts`; всё, что
разошлось и в таблицу не попало, падает в `unclassified`, а не угадывается.

## Сводка

| Метрика | Значение |
|---|---|
| Кандидатов | 76 |
| Сопоставимо (mammoth прочитал) | 76 |
| `match` | 74 |
| `expected` | 0 |
| `non-goal` | 2 |
| `bug` | 0 |
| `unclassified` | 0 |
| Доля match / (кандидаты − non-goal) | 74/74 = 1.0000 (порог 0.95) |
| Гейт | пройден |

## Кандидаты

| Фикстура | Класс | Причина | Абзацев (мы / mammoth) |
|---|---|---|---|
| `basic/astral_unicode` | match | — | 3 / 3 |
| `basic/breaks_and_tabs` | match | — | 3 / 3 |
| `basic/empty_body` | match | — | 0 / 0 |
| `basic/indentation` | match | — | 4 / 4 |
| `basic/many_runs_paragraph` | match | — | 1 / 1 |
| `basic/multiple_runs` | match | — | 1 / 1 |
| `basic/paragraph_alignment` | match | — | 4 / 4 |
| `basic/paragraph_spacing` | match | — | 4 / 4 |
| `basic/sections` | match | — | 2 / 2 |
| `basic/whitespace_preserve` | match | — | 2 / 2 |
| `cjk/chinese_simplified` | match | — | 1 / 1 |
| `cjk/japanese` | match | — | 1 / 1 |
| `cjk/korean` | match | — | 1 / 1 |
| `cjk/mixed_latin` | match | — | 1 / 1 |
| `cjk/vertical_text` | match | — | 1 / 1 |
| `complex/text_and_table` | match | — | 6 / 6 |
| `edge_cases/empty_paragraphs` | match | — | 1 / 1 |
| `edge_cases/multilingual` | match | — | 3 / 3 |
| `edge_cases/special_chars` | match | — | 1 / 1 |
| `fields/date_field` | non-goal | mammoth не читает w:fldSimple (элемента нет в карте элементов body-reader.js, неизвестный элемент отбрасывается вместе с детьми) — расхождение на его стороне | 2 / 2 |
| `fields/fld_char_with_separate` | match | — | 2 / 2 |
| `fields/fld_simple` | non-goal | mammoth не читает w:fldSimple (элемента нет в карте элементов body-reader.js, неизвестный элемент отбрасывается вместе с детьми) — расхождение на его стороне | 2 / 2 |
| `fields/hyperlink_field` | match | — | 2 / 2 |
| `fields/instr_text` | match | — | 2 / 2 |
| `fields/nested_fields` | match | — | 1 / 1 |
| `fields/unknown_field` | match | — | 1 / 1 |
| `formatting/bold` | match | — | 1 / 1 |
| `formatting/heading_1` | match | — | 1 / 1 |
| `formatting/heading_2` | match | — | 1 / 1 |
| `formatting/heading_3` | match | — | 1 / 1 |
| `formatting/italic` | match | — | 1 / 1 |
| `headers_footers/default_header_footer` | match | — | 1 / 1 |
| `headers_footers/even_odd` | match | — | 1 / 1 |
| `headers_footers/header_with_image` | match | — | 1 / 1 |
| `headers_footers/page_number_field` | match | — | 1 / 1 |
| `headers_footers/title_pg_first` | match | — | 1 / 1 |
| `images/anchor_behind_text` | match | — | 1 / 1 |
| `images/anchor_top_and_bottom` | match | — | 0 / 0 |
| `images/anchor_wrap_square` | match | — | 0 / 0 |
| `images/anchor_wrap_tight` | match | — | 0 / 0 |
| `images/inline` | match | — | 0 / 0 |
| `images/multiple_sizes` | match | — | 0 / 0 |
| `numbering/bullet_symbols` | match | — | 4 / 4 |
| `numbering/custom_format` | match | — | 3 / 3 |
| `numbering/decimal_basic` | match | — | 3 / 3 |
| `numbering/multilevel` | match | — | 5 / 5 |
| `numbering/nested_levels` | match | — | 4 / 4 |
| `numbering/restart_numbering` | match | — | 4 / 4 |
| `numbering/start_override` | match | — | 3 / 3 |
| `rtl/arabic_basic` | match | — | 1 / 1 |
| `rtl/hebrew_basic` | match | — | 1 / 1 |
| `rtl/mixed_direction` | match | — | 1 / 1 |
| `rtl/rtl_numbered_list` | match | — | 3 / 3 |
| `rtl/rtl_table` | match | — | 4 / 4 |
| `simple/empty` | match | — | 0 / 0 |
| `simple/multiple_paragraphs_0` | match | — | 5 / 5 |
| `simple/multiple_paragraphs_1` | match | — | 5 / 5 |
| `simple/multiple_paragraphs_2` | match | — | 5 / 5 |
| `simple/one_paragraph` | match | — | 1 / 1 |
| `styles/based_on_chain` | match | — | 1 / 1 |
| `styles/character_style_run` | match | — | 1 / 1 |
| `styles/default_paragraph` | match | — | 1 / 1 |
| `styles/direct_formatting_override` | match | — | 1 / 1 |
| `styles/doc_defaults` | match | — | 1 / 1 |
| `styles/linked_character` | match | — | 1 / 1 |
| `styles/qformat_latent` | match | — | 2 / 2 |
| `styles/table_style` | match | — | 4 / 4 |
| `tables/alignment_widths` | match | — | 10 / 10 |
| `tables/borders_shading` | match | — | 4 / 4 |
| `tables/grid_span` | match | — | 6 / 6 |
| `tables/header_repeat` | match | — | 62 / 62 |
| `tables/nested` | match | — | 9 / 9 |
| `tables/simple_2x2` | match | — | 4 / 4 |
| `tables/simple_3x3` | match | — | 9 / 9 |
| `tables/tbl_look` | match | — | 4 / 4 |
| `tables/v_merge` | match | — | 4 / 4 |

Совпадение на пустых списках (0 / 0) — 7: `basic/empty_body`, `images/anchor_top_and_bottom`, `images/anchor_wrap_square`, `images/anchor_wrap_tight`, `images/inline`, `images/multiple_sizes`, `simple/empty`. Текста в абзацах нет ни с
одной стороны — это совпадение, но проверяет оно только то, что ни один из парсеров
не выдумывает текст из разметки; счётчики `model.counts.images` по ним — в оракуле.

## Вне набора

| Фикстура | Причина |
|---|---|
| `alternate_content/choice_fallback_textbox` | mammoth читает mc:Fallback, мы — mc:Choice: это разные ветки |
| `alternate_content/choice_shape` | mammoth читает mc:Fallback, мы — mc:Choice: это разные ветки |
| `alternate_content/fallback_only` | mammoth читает mc:Fallback, мы — mc:Choice: это разные ветки |
| `alternate_content/group_shape` | mammoth читает mc:Fallback, мы — mc:Choice: это разные ветки |
| `alternate_content/nested_alternate` | mammoth читает mc:Fallback, мы — mc:Choice: это разные ветки |
| `broken/cyclic_based_on` | битые пакеты: политика ошибок (ADR-0016), а не сверка текста |
| `broken/macro_enabled` | битые пакеты: политика ошибок (ADR-0016), а не сверка текста |
| `broken/missing_root_rels` | битые пакеты: политика ошибок (ADR-0016), а не сверка текста |
| `broken/missing_style_and_abstract_num` | битые пакеты: политика ошибок (ADR-0016), а не сверка текста |
| `broken/no_document_xml` | битые пакеты: политика ошибок (ADR-0016), а не сверка текста |
| `broken/truncated_zip` | битые пакеты: политика ошибок (ADR-0016), а не сверка текста |
| `notes/comments_basic` | сноски и комментарии mammoth выносит отдельно от тела документа |
| `notes/comments_multiple` | сноски и комментарии mammoth выносит отдельно от тела документа |
| `notes/footnote_basic` | сноски и комментарии mammoth выносит отдельно от тела документа |
| `notes/footnote_in_table` | сноски и комментарии mammoth выносит отдельно от тела документа |
| `notes/footnote_separator` | сноски и комментарии mammoth выносит отдельно от тела документа |
| `track_changes/deleted_run` | правки (w:ins/w:del) у нас non-goal, ревизии не применяются |
| `track_changes/inserted_run` | правки (w:ins/w:del) у нас non-goal, ревизии не применяются |
| `track_changes/move_and_format_change` | правки (w:ins/w:del) у нас non-goal, ревизии не применяются |
| `track_changes/paragraph_mark_change` | правки (w:ins/w:del) у нас non-goal, ревизии не применяются |
| `track_changes/table_row_changes` | правки (w:ins/w:del) у нас non-goal, ревизии не применяются |

## Расхождения

### `fields/date_field` — non-goal

mammoth не читает w:fldSimple (элемента нет в карте элементов body-reader.js, неизвестный элемент отбрасывается вместе с детьми) — расхождение на его стороне

Наш список:

- `Date: 01.01.2026`
- `Date (fldSimple): 01.01.2026`

mammoth:

- `Date: 01.01.2026`
- `Date (fldSimple):`

### `fields/fld_simple` — non-goal

mammoth не читает w:fldSimple (элемента нет в карте элементов body-reader.js, неизвестный элемент отбрасывается вместе с детьми) — расхождение на его стороне

Наш список:

- `Page 1 of 12`
- `Page (fldChar): 1`

mammoth:

- `Page  of`
- `Page (fldChar): 1`
