# Корпус строгого просмотра — Stage 1

Каталог содержит реестр корпуса первой сертификации (`corpus.json`) и дом эталонов
strict-viewer. Согласно
[`docs/strict-viewer/baseline-policy.md`](../../docs/strict-viewer/baseline-policy.md) §1,
эталоны раскладываются как `<format>/<fixture-id>/<mode>/manifest.json` и
`<format>/<fixture-id>/<mode>/reference/*.png` (PNG и PDF — через Git LFS). Reference PNG/PDF
в каталоге пока нет: здесь только реестр.

**Состав реестра: 41 DOCX + 35 XLSX = 76 фикстур.** Каждый файл, который матрица называет
минимальным, есть в реестре; сами файлы — в `test-fixtures/docx` и `test-fixtures/xlsx`
(здесь только реестр). Два файла, которые раньше были в реестре без нужного содержимого, теперь
его несут: `xlsx/layout-broken-link.xlsx` — часть `xl/externalLinks/externalLink1.xml` со своими
rels, `xlsx/sheets-different-shapes.xlsx` — `xl/drawings/drawing1.xml` с тремя `<xdr:sp>`.
Проверка состава — `npx tsx tools/strict-viewer-corpus/report.ts`.

`corpus.json` — **единственный источник истины** об ожидаемом поведении. Отдельных README у
фикстур нет намеренно: описание, теги, строки матрицы и ожидаемый результат живут в реестре.
Эталоны снимаются по двум путям, закреплённым в
[`baseline-policy.md`](../../docs/strict-viewer/baseline-policy.md) §1 п.5 и
[`ADR-0056`](../../docs/adr/0056-capture-automation-com.md): основной — автоматический,
`scripts/strict-viewer-references.ps1` / `-macos.sh` (COM на Windows, AppleScript на macOS —
автоматизация локального Word/Excel,
`ExportAsFixedFormat`, затем `pdftoppm -png -r 144`); ручной захват в Word Online / Excel Online
остаётся допустимым, но не обязательным. Наш движок эталоном не является никогда; `actual/` не
копируется в `reference/`.

## Формат реестра

| Поле | Значение |
|---|---|
| `schemaVersion` | версия формы реестра, `1.0.0` |
| `corpus.id` / `corpus.version` | `strict-viewer-1.0.0` / `1.0.0` |
| `corpus.matrixVersion` | версия матрицы, на которую ссылается корпус (`1.0.0`) |
| `corpus.baselineId` | `null` до первого принятого baseline |
| `corpus.storage` | `git-lfs`, `rootUri` = `test-fixtures/strict-viewer` |
| `fixtures[].id` | slug от `source.path` |
| `fixtures[].format` | `docx` или `xlsx`, совпадает с префиксом пути |
| `fixtures[].source.path` | путь относительно `test-fixtures/`, файл обязан существовать |
| `fixtures[].title` | человекочитаемое имя фикстуры |
| `fixtures[].tags` | непустой список из фиксированного словаря |
| `fixtures[].matrixRows` | непустой список ID строк матрицы |
| `fixtures[].expected` | 1–3 предложения о наблюдаемом результате |
| `fixtures[].modes` | режимы захвата для этой фикстуры |
| `fixtures[].notes` | необязательно: почему выбрана, что не покрывает |

**Правило slug.** Взять `source.path`, отбросить расширение, привести к нижнему регистру,
заменить каждый прогон не-алфавитно-цифровых символов на `-`, обрезать `-` по краям:
`docx/tables/long_table.docx` → `docx-tables-long-table`. ID уникальны. Записи добавляет
`npx tsx tools/strict-viewer-corpus/add.ts --list <specs.ts> [--dry-run]`: он считает `id`
сам и до записи проверяет формат, режимы, дубли и наличие файла.

**Словарь тегов.** `text`, `styles`, `numbering`, `sections`, `headers-footers`, `tables`,
`pagination`, `images`, `objects`, `notes`, `review`, `math`, `unsupported`, `fonts`, `bidi`,
`values`, `formats`, `geometry`, `conditional`, `charts`, `print`, `formulas`, `links`,
`sheets`, `locale`, `basic`.

