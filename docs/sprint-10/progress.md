# Спринт 10: просмотрщик DOCX, PDF и текстовые экспорты

Отчёт о ходе работы. Ветка `sprint/10-docx-render`, 34 коммита от точки
ветвления `ff1bdf8` до `3ee1383`, 11.10.2026. Числа получены прогонами в этом
заходе, если не сказано иное.

## Что сделано

**Display list страницы** (`crates/docx/src/render.rs`, 607 строк).
`render::build(layout, page, viewport, out)` превращает страницу раскладки в
кадр общего `DisplayList` — тот же бинарный формат, что собирает XLSX-путь,
поэтому painter и SAB-ring не менялись. Вьюер рисует одну страницу за кадр, а
`viewport` задаёт зум и окно прокрутки. Начертания доехали до кадра: run style
теперь доходит до разложенного абзаца (коммит `b95ea54`), а `Segment` в
`layout/paragraph.rs` несёт флаги полужирного, курсива и подчёркивания —
раньше они терялись на шве раскладки (долг спринта 9, адресован сюда).

**Мост в wasm** (`crates/docx-wasm/`). Просмотрщик DOCX вынесен в отдельный
крейт `doc-converter-docx-wasm` (был `crates/wasm`): `docx_open`/`docx_open_bytes`,
`docx_is_open`, `docx_close`, `docx_build_display_list` и
`docx_build_display_list_sab` — последний собирает кадр в кадровый буфер и
кладёт его в слот SAB-ring (`SabRing::try_write`), как и XLSX-путь; painter и
SAB-аллокатор остаются в основном модуле, сюда приходит готовый
`SharedArrayBuffer`. Открытый документ живёт в `thread_local!`, рядом с
кадровым буфером.

**Протокол воркера и вьюер** (`packages/core`). В протокол добавлены
`DocFormat = 'xlsx' | 'docx'`, `DocxModuleUrls` (URL ленивого модуля),
сообщения `export-docx-pdf`, `export-docx-text`, `docx-opened`, `docx-text`;
`worker/frame_args.ts` маршрутизирует кадр по формату открытого документа.
Вьюер `createDocxViewer` (`packages/core/src/render/docx_viewer.ts`, 452
строки) отдаёт `open` → `PageInfo[]`, `showPage`, `setZoom`, `scrollTo`,
`exportPng`, `exportPdf`, `exportMarkdown`, `exportHtml`, `exportText`,
`onTick` и `destroy`; воркер импортирует wasm-модуль DOCX при открытии первого
`.docx` и только тогда. Отдельный слайс починил посадку на страницу: при
округлении scroll offset браузером вьюер выбирал соседнюю (`0f1fa2e`).

**Пример** `examples/viewer-docx/` — второй пример в репозитории (Vite +
Playwright, COOP/COEP через `coi-serviceworker.js`): навигация по страницам,
зум, четыре кнопки экспорта (PDF, Markdown, HTML, текст) и прокрутка.
Сквозные тесты — семь файлов `e2e/*.spec.ts` (11 тестов): корпус фикстур,
страницы, зум, перерисовка после `resize`, ленивая загрузка модуля, скачивание
экспортов.

**PDF-экспорт DOCX** (`crates/pdf/src/docx/{mod,tables,floats}.rs` +
`PdfExporter::export_docx`, `crates/pdf/src/lib.rs:75`). Страницы PDF — это
готовые страницы раскладки DOCX: сколько их вернул движок, столько уходит в
PDF, и того же размера. Шрифт один на canvas и PDF (Carlito из
`doc-converter-render`; метрики — общий `FontRegistry`, ADR-0005) — залог
совпадения точек переноса. Таблицы рисуются ячейками, плавающие элементы —
прямоугольниками-плейсхолдерами.
В wasm это `export_docx_pdf` в `crates/pdf-wasm/src/lib.rs` — документ
приезжает байтами и разбирается на месте, общего состояния с просмотрщиком у
модуля нет.

**Экспорт Markdown / HTML / plain text** (`crates/docx/src/export/`).
`to_markdown`, `to_html` и `to_text` — чистые функции `&Document → String`
(ADR-0020 §6: экспортёр, а не writer в DOCX); общий для трёх форматов разбор
абзаца с начертаниями после каскада, уровень заголовка и маркировка списков
живут в `export/mod.rs`, чтобы форматные модули их не дублировали. HTML —
самостоятельный документ со `<style>` (ширина и поля из последней секции,
шрифт и цвет из `docDefaults`); текст и адреса ссылок экранируются. В JS
выставлены `docx_export_markdown`/`docx_export_html`/`docx_export_text`
(`crates/docx-wasm/src/export_api.rs`), PDF — вызовом `export_docx_pdf` в
ленивом pdf-модуле; в примере это те самые четыре кнопки.

