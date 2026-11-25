# Матрица сертификации строгого просмотра DOCX/XLSX

**Версия матрицы:** `1.0.0`  
**Состояние:** `accepted`  
**Принято:** 22.11.2026 (решения `accept-modes`, `accept-environment`, `accept-visual-v1`, `accept-dynamic`, `accept-exclusions`, `accept-storage`, `accept-baseline`, `accept-feature-report`); 25.11.2026 — `accept-capture-automation`  
**Задача:** [#79](https://github.com/meerrec/wasm-converter/issues/79)  
**Схема manifest:** [`manifest.schema.json`](manifest.schema.json), версия `1.1.0`  
**Политика эталонов:** [`baseline-policy.md`](baseline-policy.md)

Этот документ определяет принятую область первой сертификации. Он не утверждает, что перечисленные возможности уже совпадают с Office Online, и не принимает 40 кандидатских документов как baseline. Статус `supported` означает обязательную цель сертификации, а не измеренное соответствие.

## 1. Правила чтения матрицы

Используются четыре статуса:

| Статус | Значение |
|---|---|
| `supported` | Возможность включена в обязательную сертификацию. Требуются native/unit и визуальные проверки по каждой указанной странице или экрану. |
| `unsupported-visible` | Возможность входит в область обнаружения, но не в область визуального соответствия. Наличие ограничения обязано попасть в `FeatureSupportReport` и стать видимым в UI/CLI. |
| `excluded` | Возможность исключена из первой сертификации. Изменение статуса требует новой строки или новой версии решения, минимальной фикстуры и явного решения maintainer. |
| `planned` | Возможность не входит в первую сертификацию, но запланирована в следующих этапах roadmap. До смены статуса визуальное соответствие не заявляется. |

Для `supported` обязательны четыре независимых результата: полное текстовое содержимое, точное число и порядок страниц или экранов, присутствие объектов и прохождение визуального профиля `visual-v1`. Средний SSIM по корпусу не заменяет ни один из них. Для DOCX не используется допуск «±1 страница».

Пути фикстур указаны от `test-fixtures/`. Запись `требуется новая` означает блокер для перехода соответствующей части строки в `supported`; нельзя молча заменить её другой фикстурой. Одна фикстура может быть минимальной для нескольких строк, но каждая проверка записывается отдельно.

## 2. Принятые режимы и среда

### 2.1 Решения о режимах

| ID решения | Решение | Состояние |
|---|---|---|
| `mode-docx` | Сертифицировать `docx.print-layout` и `docx.pdf` как два представления одной принятой пагинации. Web/Print Preview Word, Page Layout как отдельный дополнительный режим, показ разметки ревизий и комментариев в первую матрицу не включать. | `accepted` |
| `mode-xlsx` | Сертифицировать `xlsx.normal` и `xlsx.print-preview` независимо. Normal view проверяет видимый диапазон, Print Preview проверяет каждую печатную страницу выбранного листа. | `accepted` |
| `hidden-sheets` | Скрытые и очень скрытые листы не открывать и не печатать. Их имена, порядок и состояние включать в структурный отчёт; попытка захвата такого листа является `harness-error`. | `accepted` |
| `revisions-comments` | Для DOCX использовать принятое отображение документа без balloons и без панели рецензирования. Отдельно фиксировать политику обнаружения ревизий и комментариев как `unsupported-visible`, пока их режимы не приняты. | `accepted` |

Каждый режим документа описывается отдельным manifest. Наборы captures разных режимов не смешиваются, даже если они относятся к одному исходному DOCX/XLSX.

### 2.2 Фиксированная среда захвата

Конкретные build-номера обязаны попасть в каждый manifest. Ниже заданы принятые значения, которые нельзя менять внутри одного baseline:

| Параметр | `docx.print-layout`, `xlsx.normal`, `xlsx.print-preview` | `docx.pdf` |
|---|---|---|
| Office | Локальный Microsoft Word/Excel (COM, `ExportAsFixedFormat`) либо Word Online / Excel Online; точное product/build metadata в manifest | Экспорт PDF из того же Office, что и исходный захват; product/build metadata в manifest |
| OS | Windows 11, build записывается в manifest | Та же машина и OS, что у исходного захвата |
| Browser | Chromium stable, полная версия записывается | Не применимо для рендера PDF; браузер экспорта записывается |
| Viewport CSS | `1440 × 900` | Не применимо |
| DPR | `1` | Не применимо |
| Application zoom | `100 %` | `100 %` при экспорте |
| Browser zoom | `100 %` | `100 %` |
| Locale / timezone | `ru-RU` / `UTC`, если документ не требует иной явно записанной локали | Наследуются от исходного документа |
| Color scheme | `light` | Не применимо |
| Rasterization | PNG напрямую, без JPEG | `pdftoppm -png -r 144`, полная версия записывается |
| Fonts | Версионированный набор и SHA-256 каждого файла; системный fallback запрещён | Тот же набор; использованные PDF fonts сохраняются в metadata |

Office UI, лента, панели навигации, курсоры, выделение и подсветка поиска не входят в изображение. Reference содержит только область страницы, печатной страницы или согласованный видимый диапазон листа. Для Normal view начальные `scrollX=0`, `scrollY=0`, активная ячейка `A1` и отсутствие выделения являются принятым умолчанием; любое другое состояние явно записывается в capture.

### 2.3 Профиль визуальной проверки `visual-v1`

Принятый профиль применяется к каждому capture отдельно:

1. Reference и actual обязаны иметь одинаковые ширину и высоту. Иначе результат `page-mismatch`, а метрики пикселей не используются для оправдания результата.
2. Вне разрешённых масок сравниваются все пиксели RGBA. Цветовой допуск: `max(|Rref−Ractual|, |Gref−Gactual|, |Bref−Bactual|) <= 2`; alpha обязана совпадать точно.
3. `match` требует нуля пикселей вне допуска. Любое другое значение даёт `visual-diff`.
4. SSIM вычисляется для каждой страницы или экрана и сохраняется как диагностика, но не смягчает правило нуля расходящихся пикселей.
5. Маски применяются одинаково к reference и actual. Замаскированная область исключается только из pixel diff и SSIM; исчезновение объекта или текста вне маски остаётся ошибкой.
6. Текстовые, структурные и геометрические проверки не маскируются. Они выполняются native/unit-тестами и browser golden-проверками независимо от изображения.

Численный порог принят решением `accept-visual-v1`; `visual-v1` является обязательным release gate после реализации #77.

## 3. Динамические данные

| Источник изменчивости | Принятая политика | Допустимая маска |
|---|---|---|
| `DATE`, `TIME`, `NOW`, `TODAY` | По возможности использовать фикстуру с фиксированным результатом и зафиксировать значение в manifest. Пересчёт при захвате запрещён, если он меняет исходное значение. | Только точный прямоугольник значения, причина `dynamic-date` и ожидаемое текстовое правило. |
| `PAGE`, `NUMPAGES`, `REF`, TOC | Проверять как содержимое. Маскировать нельзя. | Нет. |
| Cached formula values | Использовать сохранённый cache; не запускать полный пересчёт в собственном вьюере. | Нет. |
| `dirty`, `fullCalcOnLoad`, `forceFullCalc` | Сохранять признаки в `FeatureSupportReport`. Reference фиксирует фактически отображённый cache, без молчаливого обновления. | Нет. |
| External links | Не обращаться к внешнему источнику. Использовать сохранённые значения; состояние ссылки видимо в отчёте. | Нет. |
| Имя пользователя, путь файла, случайные значения | Исключать из новых фикстур или нормализовать до фиксированного значения перед захватом. | Только отдельным решением с причиной и координатами. |
| Анимация, blinking cursor, hover | Отключать состояние захвата, а не маскировать меняющуюся картинку. | Нет. |

Маска обязана содержать стабильный ID, координаты в пикселях reference, причину и ожидаемое правило проверки. Маска «вся страница», автоматически найденная разница и маска без текстовой замены запрещены.

## 4. Матрица DOCX

Сокращения режимов: `PL` = `docx.print-layout`, `PDF` = `docx.pdf`. Для всех строк `supported` проверяются все страницы минимальной фикстуры, полный текст и точное число страниц. Дополнительно применяются критерии из последней колонки.

| ID | Возможность и OOXML | Режимы | Статус | Минимальная фикстура | Дополнительные фикстуры | Специфические критерии и ограничения |
|---|---|---|---|---|---|---|
| `docx.paragraph-runs` | `w:p`, `w:r`, whitespace, breaks, tabs | PL, PDF | `supported` | `docx/basic/multiple_runs.docx` | `docx/simple/one_paragraph.docx`, `docx/basic/breaks_and_tabs.docx` | Границы runs не меняют текст; точные line breaks, tab stops, X-координаты глифов. |
| `docx.styles-themes` | `styles.xml`, `docDefaults`, `basedOn`, direct formatting, theme colors/fonts | PL, PDF | `supported` | `docx/styles/based_on_chain.docx` | `docx/styles/doc_defaults.docx`, `docx/styles/direct_formatting_override.docx` | Resolved style совпадает с каскадом ADR-0013; family, size, weight, color и отступы проверяются native. |
| `docx.numbering` | `numbering.xml`, `abstractNum`, `num`, overrides, multilevel | PL, PDF | `supported` | `docx/numbering/multilevel.docx` | `docx/numbering/start_override.docx`, `docx/numbering/restart_numbering.docx` | Маркер, текст, indent, hanging и restart совпадают на каждом уровне. |
| `docx.sections-columns` | `w:sectPr`, page size/margins, `w:cols`, section/page/column breaks | PL, PDF | `supported` для секций; колонки `planned` | `docx/basic/sections.docx` | `docx/columns/two_equal.docx`, `docx/columns/three_unequal.docx` | Геометрия страниц, смена секций и число колонок. Колонки остаются `planned` внутри строки, но фикстура их уже показывает. |
| `docx.headers-footers` | default/first/even/odd headers and footers, relationships | PL, PDF | `supported` | `docx/headers_footers/default_header_footer.docx` | `docx/headers_footers/even_odd.docx`, `docx/headers_footers/title_pg_first.docx` | Текст и геометрия колонтитулов на каждом типе страницы; связи не теряются. |
| `docx.tables-fixed-autofit` | `w:tblLayout`, grid, widths, alignment | PL, PDF | `supported` | `docx/tables/simple_2x2.docx` | `docx/tables/alignment_widths.docx` | Native: X/Y ячеек, ширины, высоты, границы; visual: вся таблица на всех страницах. |
| `docx.tables-merged-nested` | `gridSpan`, `vMerge`, nested `w:tbl` | PL, PDF | `supported` | `docx/tables/grid_span.docx` | `docx/tables/v_merge.docx`, `docx/tables/nested.docx` | Occupancy grid, границы merged-областей и вложенный контент без дублей и пропусков. |
| `docx.tables-pagination` | Long tables, `tblHeader`, `cantSplit`, rows taller than page | PL, PDF | `supported` | `docx/tables/long_table.docx` | `docx/tables/long_table_header_repeat.docx`, `docx/tables/cant_split_rows.docx`, `docx/tables/row_taller_than_page.docx` | Точный page break, повтор заголовка, высоты строк, границы и отсутствие потерянных строк. |
| `docx.tables-appearance` | Borders, shading, cell margins, vertical alignment | PL, PDF | `supported` | `docx/tables/borders_shading.docx` | `docx/tables/cell_margins_valign.docx` | Цвет и толщина каждого ребра, fill, padding и baseline текста. |
| `docx.images-inline` | `w:drawing`, `wp:inline`, DrawingML extents | PL, PDF | `supported` | `docx/images/inline.docx` | `docx/images/multiple_sizes.docx` | Порядок, aspect ratio, crop, размер и baseline изображения; SHA-256 media сверяется с моделью. |
| `docx.images-floating` | `wp:anchor`, positioning, z-order, wrap | PL, PDF | `planned` | `docx/images/anchor_wrap_square.docx` | `docx/images/anchor_behind_text.docx`, `docx/images/anchor_wrap_tight.docx` | Первая сертификация обнаруживает объект и ограничение wrap. Переход в `supported` требует отдельного решения по каждому wrap. |
| `docx.shapes-textboxes` | DrawingML shapes, groups, transforms, text boxes | PL, PDF | `unsupported-visible` | `docx/alternate_content/choice_shape.docx` | `docx/alternate_content/group_shape.docx`, `docx/alternate_content/choice_fallback_textbox.docx` | Не удалять молча; код `docx-shape-unsupported`, fixture ID и bounds, если известны. Визуальное равенство не заявляется. |
| `docx.footnotes-endnotes` | `footnotes.xml`, `endnotes.xml`, references | PL, PDF | `planned` | `docx/notes/footnote_basic.docx` | `docx/notes/footnote_in_table.docx`, `docx/notes/endnote_basic.docx` | До включения: присутствие ссылок и ограничение в report. Для `supported` потребуется пагинация note area. |
| `docx.comments` | `comments.xml`, comment ranges/references | PL, PDF | `unsupported-visible` | `docx/notes/comments_basic.docx` | `docx/notes/comments_multiple.docx` | Комментарии не исчезают из report; balloons/panel исключены решением `revisions-comments`. |
| `docx.revisions` | `w:ins`, `w:del`, moves, property changes | PL, PDF | `unsupported-visible` | `docx/track_changes/inserted_run.docx` | `docx/track_changes/deleted_run.docx`, `docx/track_changes/move_and_format_change.docx` | Режим accepted/final показываемого текста должен быть зафиксирован до визуальной сертификации; revision UI не входит. |
| `docx.math` | Office Math `m:oMath`, `m:oMathPara` | PL, PDF | `unsupported-visible` | `docx/math/omml_simple.docx` | Нет | Код `docx-math-unsupported`; текст/MathML не подменяется визуальным соответствием. |
| `docx.fields-toc-refs` | `fldSimple`, `fldChar`, `instrText`, TOC, REF, hyperlinks | PL, PDF | `supported` для сохранённых результатов; генерация `planned` | `docx/fields/fld_simple.docx` | `docx/fields/nested_fields.docx`, `docx/fields/hyperlink_field.docx`, `docx/fields/toc_ref.docx` | Отображается сохранённый результат; поле не пересчитывается. Автоматическая генерация TOC/REF не заявляется. |
| `docx.vml-alternate` | VML, `mc:AlternateContent`, Choice/Fallback | PL, PDF | `unsupported-visible` для VML; `supported` для выбора поддерживаемой Choice | `docx/alternate_content/choice_fallback_textbox.docx` | `docx/alternate_content/fallback_only.docx`, `docx/alternate_content/nested_alternate.docx` | Политика Choice соответствует ADR-0014; отсутствие поддерживаемой ветки видимо, VML fallback молча не используется. |
| `docx.smartart` | SmartArt diagrams and related parts | PL, PDF | `excluded` | `docx/embedded/smartart_basic.docx` | Нет | Из первой визуальной матрицы исключено; при наличии в документе требуется `docx-smartart-excluded`. |
| `docx.ole` | OLE objects, embeddings, previews | PL, PDF | `excluded` | `docx/embedded/ole_object.docx` | Нет | Из первой визуальной матрицы исключено; при наличии требуется `docx-ole-excluded`, preview не считается поддержкой OLE. |
| `docx.bidi-cjk` | Unicode, bidi, RTL, CJK, combining and astral characters | PL, PDF | `planned` до завершения этапа 2 | `docx/edge_cases/multilingual.docx` | `docx/rtl/mixed_direction.docx`, `docx/cjk/mixed_latin.docx`, `docx/basic/astral_unicode.docx` | Полный текст обязателен уже сейчас; строгий glyph/shaping gate включается после детерминированного pipeline этапа 2. |

## 5. Матрица XLSX

Сокращения режимов: `N` = `xlsx.normal`, `PP` = `xlsx.print-preview`. Для `supported` проверяются порядок видимых листов, активный лист, текст или отображаемые значения и каждый выбранный экран либо каждая печатная страница. Print Preview обязателен там, где печатная геометрия может отличаться от экранной.

| ID | Возможность и OOXML | Режимы | Статус | Минимальная фикстура | Дополнительные фикстуры | Специфические критерии и ограничения |
|---|---|---|---|---|---|---|
| `xlsx.values` | Cell types, shared/inline strings, booleans, errors | N, PP | `supported` | `xlsx/content-mixed-types.xlsx` | `xlsx/values-errors.xlsx`, `xlsx/values-unicode.xlsx` | Display text и тип значения совпадают; ошибки не заменяются пустыми ячейками. |
| `xlsx.formula-cache` | Formulas with cached values, dirty/fullCalcOnLoad flags | N, PP | `supported` для cache-only | `xlsx/values-formulas.xlsx` | `xlsx/edge-formula-chain.xlsx`, `xlsx/edge-formula-error.xlsx` | Используется сохранённый cache; признаки устаревания видимы в report; полный recalculation не выполняется. |
| `xlsx.number-formats` | Built-in/custom `numFmt`, currency, percent, fractions, scientific | N, PP | `supported` | `xlsx/formats-thousands-decimals.xlsx` | `xlsx/formats-currency-ruble.xlsx`, `xlsx/formats-fraction.xlsx` | Отображаемый текст, decimal separators, negative/zero sections и `#####` проверяются отдельно. |
| `xlsx.dates-locales` | Serial dates, 1900/1904, date/time formats, locale | N, PP | `supported` для 1900; 1904 `planned` | `xlsx/formats-date-ru.xlsx` | `xlsx/formats-date-iso.xlsx`, `xlsx/formats-date-1904.xlsx` | Epoch, serial value, locale и timezone записаны; 1900 leap-year compatibility проверяется native. Сертификация 1904 начинается после перехода возможности в `supported`. |
| `xlsx.dimensions` | Row heights, column widths, zero sizes, defaults | N, PP | `supported` | `xlsx/layout-column-widths.xlsx` | `xlsx/layout-row-heights.xlsx`, `xlsx/edge-zero-height-row.xlsx` | Pixel boundaries каждой строки/столбца и hidden state совпадают. |
| `xlsx.wrap-overflow` | `wrapText`, unwrapped overflow, clipping, shrink-to-fit, rotation | N, PP | `supported` для wrap/overflow; остальное `planned` | `xlsx/values-long-text.xlsx` | `xlsx/text-cyrillic-wrap.xlsx`, `xlsx/layout-shrink-to-fit.xlsx`; `требуется новая: rotation` | Точки переноса, clipping и print extent совпадают; overflow не исчезает при соседнем содержимом. `shrinkToFit` фикстурой `xlsx/layout-shrink-to-fit.xlsx` выражен, но движком только разбирается (`crates/xlsx/src/styles.rs`) и в раскладке не применяется — до реализации не заявляется. |
| `xlsx.rich-merged` | Rich text runs, merged cells and merged styling | N, PP | `supported` | `xlsx/content-rich-text.xlsx` | `xlsx/layout-merged.xlsx`, `xlsx/edge-merged-styled.xlsx` | Run formatting, anchor cell, bounds и границы merged-диапазона. |
| `xlsx.hidden-outlines` | Hidden rows/columns, outlines, collapsed groups | N, PP | `supported` для hidden; outlines `planned` | `xlsx/layout-hidden-columns.xlsx` | `xlsx/layout-hidden-rows.xlsx`; `требуется новая: outlines` | Скрытые размеры не создают пустые разрывы; outline state до включения попадает в report. |
| `xlsx.freeze-scroll-zoom` | Frozen panes, sheet zoom, scroll state | N | `supported` | `xlsx/layout-frozen-columns.xlsx` | `xlsx/layout-frozen-rows.xlsx`, `xlsx/layout-frozen-both.xlsx` | Квадранты, clipping, dividers, scroll и zoom записаны; frozen content не дублируется. |
| `xlsx.styles-themes` | Fonts, fills, borders, alignment, themes | N, PP | `supported` | `xlsx/styles-bold-italic.xlsx` | `xlsx/styles-border-styles.xlsx`, `xlsx/styles-pattern-fills.xlsx`, `xlsx/styles-alignment.xlsx` | Resolved style, theme color, border priority, alignment и fill проверяются native и visual. |
| `xlsx.conditional` | `cellIs`, `expression`, colorScale, dataBar, iconSet, priority/stopIfTrue | N, PP | `supported` | `xlsx/cf-cell-is.xlsx` | `xlsx/cf-expression.xlsx`, `xlsx/cf-color-scale.xlsx`, `xlsx/cf-data-bar.xlsx`, `xlsx/cf-icon-set.xlsx`, `xlsx/cf-priorities.xlsx` | Applied style и геометрия визуальных правил; приоритет и stopIfTrue проверяются native. |
| `xlsx.images` | oneCell/twoCell anchors, images, crop, z-order | N, PP | `supported` | `xlsx/images-over-data.xlsx` | `xlsx/images-png.xlsx`, `xlsx/images-jpeg.xlsx` | Anchor bounds, clamping, crop, print extent, порядок и SHA-256 media. |
| `xlsx.shapes` | Shapes and text boxes | N, PP | `unsupported-visible` | `xlsx/sheets-different-shapes.xlsx` | Нет | Код `xlsx-shape-unsupported`; наличие и bounds попадают в report, визуальная поддержка не заявляется. |
| `xlsx.charts` | Bar, line, pie, scatter, area; axes, legend, labels | N, PP | `supported` для пяти существующих видов; комбинированные `planned` | `xlsx/charts-five-kinds.xlsx` | `требуется новая: combined chart` | Chart count, type, series order, bounds, axes/legend/labels и включение в print extent. |
| `xlsx.comments-notes` | Legacy comments/notes and threaded comments | N, PP | `unsupported-visible` | `xlsx/comments-legacy.xlsx` | `требуется новая: threaded comments` | Комментарий не влияет на page extent без явного print setting; наличие видимо в report. |
| `xlsx.pivot-slicers-controls` | Pivot tables, slicers, form controls, ActiveX | N, PP | `excluded` | `xlsx/pivot-basic.xlsx` | Нет | Из первой визуальной матрицы исключено; обнаружение даёт `xlsx-pivot-slicer-control-excluded`. |
| `xlsx.print-areas-breaks` | Print areas, manual row/column breaks, page order | PP | `planned` | `xlsx/print-area-titles.xlsx` | `требуется новая: manual breaks` | Точное число и порядок страниц, print extent и отсутствие лавинообразных пустых страниц. Переход в `supported` требует фикстуры print area/manual breaks. |
| `xlsx.print-titles-scale` | Repeating titles, scale, fit-to-width/height | PP | `supported` для scale; repeating titles/fit `planned` | `xlsx/print-area-titles.xlsx` | `xlsx/print-fit-to-page.xlsx` | Scale применяется один раз. Заголовки повторяются, fit limits и page breaks совпадают после добавления соответствующей фикстуры и перехода этих возможностей в `supported`. |
| `xlsx.print-headers-footers` | Print headers/footers, margins, orientation, paper size | PP | `planned` | `xlsx/print-headers-footers.xlsx` | Нет | Текст, положение, поля, paper size и orientation. Строка блокирует полное принятие печатной матрицы до появления фикстуры. |
| `xlsx.sheets-visibility` | Sheet order, active sheet, hidden/veryHidden, multi-sheet print | N, PP | `supported` | `xlsx/sheets-three.xlsx` | `xlsx/sheets-hidden.xlsx`, `xlsx/sheets-very-hidden.xlsx` | Видимые листы сохраняют порядок; hidden листы не захватываются; multi-sheet print ограничен явным списком captures. |
| `xlsx.external-links` | External workbook references and cached values | N, PP | `unsupported-visible` для обновления; `supported` для offline display cache | `xlsx/layout-broken-link.xlsx` | `xlsx/external-workbook-link.xlsx` | Сетевых запросов нет; cache и признак внешней связи сохраняются, обновление не выполняется. |

## 6. Документный контракт `FeatureSupportReport`

Это принятая документная модель данных, а не изменение публичного API. Реализация и экспорт из `@meerrec/core` требуют отдельного слайса, ADR при архитектурном изменении и двух approver для публичного API.

| Поле | Тип и обязательность | Назначение |
|---|---|---|
| `schemaVersion` | string, обязательное | Версия отчёта, независимая от версии manifest. |
| `format` | `docx` или `xlsx`, обязательное | Формат источника. |
| `documentSha256` | 64 lowercase hex, обязательное | Связь с исходным документом. |
| `matrixVersion` | string, обязательное | Матрица, по которой классифицированы возможности. |
| `entries` | array, обязательное | По одной записи на обнаруженную возможность или экземпляр ограничения. |
| `entries[].featureId` | стабильный ID из матрицы | Например, `docx.tables-pagination`. |
| `entries[].code` | стабильный машинный код | Например, `docx-math-unsupported`. |
| `entries[].status` | `supported`, `unsupported-visible`, `excluded`, `planned` | Решение матрицы, а не результат pixel diff. |
| `entries[].location` | optional object | OOXML part, XPath-like locator, sheet/cell/page, без содержимого документа в telemetry. |
| `entries[].boundsPx` | optional rectangle | Координаты видимого ограничения, если их можно надёжно определить. |
| `entries[].message` | string, обязательное | Человекочитаемое ограничение для UI/CLI. |
| `entries[].fixturePath` | string, обязательное для ограничения | Минимальная регрессионная фикстура. |
| `entries[].documentation` | string, обязательное для ограничения | Относительная ссылка на матрицу, ADR или roadmap. |
| `entries[].count` | integer ≥ 1 | Число одинаковых вхождений при агрегации. |
| `summary` | object, обязательное | Counts по статусам и `hasVisibleLimitations`. |
| `formulaCache` | optional object | `dirty`, `fullCalcOnLoad`, `forceFullCalc`, external links и выбранный cache-only режим. |
| `fontSubstitutions` | array, обязательное | Все фактические подстановки; незаявленная подстановка запрещена в сертифицированном режиме. |

### 6.1 Коды ограничений и поведение

| Код | Применение | UI | CLI/JSON |
|---|---|---|---|
| `docx-shape-unsupported` | Shapes/text boxes | Постоянный значок ограничения и запись в панели совместимости | Ненулевой статус только в строгой команде сертификации; запись в structured report |
| `docx-comments-visible-limitation` | Комментарии без сертифицированного режима | Уведомление без скрытия основного документа | Запись с count и fixture |
| `docx-revisions-visible-limitation` | Ревизии без выбранного режима | Уведомление и выбранное final/accepted состояние | Запись с revision mode |
| `docx-math-unsupported` | OMML | Видимое ограничение вместо молчаливого пропуска | Запись с location |
| `docx-smartart-excluded` | SmartArt | Видимое исключение | Запись с location |
| `docx-ole-excluded` | OLE | Видимое исключение | Запись с location |
| `xlsx-shape-unsupported` | Shapes/text boxes | Видимое ограничение | Запись с sheet/location |
| `xlsx-comments-visible-limitation` | Comments/notes | Индикатор наличия notes без заявления полного соответствия | Запись с sheet/cell |
| `xlsx-pivot-slicer-control-excluded` | Pivot/slicers/controls | Видимое исключение | Запись с sheet/location |
| `xlsx-formula-cache-stale` | Признаки dirty/full recalculation | Предупреждение о cache-only результате | Структурированные flags |
| `xlsx-external-link-not-updated` | External links | Предупреждение без попытки сетевого запроса | Запись с count, без URL в telemetry |

Обычный просмотр может показать поддерживаемую часть документа вместе с ограничениями. Строгая сертификация не имеет права превратить `unsupported-visible` или `excluded` в `match`. Скрытое удаление объекта, смена шрифта, пересчёт формулы, внешний запрос и снижение качества являются дефектами контура.

## 7. Связь с кандидатским корпусом #75

Предварительные 20 DOCX и 20 XLSX остаются кандидатами — это минимальный порог, а не потолок: реестр растёт под приоритетный состав по областям матрицы. Обязательные шесть регрессионных фикстур уже представлены в матрице: `long_table`, `long_table_header_repeat`, `charts-five-kinds`, `images-over-data`, `values-long-text`, `layout-frozen-columns`. Их SHA-256 проверяются при создании каждого manifest.

До принятия #75 необходимо закрыть пробелы, обозначенные как `требуется новая`, для возможностей `supported`, либо явно оставить соответствующую часть в `planned` или другом неподдерживаемом статусе. Недопустимо сохранять часть строки `supported`, удалив отсутствующую фикстуру из проверки. Окончательный corpus index должен ссылаться на эту версию матрицы, различать manifest по `source.path` и `view.mode`, содержать ровно выбранные документы и проходить проверки из [`baseline-policy.md`](baseline-policy.md).

## 8. Принятые решения

| ID | Подтверждённое решение | Разблокирует |
|---|---|---|
| `accept-modes` | Режимы DOCX/XLSX, hidden sheets, comments/revisions | #75 |
| `accept-environment` | OS, browser, viewport, DPR, zoom, locale, PDF DPI и версионированные шрифты | #75, #72 |
| `accept-visual-v1` | Цветовой допуск, нулевой mismatch и диагностическую роль SSIM | #77, release gate |
| `accept-dynamic` | Политику полей, дат, external links, formula cache и масок | #75, #77 |
| `accept-exclusions` | SmartArt, OLE, Pivot/slicers/controls и остальные `unsupported-visible` | Матрица, UI/CLI contract |
| `accept-storage` | Хранилище PNG, URI-схему, предельный размер корпуса и retention | #75 |
| `accept-baseline` | Процедуру review, approvers, rollback и запрет auto-update | #75, #77 |
| `accept-capture-automation` | Способ захвата эталона: COM-автоматизация локального Word/Excel вместо обязательного ручного Office Online | #75, #80 |
| `accept-feature-report` | Документные поля и коды; отдельно решение о публичной реализации | Последующий API slice |

Все восемь решений приняты 22.11.2026. Дальнейшие изменения области сертификации требуют новой версии матрицы и явного PR. Завершение Stage 0 фиксируется в roadmap со ссылкой на PR #80.