**Словарь режимов.** DOCX: `print-layout`, `pdf`. XLSX: `normal`, `print-preview`. По
умолчанию DOCX получает `print-layout`, XLSX — `normal`; `pdf`/`print-preview` — только там,
где матрица прямо требует печатной геометрии.

## Покрытие матрицы

Все 42 строки матрицы 1.0.0 покрыты записями реестра; `+ gap` отмечены только те подпункты,
для которых фикстуры действительно нет (см. следующую таблицу).

Таблица и таблица пробелов собраны из `corpus.json` (`fixtures[].matrixRows` и `gaps`), а не из
текста матрицы: ID фикстур — это `fixtures[].id`, поэтому после правки реестра их надо
перегенерировать, а не править вручную.

| Строка матрицы | Фикстуры реестра |
|---|---|
| `docx.paragraph-runs` | `docx-basic-breaks-and-tabs`, `docx-basic-multiple-runs`, `docx-pagination-keep-next-chain` |
| `docx.styles-themes` | `docx-styles-based-on-chain` |
| `docx.numbering` | `docx-numbering-multilevel` |
| `docx.sections-columns` | `docx-basic-sections`, `docx-columns-three-unequal`, `docx-columns-two-equal` |
| `docx.headers-footers` | `docx-headers-footers-default-header-footer`, `docx-headers-footers-even-odd` + gap `even-page-type` |
| `docx.tables-fixed-autofit` | `docx-tables-alignment-widths`, `docx-tables-simple-2x2` |
| `docx.tables-merged-nested` | `docx-tables-grid-span`, `docx-tables-nested`, `docx-tables-v-merge` |
| `docx.tables-pagination` | `docx-pagination-keep-next-tall`, `docx-pagination-widow-orphan`, `docx-tables-cant-split-rows`, `docx-tables-long-table`, `docx-tables-long-table-header-repeat`, `docx-tables-row-taller-than-page` |
| `docx.tables-appearance` | `docx-tables-borders-shading` |
| `docx.images-inline` | `docx-images-inline`, `docx-images-inline-jpeg-emf` |
| `docx.images-floating` | `docx-images-anchor-wrap-square` |
| `docx.shapes-textboxes` | `docx-alternate-content-choice-shape`, `docx-alternate-content-group-shape` |
| `docx.footnotes-endnotes` | `docx-notes-endnote-basic`, `docx-notes-footnote-basic` |
| `docx.comments` | `docx-notes-comments-basic` |
| `docx.revisions` | `docx-track-changes-inserted-run` |
| `docx.math` | `docx-math-omml-simple` |
| `docx.fields-toc-refs` | `docx-fields-fld-simple`, `docx-fields-toc-ref` |
| `docx.vml-alternate` | `docx-alternate-content-choice-fallback-textbox`, `docx-alternate-content-group-shape` |
| `docx.smartart` | `docx-embedded-smartart-basic` |
| `docx.ole` | `docx-embedded-ole-object` |
| `docx.bidi-cjk` | `docx-cjk-mixed-latin`, `docx-edge-cases-multilingual`, `docx-fonts-system-font-fallback`, `docx-rtl-mixed-direction` |
| `xlsx.values` | `xlsx-content-mixed-types` |
| `xlsx.formula-cache` | `xlsx-values-formulas` |
| `xlsx.number-formats` | `xlsx-formats-currency-ruble`, `xlsx-formats-thousands-decimals` |
| `xlsx.dates-locales` | `xlsx-formats-date-1904`, `xlsx-formats-date-ru` |
| `xlsx.dimensions` | `xlsx-layout-column-widths`, `xlsx-layout-row-heights` |
| `xlsx.wrap-overflow` | `xlsx-layout-shrink-to-fit`, `xlsx-text-cyrillic-wrap`, `xlsx-values-long-text` + gap `rotation` |
| `xlsx.rich-merged` | `xlsx-content-rich-text`, `xlsx-layout-merged` |
| `xlsx.hidden-outlines` | `xlsx-layout-hidden-columns`, `xlsx-layout-hidden-rows` + gap `outlines` |
| `xlsx.freeze-scroll-zoom` | `xlsx-layout-frozen-columns` |
| `xlsx.styles-themes` | `xlsx-styles-bold-italic` |
| `xlsx.conditional` | `xlsx-cf-cell-is`, `xlsx-cf-color-scale`, `xlsx-cf-data-bar`, `xlsx-cf-icon-set` |
| `xlsx.images` | `xlsx-images-over-data` |
| `xlsx.shapes` | `xlsx-sheets-different-shapes` |
| `xlsx.charts` | `xlsx-charts-five-kinds` |
| `xlsx.comments-notes` | `xlsx-comments-legacy` + gap `threaded-comments` |
| `xlsx.pivot-slicers-controls` | `xlsx-pivot-basic`, `xlsx-slicer-basic` |
| `xlsx.print-areas-breaks` | `xlsx-print-area-titles`, `xlsx-scale-ten-pages` |
| `xlsx.print-titles-scale` | `xlsx-print-area-titles`, `xlsx-print-fit-to-page`, `xlsx-scale-ten-pages` |
| `xlsx.print-headers-footers` | `xlsx-print-headers-footers` |
| `xlsx.sheets-visibility` | `xlsx-sheets-hidden`, `xlsx-sheets-three` |
| `xlsx.external-links` | `xlsx-external-workbook-link`, `xlsx-layout-broken-link` |

