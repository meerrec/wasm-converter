# Спринт 7 — edge cases и честная карта покрытия (J5)

Ветка `sprint/7-pagination-pdf`, HEAD `3703423`, дерево чистое. Дата прогонов — 06.10.2026.
Документ собран приёмкой: числа получены повторным прогоном на этой машине, а не пересказом
отчётов. Где покрытия нет — так и написано; список дыр в конце важнее таблицы.

## 1. Прогоны (числа этой машины)

| # | Команда | Результат |
| --- | --- | --- |
| 1 | `cargo test --workspace` | **547 passed, 0 failed, 1 ignored** (35 suite'ов); ignored — `crates/xlsx/tests/budget.rs::million_cells_open_within_two_seconds` (тяжёлый, релиз) |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0; 3 warning'а только у вендоренного `printpdf` (unused import/variable), workspace чист |
| 3 | `cargo fmt --all --check` | exit 0 |
| 4 | `cargo deny check` | exit 0 |
| 5 | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 (те же 3 warning'а форка) |
| 6 | `cd vendor/printpdf && cargo test --offline --no-default-features` | **22 passed, 0 failed, 11 ignored** |
| 7 | `pnpm exec playwright test` (examples/viewer-xlsx) | **22 passed** ×4 проекта (chromium/firefox/webkit/chromium-dpr2), 16.2 с; wasm собирать не потребовалось |
| 8 | `pnpm turbo run typecheck --force` | **5 successful, 0 cached**, 3.4 с |

Дополнительно (не входило в список, но это несущие числа DoD 1/8 — сняты мной, не пересказаны):

```text
target/release/examples/mem_probe target/fixtures/scale-500-pages.xlsx \
  /usr/bin/time -l
→ вход 386 987 Б, PDF 535 870 Б, страниц 480, экспорт 155,1 мс
  maximum resident set size 29 573 120 Б (28,2 МиБ)
```

`packages/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm`: 2 632 738 Б, gzip **1 349 727 Б**
(DoD 12 записал 1 335 460 Б — артефакт подрос на ~14 КБ, см. дыры).

## 2. Возможности спринта × негативные кейсы

| Возможность | Негативный кейс | Где тест | Покрыт |
| --- | --- | --- | --- |
| Пагинация по строкам | строка выше целой страницы не дробится и в режиме разрыва | `crates/pdf/src/pagination.rs::row_taller_than_page_is_never_split` | да (unit) |
| Пагинация по столбцам | ни одна ячейка не теряется; каждая страница непуста; полосы без сдвига | `crates/pdf/tests/pagination.rs::all_cells_survive_column_split`, `page_counts_are_pinned` | да |
| Повтор заголовков | `repeat_header_rows: 0` (умолчание) не меняет числа страниц | прямо теста «0 → прежнее» нет; косвенно — `page_counts_are_pinned` + `golden_drawing_is_unchanged` на дефолте | частично |
| Повтор столбцов | повторные столбцы не ломают центрирование | `crates/pdf/tests/pagination.rs::first_columns_repeat_on_every_page`; unit `center_horizontally_counts_repeated_columns` | да |
| `avoid_row_break` | оба режима: `true` переносит строку целиком, `false` рвёт и прижимает к границе | `src/pagination.rs::avoid_row_break_moves_partial_row_whole_to_next_page`, `avoid_row_break_counts_repeated_header`; интеграционный `tests/pagination.rs::avoid_row_break_false_repeats_split_row` | да |
| `orphan_rows` | хвост короче минимума уезжает на следующую страницу; конфликт с `widow_rows` | `src/pagination.rs::orphan_rows_moves_short_block_tail_to_next_page`, `widow_shift_respects_orphan_minimum` | **только unit** (интеграционного теста нет) |
| `widow_rows` | вдова тянет строки назад, но не больше потока | `src/pagination.rs::widow_rows_pulls_rows_from_previous_page`, `widow_longer_than_flow_does_not_panic` | **только unit** |
| `fit_to_width` | `0` = не ужимать; не увеличивать; перебивает `scale` | `src/pagination.rs::zero_fit_to_width_means_no_fit`, `fit_to_width_does_not_enlarge`, `fit_to_width_overrides_manual_scale` | **только unit** |
| `fit_to_height` | ужимает ровно до N страниц и только вниз | `src/pagination.rs::fit_to_height_shrinks_to_given_page_count`; интеграционный `tests/pagination.rs::fit_to_height_limits_page_count` | да |
| Центрирование (`center_horizontally`/`center_vertically`) | полоса шире области не сдвигается влево; учитываются повторные шапка/столбцы | `src/pagination.rs::center_horizontally_centers_every_column_band`, `center_horizontally_counts_repeated_columns`, `center_vertically_centers_every_row_band`, `center_vertically_counts_repeated_header` | **только unit** (в PDF-потоке не проверено) |
| Сетка | при `print_grid_lines: false` линий нет, рамки ячеек остаются; пустой лист без линий; цвет `D9D9D9`, толщина 0.75 pt; координаты = рёбра `SheetLayout` | `crates/pdf/tests/grid.rs` (4 теста) | да |
| Диаграммы | лист без диаграмм не даёт ни клипов, ни `/Do`; пустая диаграмма не эмитит ничего; вырожденные данные | `crates/pdf/tests/chart.rs::sheet_without_charts_gets_no_chart_operators`; `src/chart.rs` (18 unit, среди них `empty_chart_emits_nothing`); `crates/render/src/chart/layout.rs` (15 unit) | да в Rust; **в e2e — нет** |
| Изображения (в т.ч. битые байты) | битая картинка пропускается, остальные рисуются; нерастяжимый якорь отсекает; нет картинок — нет XObject | `crates/pdf/tests/image.rs` (6), `src/image.rs` (14 unit) | **нет в продукте**: модуль не подключён к `painter.rs`/`lib.rs` (см. дыру D1) |
| Гиперссылки (внешние/внутренние/битые) | битая ссылка не даёт аннотации; ссылка на уехавшую на вторую страницу ячейку ведёт на неё; прямоугольник покрывает весь диапазон | `crates/pdf/tests/annot.rs::broken_link_produces_no_annotation`, `link_follows_cell_to_second_page`, `internal_link_goes_to_page_of_target`, `link_rect_covers_whole_range`, `link_annotations_match_sheet_hyperlinks` | да |
| Закладки | `bookmarks: false` → `/Outlines` нет | `crates/pdf/tests/annot.rs::bookmarks_disabled_leaves_no_outline`, `bookmarks_reach_the_exported_pdf` | да |
| Комментарии | комментарий вне области печати не получает страницы; шапка приколота к первой странице; связь комментария с уехавшей ячейкой | `crates/pdf/src/annot.rs` (3 unit на `comment_placements`), `crates/xlsx/src/comments.rs` unit-тесты парсера | **нет в PDF**: эмиссия `/Subtype /Text` (E4) не сделана, `/Text` в потоке не появляется (см. дыру D3) |
| Стриминг и память | частичная запись/ошибка приёмника не роняет экспорт; sink даёт те же страницы, что `Vec` | `crates/pdf/tests/sink.rs` (3 теста) | да для Rust; **в wasm/воркер путь не проведён** (дыра D6) |
| Сжатие | `compress: false` — потоки без фильтра; `Length1` = распакованному размеру; сжатый файл ≥1,5× меньше | `crates/pdf/tests/compression.rs` (4 теста), `budgets.rs::content_dense_compressed_pdf_fits_under_32_kib` | да |
| Batch-экспорт книги | ни один лист не теряется, страницы = сумме страниц листов, закладки на каждый лист, скрытые листы экспортируются, пустой лист не ломает книгу | `crates/pdf/tests/batch.rs` (7 тестов) | в API да; **в UI/e2e нет** (дыра D5) |
| Пустой лист | пустой лист — всё равно страница; без линий сетки; не ломает книгу | `tests/export.rs::empty_sheet_is_still_a_page`, `tests/grid.rs::empty_sheet_exports_without_grid_lines`, `tests/batch.rs::empty_sheet_does_not_break_the_book` | да |
| Лист с одной ячейкой | одна страница; несуществующий номер листа — ошибка с текстом | `tests/export.rs::short_sheet_stays_one_page`, `out_of_range_sheet_is_an_error` | да |
| Книга с одним листом | совпадает с постраничным экспортом листа; закладка на лист | `tests/batch.rs::single_sheet_book_exports_with_bookmarks`, `single_sheet_book_matches_the_sheet_path` | да |
| Объединённые ячейки + сетка | — | тестов нет ни на canvas, ни в PDF; сверка в `tests/grid.rs` идёт на листе без объединений | **нет** (дыра D7) |
| Колонтитулы/водяной знак | — | `crates/pdf/tests/overlay.rs` (25 тестов) проверяют модуль, но `PdfOptions` поля `overlay` не имеет и `overlay.rs` никем не вызывается | **нет в продукте** (дыра D2) |

## 3. Дыры и дефекты (по убыванию важности)

**D1. Изображения не доходят до PDF — это дефект против цели спринта, а не пробел тестов.**
Формулировка спринта обещает выгрузку «…с векторными диаграммами, изображениями, кликабельными
гиперссылками…». `crates/pdf/src/image.rs` написан и покрыт 20 тестами, но на него нет ни одной
ссылки за пределами самого файла (`grep -rn "crate::image\|image::place" crates/pdf/src` пуст).
Проверил эмпирически: `mem_probe test-fixtures/xlsx/images-png.xlsx --out /tmp/sprint-7-img.pdf`
даёт PDF, в котором из `/Subtype` есть только `CIDFontType` и `Type` — ни одного `/Image`, хотя
фикстура несёт `xl/drawings/drawing1.xml` и media-часть. То есть в UI-экспорте картинок нет
вовсе, а не «есть, но криво».

**D2. Колонтитулы, водяной знак и правило (модуль `overlay`) тоже не подключены.**
`pub mod overlay;` объявлен в `lib.rs:19`, но `overlay` не вызывается ни из `painter.rs`, ни из
`lib.rs`, и в `PdfOptions` нет поля для `OverlayConfig` — заказать колонтитул через экспортёр
нельзя даже теоретически. 25 тестов `tests/overlay.rs` проверяют функции в вакууме.

**D3. Комментарии остались парсером.** `crates/xlsx/src/comments.rs` разбирает legacy notes,
`crates/pdf/src/annot.rs::comment_placements` считает страницу и место, но эмиссии
`/Subtype /Text` нет (E4 в плане — ⬜). В присланных DoD спринта пункта про комментарии нет, но в
README спринта «комментарии» числятся среди вещей J1; в PDF их нет.

**D4. «500 страниц» в DoD 1 — фактически 480.** Генератор `scripts/gen-fixtures.ts:1144`
обещает «≈500», `HEAVY_FIXTURES` пишет книгу в `target/fixtures/` (в git её нет), мой прогон
`mem_probe` даёт **480 страниц**. Экспорт 155 мс — до 3 с с запасом, пик 28,2 МиБ ≤ 40 МиБ (DoD 8
сходится по абсолютной границе). Но `cargo bench` в CI не гоняется ни на одной ноге, а тяжёлой
фикстуры в CI нет: **ни DoD 1, ни DoD 8 не имеют автоматического гейта** — числа держатся на
локальных прогонах.

**D5. Batch-экспорт книги есть в API, но не в продукте и не в e2e.**
`export_xlsx_book`/`export_pdf_book` работает (`worker.ts:318`), но в примере кнопки «Экспорт всех
листов» нет (`examples/viewer-xlsx/src/main.ts` знает только `active.exportPdf()`,
`index.html` — одну кнопку). H2 («в примере — выбор листов и прогресс») и H3 (многолистовая
фикстура в трёх браузерах) не сделаны. Плюс в `tests/batch.rs` нет негативного кейса
«ошибка на одном листе не съедает остальные» из DoD H2.

**D6. Sink-путь не проведён в wasm и воркер.** `crates/pdf/src/lib.rs:86` упоминает
`StreamSession`, но `crates/pdf-wasm` отдаёт `Vec<u8>`, `worker.ts:320` шлёт `bytes` с
transferable. DoD 8 меряется на Rust-пути (`mem_probe`), продукт этим не пользуется. Открытый
вопрос 15, сужение сознательное — но тогда DoD 8 не про продукт.

**D7. Объединённые ячейки и сетка не проверены нигде.** В `crates/pdf/src/painter.rs` нет ни
одного обращения к `sheet.merges` (кроме `pagination.rs:627`, где merge учитывается для текста):
`draw_grid` рисует рёбра по раскладке без оглядки на объединения, а заливка объединённого
диапазона ложится поверх сетки. Канвас устроена так же (`xlsx::paint::draw_grid` без merge-
фильтра, затем заливки merged поверх). Итог: **объединение без заливки просвечивает сеткой и в
PDF, и на canvas** — это расхождение с Excel, а не только с PDF-веткой. Тест
`xlsx::paint::merged_range_is_painted_once_and_hides_the_grid` покрывает случай *с* заливкой.
Известная формулировка «в PDF просвечивает» подтверждается, но неполна: это свойство обоих путей.

**D8. Расхождение слоёв «сетка/заливка» между canvas и PDF.** В PDF комментарий в коде прямо
говорит: «Сетка — под всем содержимым: заливка и рамка её перекрывают, как в Excel»
(`painter.rs:359`). На canvas порядок обратный: заливки → сетка → заливки merged
(`xlsx/src/paint.rs:458-462`). Для ячейки с заливкой, перекрывающей ребро, картинки экрана и
печати разойдутся. Тестов на это расхождение нет ни в Rust, ни в e2e; внесено осознанно, но
нигде не проверено.

**D9. DoD 4 «паритет с canvas через `exportPng` в e2e» не существует.** В
`examples/viewer-xlsx/e2e` слово `chart` не встречается ни разу (`grep -ri chart e2e` пуст;
`exportPng` вызывается только в `viewer.spec.ts`/`dpr.spec.ts` на `content-table.xlsx` и
DPR-фикстурах). Диаграммы, картинки, ссылки, аннотации и batch в e2e не проверяются: паритет
экрана и печати для них держится на общих рассуждениях. Это дыра, а не «покрыто».

**D10. Внешние проверки локально холостые.** `qpdf`/`pdftotext` на этой машине не установлены
(`which` не находит), поэтому `crates/pdf/tests/external.rs` печатает «пропуск» и возвращается —
оба теста отчитываются `ok`, ничего не проверив. В CI шаг «pdf tooling» ставит утилиты
(`.github/workflows/ci.yml:50-52`), так что там они настоящие; локальный зелёный — не доказательство.
То же с `#[ignore]` у `million_cells_open_within_two_seconds`.

**D11. Размер pdf-wasm подрос против зафиксированного в DoD 12.** 1 349 727 Б gzip против
1 335 460 Б в плане (+1,1 %). Рост объясним (подключение диаграмм и сетки после замера), но
DoD 12 требует «замерен до/после», а не «после после»: числа в плане устарели.

**D12. Пре-проход глифов выбран аргументом, а не замером** (подтверждаю формулировку плана:
в отчёте есть только +7 % для выбранного варианта, 35 из 162 мс; альтернативы отброшены
доводами). Решение оркестратора — не дозамерять; это зафиксировано, но замером не является.

## 4. Где зелёный тест ничего не доказывает (адверсариальный разбор)

- **`chart_is_vector_not_image` падает не на пустоте.** Фикстура `charts-five-kinds.xlsx`
  реально содержит `xl/charts/chart1..5.xml` и drawing с пятью якорями; тест требует
  `blocks.len() == 5` (клип на каждую диаграмму) и в каждом блоке ≥2 оператора пути и ≥1
  закраску, плюс наличие кривых Безье в документе. При отключении отрисовки диаграмм клипов
  стало бы 0 и ассерт упал. Парный тест `sheet_without_charts_gets_no_chart_operators` требует
  обратного на листе без диаграмм — сканер не «всегда находит клип». Тест фальсифицируем.
  Чего он **не** доказывает: совпадения геометрии с canvas (число примитивов/габаритный ящик
  против `layout()`) — такого теста нет; проверяется факт «вектор, а не картинка».
  Оговорка: часть «нет `/Do` во всём документе» на этой фикстуре тривиальна (картинок в ней нет),
  несущая часть — положительные ассерты.
- **`grid_lines_follow_layout_edges` — сверка относительная.** Эталон считается из
  `SheetLayout::column_x/row_y` и геометрии страницы, а не записан числами. Это независимый
  путь вычисления (реализация `painter::draw_grid` и оракул в тесте — разный код), и он поймает
  сдвиг/потерю/удвоение линий. Но систематическая ошибка самой `SheetLayout` (например, неверная
  ширина столбца) пройдёт незамеченной: оракул и реализация делят один источник. Абсолютных
  координат в тестах сетки нет; `page_counts_are_pinned` и golden-хеши — тоже сняты с кода.
- **Golden-эталоны (`DRAWING`, `golden_drawing_is_unchanged`) ловят изменение, а не дефект** —
  это уже зафиксировано приёмкой волны 3–4 и остаётся в силе.
- **`page_counts_are_pinned`** не включает тяжёлую 500-страничную книгу (её нет в git) — DoD 1
  вне автоматической проверки.
- **e2e `pdf.spec.ts`** проверяет только «файл скачался, начинается с `%PDF-`, не пустой» на
  одной текстовой фикстуре — ни страниц, ни содержимого, ни валидности (валидность — на CI через
  `qpdf`, которого локально нет).

## 5. Вердикт по DoD

| DoD | Статус по тому, что я увидел |
| --- | --- |
| 1 (500 страниц < 3 с) | **не подтверждён полностью**: фикстура даёт 480 страниц, экспорт 155 мс; CI-гейта нет |
| 2 (заголовки на каждой странице) | подтверждён (integration + `pdftotext` в CI) |
| 3 (ни одна ячейка не теряется) | подтверждён |
| 4 (диаграммы векторные) | подтверждён в Rust; **паритет с canvas в e2e отсутствует** |
| 5 (гиперссылки) | подтверждён |
| 6 (все edge cases) | **не выполнен в буквальном смысле**: см. таблицу §2 — шесть строк «только unit», четыре «нет в продукте» |
| 7 (три браузера) | подтверждён локально (22 e2e × 4 проекта); `qpdf --check` — только в CI |
| 8 (пик ≤ 40 МиБ) | подтверждён на Rust-пути (28,2 МиБ), но sink в продукт не проведён |
| 9 (сжатие ≥1,5×) | подтверждён |
| 10 (сетка) | подтверждён, с оговоркой про относительный оракул |
| 11 (регрессии Спринта 6) | подтверждён |
| 12 (размер pdf-wasm) | **числа в плане устарели**: 1 349 727 Б против 1 335 460 Б |

Итог: ядро спринта (пагинация, диаграммы, ссылки, закладки, сжатие, стриминг-в-Rust, batch-API,
сетка) сделано и проверено. Но **две заявленные в цели возможности — изображения и колонтитулы —
в продукте отсутствуют**, комментарии остались парсером, batch-экспорт не доведён до UI и e2e,
а паритет печати с экраном для диаграмм и картинок не проверяется ничем. Считать DoD спринта
выполненным целиком нельзя: пункт 6 («все edge cases») и цель спринта в части изображений —
не выполнены; DoD 1 держится на 480-страничной фикстуре без CI-гейта.