**Разрез wasm-модуля.** DOCX-просмотрщик переехал из общего модуля в ленивый
`crates/docx-wasm` → `packages/wasm-docx/pkg`: грузится воркером при открытии
`.docx` и на старт XLSX-просмотрщика не влияет. Бюджет у ленивого модуля свой
(`.size-limit.mjs`) — DOCX второй просмотрщик, и строка стережёт и рост
модуля, и возврат его кода в основной бандл.

**CI.** Джобы починены и добавлены (`.github/workflows/ci.yml`, 8 джоб).
Chromedriver в `wasm-test` ставится под версию Chrome раннера (без
`google-chrome` и без сборки в бакете CfT шаг падает с `::error`), потому что
ChromeDriver предпочитает предустановленный браузер образа, а `stable`-драйвер
от wasm-pack с ним разъезжался. Playwright-браузеры ставятся один раз в джобе
`js` с тремя попытками на гонку за apt-лок; wasm-тесты гоняются на
release-профиле — и `crates/wasm`, и новые `crates/docx-wasm` и
`crates/pdf-wasm`; третий модуль приезжает артефактом `wasm-docx-pkg`;
появились джоба `perf-docx` и два гейта в `wasm`: «check wasm exports»
сверяет `.d.ts` с `#[wasm_bindgen]`-экспортами, «check viewer module has no
docx exports» валит CI, если DOCX-экспорты вернулись в основной модуль.
Сквозные тесты примеров идут шагом `pnpm turbo run typecheck test build` в
джобе `js` — chromium/firefox/webkit.

**Бюджеты.** PDF 200 страниц закреплён в
`crates/pdf/tests/docx_budgets.rs`. Гейт `painter_perf` стал мягким: 300 мс по
медиане пяти проходов после двух прогревающих — headless Chrome на раннере
рисует программно, там меряется среда, а не сборка; бюджет §9 `< 16 мс`
остаётся целью для живой машины, тест ловит регрессию на порядок.

## Метрики

Прогоны этого захода; всё зелёное.

| Что | Значение | Чем получено |
|---|---|---|
| Коммиты спринта | 34 от `ff1bdf8` | `git log --oneline ff1bdf8..HEAD \| wc -l` |
| Диф спринта | 77 файлов, +9552/−124, из них 50 новых | `git diff --stat main..HEAD` |
| Фикстуры DOCX | 97 `.docx` (91 обычная + 6 в `broken/`), у каждой сайдкар | `find test-fixtures/docx -name '*.docx' \| wc -l` |
| Тесты `crates/docx` | 424: 353 библиотечных + 71 в 10 сюитах + 1 doc-тест (`ignore`) | `cargo test -p doc-converter-docx` |
| Тесты `crates/pdf` | 196: 82 библиотечных + 114 в 16 сюитах | `cargo test -p doc-converter-pdf` |
| Wasm-bindgen-тесты (новые крейты) | `crates/docx-wasm` 13, `crates/pdf-wasm` 5 — в CI джобой `wasm-test` на release | `grep -c wasm_bindgen_test crates/{docx,pdf}-wasm/tests/*.rs` |
| PDF 200+ страниц | 211 страниц: 438–469 мс (5 прогонов) в debug, 23,6–24,0 мс в release; бюджет 2000 мс в коде | `cargo test -p doc-converter-pdf --test docx_budgets [--release] -- --nocapture` |
| Тесты `@doc-converter/core` | 63 в 11 файлах; новые — docx 7, docx_export 15, docx_lazy 5, frame_args 8 | `pnpm -F @doc-converter/core test` |
| Сквозные примеры DOCX | 31 passed, 2 skipped: chromium 11, firefox 10+1, webkit 10+1; пропуск — корпус вне chromium (`test.skip(browserName !== 'chromium')`) | `pnpm -F @doc-converter/example-viewer-docx test` |
| Сквозные примеры XLSX | 40 passed: chromium/firefox/webkit по 13 + chromium-dpr2 1 | `pnpm -F @doc-converter/example-viewer-xlsx test` |
| Корпус DOCX | открыто 94 из 94 (87 с чернилами, 7 пустых — рисунки не рисуются), отвергнуто 3; обход 8 с; доля листа в кадре 0,62 | вывод `examples/viewer-docx/e2e/corpus.spec.ts` |
| Бюджеты `size-limit` | worker 5,82 из 550 КБ; index 3,01 из 200 КБ; viewer 285,38 из 300 КБ; docx 354,58 из 380 КБ | `pnpm size-limit` |
| Разрез модуля просмотрщика | 440,17 → 285,38 КБ gzip; ленивый DOCX-модуль 354,58 КБ | `pnpm size-limit`, до разреза — `ROADMAP.md:1075` |