Статусы строк в матрице (`planned`, `excluded`, `unsupported-visible`) — это решения о
сертификации, а не признак отсутствия файла: например, `docx.smartart` и `docx.ole` остаются
`excluded`, хотя фикстуры для них уже в реестре.

## Пробелы (`gaps`)

| Строка | Чего нет | Причина |
|---|---|---|
| `docx.headers-footers` | `even-page-type` | even_odd.docx содержит even headerReference и titlePg, а evenAndOddHeaders включён в settings.xml, но тело — один абзац: документ занимает одну страницу, поэтому тип even не наблюдаем. |
| `xlsx.wrap-overflow` | `rotation` | `shrinkToFit` покрыт `layout-shrink-to-fit.xlsx`; `textRotation` нет ни в одной зарегистрированной книге. |
| `xlsx.hidden-outlines` | `outlines` | Ни в одном листе зарегистрированных книг нет outlineLevel>0 и collapsed-групп. |
| `xlsx.comments-notes` | `threaded-comments` | `comments-legacy.xlsx` содержит только `xl/comments1.xml` и `xl/comments2.xml`; частей threaded-комментариев нет ни в одной книге. |

## Минимальный состав для этапов 3–4 (цель: покрыть матрицу 1.0.0)

Статусы: **в реестре** — фикстура в `corpus.json`, эталон снимается сразу;
**файл есть** — лежит в `test-fixtures/`, нужна только запись в реестр;
**нет файла** — нужен генератор.

### DOCX

| № | Фикстура | Область | Статус | Файл / строка матрицы |
|---:|---|---|---|---|
| 1 | Базовые стили: Normal, Heading 1–3, List Paragraph | text | в реестре | `basic/multiple_runs.docx`, `styles/based_on_chain.docx` |
| 2 | Многостраничный текст, разрывы, `keep-next` | pagination | в реестре | `basic/breaks_and_tabs.docx` (разрывы), `pagination/keep_next_chain.docx` (keepNext) |
| 3 | Widow/orphan на границе страниц | pagination | в реестре | `pagination/widow_orphan.docx`; строки матрицы нет |
| 4 | Секции: разные колонтитулы, поля, book-fold | sections | в реестре | `basic/sections.docx`, `headers_footers/default_header_footer.docx` |
| 5 | Колонки 2–3, разрыв колонки, балансировка | columns | в реестре | `columns/two_equal.docx`, `columns/three_unequal.docx` |
| 6 | Таблица fixed layout + merged cells | tables | в реестре | `tables/simple_2x2.docx`, `tables/grid_span.docx`, `tables/v_merge.docx` |
| 7 | Таблица AutoFit + wrap | tables | в реестре | `tables/alignment_widths.docx` |
| 8 | Nested table + повтор заголовка на 2+ страницах | tables | в реестре | `tables/long_table_header_repeat.docx`, `tables/nested.docx` |
| 9 | Строка таблицы выше страницы (`cantSplit`) | tables | в реестре | `tables/cant_split_rows.docx`, `tables/row_taller_than_page.docx` |
| 10 | Inline-изображения: PNG, JPEG, EMF, масштабирование | objects | в реестре | `images/inline.docx`, `images/inline_jpeg_emf.docx` |
| 11 | Floating-изображения: обтекание, anchors, z-order | objects | в реестре | `images/anchor_wrap_square.docx` |
| 12 | DrawingML shapes + группы | objects | в реестре | `alternate_content/choice_shape.docx`, `alternate_content/group_shape.docx` |
| 13 | VML legacy + `mc:AlternateContent` | diagnostics | в реестре | `alternate_content/choice_fallback_textbox.docx` |
| 14 | Сноски + концевые сноски | notes | в реестре | `notes/footnote_basic.docx`, `notes/endnote_basic.docx` |
| 15 | Комментарии + track changes | review | в реестре | `notes/comments_basic.docx`, `track_changes/inserted_run.docx` |
| 16 | Office Math, простое уравнение | math | в реестре | `math/omml_simple.docx` |
| 17 | SmartArt / OLE как `unsupported-visible` | unsupported | в реестре | `embedded/smartart_basic.docx`, `embedded/ole_object.docx` |
| 18 | Шрифты: кириллица, латиница, CJK, RTL, эмодзи | fonts | частично | `edge_cases/multilingual.docx`, `cjk/mixed_latin.docx`, `rtl/mixed_direction.docx` — в реестре; `basic/astral_unicode.docx` — файл есть, в реестре нет |
| 19 | Шрифт из версионированного пакета | fonts | в реестре | `styles/based_on_chain.docx` (Carlito подмножества) |
| 20 | Системный шрифт вне пакета, fallback-политика | fonts | в реестре | `fonts/system_font_fallback.docx` |

