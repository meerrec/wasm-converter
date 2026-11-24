# Stage 1: измеренная baseline и остаточные расхождения

## Привязка и область

- Ветка: `stage-1/offline-oracle`.
- Ревизия измерения: `404c8fe97d1a688ae5e7fac3d0c3844d4474ae75`; последующие commits этой ветки меняют только evidence/docs и не используются как основание для другого pixel report.
- Baseline ID: `office-online-2026-11-23-candidate`.
- Corpus: 20 DOCX и 20 XLSX, 84 manifests, 1936 reference PNG и 20 Word reference PDF.
- Среда capture: Linux, Chromium `151.0.7922.108`, DPR 1, zoom 100 %. Это не сертификация Windows 11 matrix `1.0.0`.
- References остаются `pending`: independent QA и два approvals ещё не выполнены. Совпадение повторных captures не означает визуальное соответствие собственного viewer.

Машинные результаты сохранены рядом: `stage-1-baseline-report.json`, `stage-1-classification.json`, `stage-1-reference-repeatability.json` и `stage-1-actual-repeatability.json`. Пути изображений в report относятся к capture-time workspace; эталонные PNG/PDF находятся в `test-fixtures/strict-viewer/references` через Git LFS.

## Воспроизводимость

| Проверка | Результат |
| --- | ---: |
| Actual DOCX, два прогона | 48/48 PNG и 40/40 PDF идентичны |
| Actual XLSX, два прогона | 1888/1888 PNG и 22/22 PDF идентичны |
| Всего actual | 1936/1936 captures и 62/62 PDF идентичны |
| Prepared Office references | 1936/1936 captures идентичны вне declared masks |
| Masked identical captures | 7, только `office-row-gutter-focus` |
| Manifest errors | 0 |
| JSON Schema | 84/84 manifests, 0 errors |
| PNG decoder/diff tests | 6/6 passed |
| Source digests | 20 DOCX + 20 XLSX passed |
| Native workspace | `cargo test --workspace --all-features --no-fail-fast` passed |
| Strict clippy | `cargo clippy --workspace --all-targets -- -D warnings` passed |
| Core/viewer | 225/225 core tests, typecheck и production build passed |
| Full actual capture | 84/84 tests в каждом из двух прогонов |
| Clean local clone | LFS checkout, source digests, 84 manifests, 1936 PNG, 20 PDF и 6/6 PNG tests passed; это не заменяет independent reproduction |

У Office PDF байты нестабильны между сессиями: 2/20 Word PDF совпали побайтово, 18/20 различаются. Для визуальной повторяемости используются decoded raster/print PNG; PDF digest differences не скрываются. В manifests также сохранены 34 записи об изменении Office application build между двумя сериями. Это не засчитывается как pixel difference, но ограничивает переносимость baseline.

## Normal canvas font readiness

До исправления `xlsx-07-sheet-1-normal` в двух полных прогонах чередовал правильные bold/italic glyphs с regular fallback, тогда как PDF был идентичен. Воркер содержал четыре загруженных `FontFace`, поэтому одного ожидания `FontFace.load()` и одинаковых соседних кадров оказалось недостаточно. Теперь renderer явно разрешает regular, bold, italic и bold italic через `measureText` в том же 2D-контексте до первого кадра; ошибка загрузки доступных browser FontFace API прерывает init вместо скрытого fallback. Регрессионный unit test закрепляет все четыре разрешения, полный core suite включает 225 passed tests. Два новых полных прогона на этой ревизии дали 1936/1936 идентичных PNG и 62/62 побайтово идентичных PDF.

## Полный offline report

Всего 1937 results: 1936 captures и одна независимая проверка page count.

| Status | Count |
| --- | ---: |
| `match` | 2 |
| `visual-diff` | 1934 |
| `page-mismatch` | 1 |

Единственная page-count ошибка: `charts-five-kinds` имеет 3 actual против 1 reference. `images-over-data` теперь имеет 1821 actual и 1821 reference; все выбранные страницы захвачены.

Triage classification, не доказанные root causes:

| Категория | Count | Первый технический фокус |
| --- | ---: | --- |
| `parse` | 0 | Новых parse-классов не выделено |
| `model` | 5 | Formula/error/date semantics и cached field results |
| `layout` | 36 | Sheet extent, visibility, overflow и scaling |
| `font` | 16 | Метрики и rasterization текста |
| `render` | 11 | Charts/images и cell painting |
| `harness` | 1866 | Office PNG 594×815 против native PDF raster 1191×1684; растягивание запрещено |
| `reference` | 1 | Public Excel Print Preview не показывает пять OOXML charts |
| `match` | 2 | Все unmasked pixels удовлетворяют `visual-v1` |

Наиболее приоритетный backlog: сначала несовпадения raster dimensions и reference limitation для charts, затем пагинация и layout, затем font metrics, затем декоративные различия. Dimension mismatch не превращается в engine defect и не разрешается ресайзом.

## Инженерные регрессии #73/#74

Общий табличный код теперь квантует полную высоту строки до 0,5 pt вверх, учитывает default text inset и не втискивает строку за счёт отброшенных интервалов. Нативный тест `office_long_table_page_breaks_and_repeated_header` подтверждает:

- `long_table`: 2 страницы, 31 строка на первой и 29 на второй;
- `long_table_header_repeat`: 2 страницы, 31 строка на первой и 10 на второй;
- ровно один repeated header на continuation page;
- continuation body начинается со строки 31, split rows не появляются;
- измеренная row height — `30.666666`.

Полный `doc-converter-docx` native suite прошёл. Визуальные остатки не скрываются:

| Fixture | Mode/page | Mismatching pixels | Mismatch ratio | SSIM |
| --- | --- | ---: | ---: | ---: |
| `long_table` | print 1 | 87719 | 0.0983770 | 0.736007 |
| `long_table` | print 2 | 82170 | 0.0921538 | 0.762563 |
| `long_table` | PDF 1 | 137116 | 0.0683651 | 0.831511 |
| `long_table` | PDF 2 | 133807 | 0.0667152 | 0.837812 |
| `long_table_header_repeat` | print 1 | 85932 | 0.0963728 | 0.736928 |
| `long_table_header_repeat` | print 2 | 25062 | 0.0281071 | 0.931700 |
| `long_table_header_repeat` | PDF 1 | 134724 | 0.0671724 | 0.830350 |
| `long_table_header_repeat` | PDF 2 | 56557 | 0.0281989 | 0.916812 |

Число страниц и разрывы подтверждены, но полного pixel match нет. Следующий этап должен разбирать остаточную геометрию границ и текста, а не изменять эталонные PNG.

## Результат #76

Реализованы общий `print_extent`, chart/image anchors, дальний `oneCellAnchor`, overflow через `render::text_measure`, ограничение 1024 отображаемых символов и пустые страницы внутри print area. Полный `doc-converter-pdf` native suite прошёл.

| Книга | Office reference | Actual | Состояние |
| --- | ---: | ---: | --- |
| `layout-frozen-columns` | 2 | 2 | Page count совпадает |
| `values-long-text` | 12 | 12 | Page count совпадает |
| `charts-five-kinds` | 1 | 3 | Public Excel Normal/Print Preview опускает charts; native engine сохраняет OOXML charts |
| `images-over-data` | 1821 | 1821 | Distant checkerboard находится на последней странице; страниц после объекта нет |

Удалять charts или обрезать distant image ради pixel/page-count match нельзя. Старый вывод о пустых Office pages 1820–1821 был основан на invalid capture после страницы 100 и отозван. Печатная геометрия столбцов отделена от экранной: standard column остаётся 64 px в Normal и использует 68,1 px в Print Preview. Regression закрепляет дальний `oneCellAnchor` на странице 1821 и отсутствие дублирования на странице 1820. Полный actual capture подтверждает checkerboard на последней странице. Технические roadmap checkboxes #76 выполнены; issue не закрывается без явного подтверждения maintainer. Расхождение charts классифицировано как ограничение public reference, а не как основание удалить содержимое.

## Команды

Offline capture/diff и классификация описаны в `oracle.md`. Обычный test не вызывает Office и не изменяет references. Подготовка новых references выполняется только отдельной командой с явным `--output`; принятие candidate требует independent QA и двух approvals.