## Чего нет

**Рисунки DOCX не рисуются нигде.** Встроенные (`wp:inline`) не порождают ни
одного элемента раскладки — `Inline::Drawing` без анкора пропускается
(`crates/docx/src/layout/paragraph.rs:140,305-311`), а тест
`a_float_becomes_a_placeholder_rect` (`crates/docx/src/render.rs:561-585`)
ассертом требует **ноль** команд `DrawCommand::Image`. Плавающие рисуются
прямоугольником-плейсхолдером: серый контур 1 px, заливки нет — ни в canvas
(`render.rs:44,47,194-208`), ни в PDF (`crates/pdf/src/docx/floats.rs:26,30,64-96`).
Корень — байты медиачасти недоступны через публичное API `docx`: `Document` не
держит `Archive` (`crates/docx/src/model/mod.rs:66-92`), а `parse_docx` роняет
архив в конце функции (`crates/docx/src/parse.rs:733-738`). Решение
пользователя: отдельный слайс после спринта. Побочно: HTML и Markdown
подставляют путь части как URL и получают висячую ссылку —
`<img src="word/media/inline.png">` (`crates/docx/src/export/html.rs:416-422`,
`markdown.rs:207`), и тесты закрепляют это как ожидаемое. Корпусный тест держит
состояние поимённым списком `DRAWINGS_NOT_PAINTED`
(`examples/viewer-docx/e2e/corpus.spec.ts:59-66`) и валится в обе стороны — если
рисунки начнут рисоваться, тест укажет на список, а не проглотит изменение.

**Подчёркивание не рисуется в PDF** (`crates/pdf/src/docx/mod.rs:19-21`):
printpdf линию сам не ставит, отдельного среза нет — у ячеек XLSX та же дыра
(`crates/pdf/src/styles.rs:84-85`). Canvas-путь DOCX подчёркивание рисует
(`crates/docx/src/render.rs:103,126`; painter — `painter_2d.rs:291`), так что
дыра только в PDF-ветке.

**Колонтитулы в раскладку не попадают вовсе.** Содержимое разобрано
(`Document.headers`/`footers`, `crates/docx/src/model/section.rs:170`) и лежит
мёртвым грузом: потребителей нет ни в `layout`, ни в `render`, ни в
`crates/pdf/src/docx/`; `LayoutOptions` колонтитулов не знает
(`crates/docx/src/layout/engine.rs:239-243`). Пять фикстур
`test-fixtures/docx/headers_footers/` проверяют только разбор ссылок
(`crates/docx/src/document.rs:5333`). И canvas-путь их не рисует, и PDF.

**Раскладка не несёт:**

- списки и нумерацию — `w:numPr` переносится ссылкой, но `w:lvl` не
  применяется (`crates/docx/src/layout/cascade.rs:16`, раздел «Вне объёма»);
  маркера у `LayoutItem::Paragraph` нет, ни canvas, ни PDF буллеты не рисуют;
- сноски и концевые сноски — разбор есть (`crates/docx/src/model/annotation.rs`),
  в `layout`/`pdf` слова `footnote`/`endnote` нет;
- гиперссылки и поля в теле абзаца — `Inline` прочих вариантов раскладка «не
  несёт» (`crates/docx/src/layout/paragraph.rs:140-141`), текст ссылки не
  рисуется вовсе (в ячейках таблиц, наоборот, собирается — `layout/tables.rs:368`);
- `w:tab` и `w:br` (кроме разрыва страницы) — `run_text` берёт только
  `RunContent::Text` (`layout/paragraph.rs:315-323`);