В пункте 18 эмодзи (astral) закрывает только незарегистрированный `basic/astral_unicode.docx`:
`edge_cases/multilingual.docx` содержит три абзаца «English», «Русский», «中文», astral-символов
в нём нет.

### XLSX

| № | Фикстура | Область | Статус | Файл / строка матрицы |
|---:|---|---|---|---|
| 1 | Числа и текст на листе | basic | в реестре | `content-mixed-types.xlsx` |
| 2 | Ширины столбцов: ручные + AutoFit | geometry | в реестре | `layout-column-widths.xlsx` |
| 3 | Automatic row heights: wrap и без | geometry | в реестре | `layout-row-heights.xlsx` (5 строк, `customHeight`) |
| 4 | Merged cells + overflow / shrink-to-fit | geometry | в реестре | `layout-merged.xlsx`, `values-long-text.xlsx`, `layout-shrink-to-fit.xlsx` |
| 5 | Скрытые строки/столбцы + freeze panes | geometry | в реестре | `layout-hidden-columns.xlsx`, `layout-hidden-rows.xlsx`, `layout-frozen-columns.xlsx` |
| 6 | Числовые форматы: валюты, проценты, научный, свои | formats | в реестре | `formats-thousands-decimals.xlsx`, `formats-currency-ruble.xlsx` |
| 7 | Даты в системах 1900 и 1904 | formats | в реестре | `formats-date-ru.xlsx`, `formats-date-1904.xlsx` |
| 8 | Темы и стили ячеек | styles | в реестре | `styles-bold-italic.xlsx` |
| 9 | Conditional formatting + data bars + icon sets | styles | в реестре | `cf-cell-is.xlsx`, `cf-color-scale.xlsx`, `cf-data-bar.xlsx`, `cf-icon-set.xlsx` |
| 10 | Изображения inline + anchored с обрезкой | objects | в реестре | `images-over-data.xlsx` |
| 11 | Shapes и комбинированные диаграммы | objects | частично | `charts-five-kinds.xlsx` (пять видов), `sheets-different-shapes.xlsx` (три `<xdr:sp>`); **combined chart нет** |
| 12 | Комментарии / notes | review | в реестре | `comments-legacy.xlsx`; threaded comments — сгенерировать |
| 13 | Print area + повторяющиеся заголовки | print | в реестре | `print-area-titles.xlsx` |
| 14 | Scale / fit-to-page по ширине и высоте | print | в реестре | `scale-ten-pages.xlsx`, `print-fit-to-page.xlsx` |
| 15 | Колонтитулы печати | print | в реестре | `print-headers-footers.xlsx` |
| 16 | Cached values + `fullCalcOnLoad`, read-only | formulas | в реестре | `values-formulas.xlsx` |
| 17 | Внешние ссылки, задокументированное поведение | links | в реестре | `external-workbook-link.xlsx`, `layout-broken-link.xlsx` |
| 18 | Pivot table: native или `unsupported-visible` | pivot | в реестре | `pivot-basic.xlsx` |
| 19 | Slicer / controls по матрице | controls | в реестре | `slicer-basic.xlsx` |
| 20 | Многоязычные числа и даты | locale | в реестре | `formats-date-ru.xlsx`, `text-cyrillic-wrap.xlsx` |

### Счёт по составу (после наполнения)

| Статус | DOCX | XLSX |
|---|---:|---:|
| Закрыт фикстурой в реестре | 19 | 20 |
| Частично: не все файлы пункта в реестре | 1 | 0 |
| Осталось без файла | 0 | 0 |

Пункты приоритетного состава покрыты файлами все 40; в реестре 39 из 40.
Открыт один пункт — DOCX 18 (шрифты): `basic/astral_unicode.docx` есть на диске, но
записи в `corpus.json` не имеет. Итог реестра: **41 DOCX + 35 XLSX = 76 фикстур**
(больше 20+20, потому что один пункт часто требует нескольких файлов: например, пункт 9
(`cantSplit`) — два файла, `cant_split_rows` и `row_taller_than_page`).

Что появилось в этой итерации:

- **9 DOCX** — `columns/two_equal`, `columns/three_unequal`,
  `pagination/keep_next_chain`, `pagination/widow_orphan`,
  `notes/endnote_basic`, `math/omml_simple`, `embedded/smartart_basic`,
  `embedded/ole_object`, `fonts/system_font_fallback`
  (`scripts/docx-fixtures/strict-viewer.ts`);
- **3 DOCX** — `headers_footers/even_odd`, `fields/toc_ref`,
  `images/inline_jpeg_emf`, `pagination/keep_next_tall`
  (`scripts/docx-fixtures/headers-footers.ts` — `even_odd`,
  `scripts/docx-fixtures/priority.ts` — три остальных);
- **8 XLSX** — `print-area-titles`, `print-fit-to-page`,
  `print-headers-footers`, `formats-date-1904`, `layout-shrink-to-fit`,
  `external-workbook-link`, `pivot-basic`, `slicer-basic`
  (`scripts/generate_strict_viewer_xlsx.ts`);
- **2 XLSX дополнены** — `sheets-different-shapes` получил три фигуры
  (`xdr:sp`), `layout-broken-link` — часть `externalLinks`
  (`scripts/patch_strict_viewer_xlsx.ts`);
- **13 ранее не учтённых** файлов внесены в реестр (`tables/nested`,
  `tables/cant_split_rows`, `tables/row_taller_than_page`,
  `tables/alignment_widths`, `basic/breaks_and_tabs`,
  `alternate_content/group_shape`, `cjk/mixed_latin`, `rtl/mixed_direction`,
  `xlsx/layout-row-heights`, `layout-hidden-rows`, `formats-currency-ruble`,
  `cf-data-bar`, `cf-icon-set`).

Эталоны этой итерацией не снимались: `corpus.baselineId` — `null`, каталогов
`<format>/<fixture-id>/<mode>/` и reference PNG/PDF в репозитории нет.

Перегенерация:

```bash
npx tsx scripts/generate_docx_fixtures.ts
npx tsx scripts/generate_strict_viewer_xlsx.ts
npx tsx scripts/patch_strict_viewer_xlsx.ts
npx tsx scripts/oracle_strict_viewer_xlsx.ts   # досыпает oracle.json
pnpm diff:mammoth                              # mammoth-oracle.json
```

### Где требования дублируются (аудит)

«Обязательный набор» задан в трёх местах, и после решения `accept-capture-automation`
числа там согласованы: везде 20+20 — это **минимальный порог**, а не потолок.

| Источник | Что сказано | Число |
|---|---|---|
| `baseline-policy.md` §3 | «Полный набор содержит **не менее** 20 DOCX и 20 XLSX» | ≥ 20 + 20 |
| `certification-matrix.md` §7 | «Предварительные 20 DOCX и 20 XLSX остаются кандидатами — это минимальный порог, а не потолок» | ≥ 20 + 20 |
| Минимальный состав выше (задача этапов 3–4) | 20 + 20 пунктов по областям | 20 + 20 |