- начертание внутри абзаца — отрезки сливаются по ключу «кегль + цвет», стиль
  берётся у первого run'а: «обычный **жирный** обычный» рисуется начертанием
  первого слова (`layout/paragraph.rs:92-96`), долг переехал из спринта 9;
- гарнитуру: рисование всегда идёт `"Carlito"` (`crates/docx/src/render.rs:26-33`),
  измерение — по `FontId::default()`, `w:rFonts` не влияет ни на переносы, ни
  на картинку;
- границы и заливку абзаца — `w:pBdr`/`w:shd` резолвятся каскадом
  (`layout/cascade.rs:157-159`), но ни в `LayoutItem`, ни в рендер не идут.

**HTML отдаёт начертания, но не прогонные цвет/кегль/гарнитуру.** Шов `Segment`
(`crates/docx/src/export/mod.rs:79`) несёт `Piece` и `Marks` — четыре флага
(полужирный, курсив, подчёркивание, зачёркивание; `export/mod.rs:29-40`), а
`mark()` в HTML выдаёт ровно `<strong>`, `<em>`, `<u>`, `<del>`
(`export/html.rs:437-452`); цвет, кегль и `w:rFonts` прогона не приходят —
в HTML они только из `docDefaults` (`export/html.rs:169-193`). Там же:
`w:vMerge` не даёт `rowspan` (умеем только `colspan` из `gridSpan` —
`export/html.rs:369-376`, у Markdown vMerge не читается вовсе), заголовок
таблицы в Markdown объявляется по первой строке (этого требует GFM —
`export/markdown.rs:108-127`), а в HTML — по `w:tblHeader`
(`export/html.rs:359-363`).

**В PDF не рисуются** границы ячеек `w:tcBorders`, заливка `w:shd` и вложенные
таблицы. Рисуется одинарная чёрная рамка 1 px по прямоугольнику каждой ячейки
(`crates/pdf/src/docx/tables.rs:7-11,51-83`): настоящие границы до раскладки не
доезжают — `TableCellLayout` несёт только `rect` и `content`
(`crates/docx/src/layout/engine.rs:153-165`). Вложенные таблицы теряются ещё на
раскладке: всё, что не абзац, из ячейки пропускается
(`crates/docx/src/layout/tables.rs:259-262`); в HTML-экспорте вложенность, в
отличие от PDF, сохраняется (`export/html.rs:335-338`).

**Деление фикстур `broken/` — 3 фатальных и 3 восстановимых, а не «6 битых».**
Фатальные (разбор обязан отвергнуть, ADR-0016 §2): `truncated_zip`,
`macro_enabled`, `no_document_xml` — `crates/docx/tests/warnings.rs:42-65`.
Восстановимые: `missing_root_rels`, `cyclic_based_on`,
`missing_style_and_abstract_num` — открываются как все остальные, с
предупреждениями (`warnings.rs:72-106`).

**Прочее.** Многоколоночная раскладка не сделана: `TODO (Спринт 10)` в
`crates/docx/src/layout/engine.rs:921` — единственный оставшийся маркер
спринта; в `crates/docx/src` и `crates/pdf/src` нет ни одного
`TODO (Спринт 11)`, `TODO (Фаза …)` и `todo!()`. Доккомментарий
`crates/docx/src/export/mod.rs:10` («HTML — отдельный слайс: `html::to_html`
пока заглушка») устарел: `export/html.rs` — 577 строк реализации.

## Дальше

1. **Рисунки DOCX** — отдельный слайс по решению пользователя. Сначала дать
   `docx` доступ к байтам медиачасти (`Document` не держит `Archive`,
   `parse_docx` роняет его после разбора), затем дорисовать встроенные рисунки
   в кадре и плавающие — прямоугольником реального содержимого, а не
   плейсхолдером; в PDF — тем же путём.
2. **Колонтитулы** — содержимое в `Document` разобрано (`Document.headers` /
   `footers`), но в раскладку не попадает: нужен `LayoutItem` и место в
   пагинации.
3. **Остатки таблиц в PDF**: `w:tcBorders`, заливка ячеек, вложенные таблицы.
4. **Спринт 11 по роадмапу** — SAB + CLI: batch-конвертация из командной
   строки, `WorkerPool`, оба пути (с SAB и без) и README про COOP/COEP.
5. **Многоколоночная раскладка** — `TODO (Спринт 10)` в
   `crates/docx/src/layout/engine.rs:921` остался: до неё заход не дошёл.