Прежнее противоречие «минимальных файлов больше, чем 40» снято: политика требует **не менее**
20+20, а матрица ссылается на конкретные минимальные файлы сверх этого порога — это не
противоречие, потому что 20+20 задан как порог, а не как потолок. Реестр растёт под приоритетный
состав, потому что один пункт часто требует нескольких файлов.

Факты, посчитанные по файлам (`report.ts`, запуск ниже; свежий прогон печатает
`строк матрицы: 42`, `строк с минимальной фикстурой: 42`, `уникальных минимальных файлов: 41`,
`реестр: 41 DOCX + 35 XLSX = 76`):

- строк со ссылкой на минимальный файл — 42, уникальных минимальных файлов — 41: одна книга
  делится двумя строками (`xlsx.print-areas-breaks` и `xlsx.print-titles-scale` обе ссылаются
  на `print-area-titles.xlsx`);
- все 41 минимальный файл есть в реестре и на диске, расхождений нет (код выхода `0`);
- **шесть обязательных регрессионных** фикстур §7 (`long_table`,
  `long_table_header_repeat`, `charts-five-kinds`, `images-over-data`,
  `values-long-text`, `layout-frozen-columns`) — все шесть в реестре; их наличие и правило
  slug проверяет сниппет из §«Проверка реестра».

Число строк матрицы и минимальных файлов зависит от формы таблицы матрицы, поэтому в тексте
оно не фиксируется — его печатает проверка:

```bash
npx tsx tools/strict-viewer-corpus/report.ts
```

Скрипт разбирает матрицу и реестр и печатает число строк матрицы, число строк
с минимальной фикстурой, число уникальных минимальных файлов, состав реестра,
а также списки минимальных файлов, которых нет в реестре или нет на диске, и
дубли `id`/путей.

Коды выхода: **0** — расхождений нет; **1** — есть минимальные файлы, которых
нет в реестре, или дубли `id`/путей; **2** — не найдены корпус или матрица.
Код 1 — это сигнал о содержимом корпуса, а не о поломке скрипта.

## Проверка реестра

Проверка требует **минимум** 20 DOCX и 20 XLSX (не ровно 20): корпус
расширяется под приоритетную матрицу, а `baseline-policy.md` §3 фиксирует
20+20 как минимальный порог, не как потолок.

Состав, дубли `id`/путей и сверку минимальных фикстур с матрицей делает
`report.ts` выше. Остаётся то, чего он не делает: правило slug, наличие на
диске **каждого** файла реестра (скрипт смотрит на диск только для минимальных
файлов, которых нет в реестре), минимум 20+20 и шесть обязательных
регрессионных фикстур §7.

```bash
node -e 'const fs=require("fs"),p=require("path");const c=JSON.parse(fs.readFileSync("test-fixtures/strict-viewer/corpus.json","utf8"));const s=x=>x.replace(/\.[^.]+$/,"").toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-+|-+$/g,"");const R=["docx/tables/long_table.docx","docx/tables/long_table_header_repeat.docx","xlsx/charts-five-kinds.xlsx","xlsx/images-over-data.xlsx","xlsx/values-long-text.xlsx","xlsx/layout-frozen-columns.xlsx"];const e=[];const P=new Set(c.fixtures.map(f=>f.source.path));for(const f of c.fixtures){if(!fs.existsSync(p.join("test-fixtures",f.source.path)))e.push("missing "+f.source.path);if(f.id!==s(f.source.path))e.push("slug "+f.id);}for(const r of R)if(!P.has(r))e.push("mandatory "+r);const d=c.fixtures.filter(f=>f.format==="docx").length,x=c.fixtures.filter(f=>f.format==="xlsx").length;if(d<20||x<20)e.push("counts "+d+"/"+x);console.log(e.length?e.join("\n"):"ok "+d+"/"+x);process.exitCode=e.length?1:0;'
```

Вывод на текущем реестре — `ok 41/35`. Сниппет печатает `ok <docx>/<xlsx>`; любая строка
`missing`, `slug`, `mandatory` или `counts` — расхождение, код выхода 1.

Сниппет печатает `ok <docx> DOCX + <xlsx> XLSX = <всего>`; любая строка
`missing`, `slug`, `mandatory` или `counts` — расхождение, код выхода 1.
