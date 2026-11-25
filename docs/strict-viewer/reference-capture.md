# Захват эталона и прогон оракула

Операторская инструкция: как снять эталон (автоматически через COM-автоматизацию
локального Word/Excel или вручную в Office Online), как превратить его в
manifest и как прогнать собственный вьюер против принятого baseline. Политика
хранения и принятия — [`baseline-policy.md`](baseline-policy.md), состав
корпуса — [`certification-matrix.md`](certification-matrix.md) и
[`test-fixtures/strict-viewer/README.md`](../../test-fixtures/strict-viewer/README.md).

Инструмент живёт в `tools/visual-oracle`; из корня репозитория он вызывается
как `pnpm visual-oracle <command>`.

## 1. Что где лежит

```text
test-fixtures/strict-viewer/
  corpus.json                                  # единственный источник истины
  <format>/<fixture-id>/<mode>/
    manifest.json                              # среда, captures, хеши
    reference/page-1.png                       # эталон (Git LFS)
    reference/export.pdf                       # только для mode = "pdf"
```

`<fixture-id>` — slug от `source.path`: расширение отбрасывается, строка
приводится к нижнему регистру, прогоны не-алфавитно-цифровых символов
схлопываются в `-`. `docx/tables/long_table.docx` → `docx-tables-long-table`.

PNG и PDF эталонов хранятся в Git LFS (`.gitattributes`). После клона нужен
`git lfs pull`, иначе на диске окажутся указатели и каждый manifest упадёт на
несовпадении SHA-256.

## 2. Снять эталон

Захват делается по §3 baseline-policy: автоматически —
`scripts/strict-viewer-references.ps1` (Windows) или
`scripts/strict-viewer-references-macos.sh` (macOS; см. `docs/testing/strict-viewer-references.md`),
либо **вручную** в Word Online / Excel Online. Наш движок эталоном не является
никогда: `actual/` не копируется в `reference/`.

1. Открыть документ на машине подготовки baseline, выставить состояние из
   будущего manifest: режим, лист, страницу, диапазон, scroll, zoom, DPR,
   `comments: hidden`, `revisions: accepted`, скрытые листы не открывать.
2. Дождаться загрузки шрифтов и стабильного изображения по наблюдаемому
   состоянию, а не по фиксированному `sleep`.
3. Сохранить PNG напрямую, без JPEG. Для `docx.pdf` сохранить исходный PDF и
   растеризовать его: `pdftoppm -png -r 144 export.pdf page`.
4. Повторить захват в чистой сессии той же среды: расхождение двух эталонов —
   проблема среды или документа, и маскировать его нельзя.

## 3. Собрать drop и импортировать manifest

Импорт ожидает каталог, последние три сегмента которого —
`<format>/<fixture-id>/<mode>`:

```text
<drop>/docx/docx-tables-long-table/print-layout/
  capture.json
  reference/page-1.png
```

`capture.json` описывает то, что знает только оператор: источник, среду,
режим, политику динамических данных и список captures. Импортёр сам считает
`source.sha256`/`bytes`, `image.sha256`/`bytes`/`width`/`height`/`colorType`,
`reference.exportedPdf.sha256` и `baselineId` — выдуманное значение здесь
сделало бы baseline невоспроизводимым.

`captures[].image` задаётся относительно drop и обязан начинаться с
`reference/`: `reference/page-1.png` — верно, `page-1.png` или
`actual/page-1.png` — нет, потому что в manifest попал бы URI на файл, которого
нет в папке захвата. Импорт падает с
`captures[0].image "page-1.png" must live under reference/`;
`reference.exportedPdf.uri` обязан быть ровно `reference/export.pdf`.

### Поля `reference`

Способ захвата записывается в manifest без округлений: локальный Word/Excel
через COM — это не Office Online, и наоборот.

| Поле | Допустимые значения |
|---|---|
| `reference.provider` | `office-online`, `office-desktop` |
| `reference.application` | `word-online`, `excel-online`, `word-desktop`, `excel-desktop` |
| `reference.captureMethod` | `manual`, `semi-automated`, `automated` |
| `reference.applicationVersion` | необязательно: версия приложения, для desktop — `$word.Version`/`$excel.Version` из COM |
| `reference.exportedPdf` | обязателен для `mode = "pdf"` |

Пара проверяется, а не исправляется:

- семейство приложения совпадает с провайдером: `office-online` ⇒ `*-online`,
  `office-desktop` ⇒ `*-desktop`;
- приложение совпадает с форматом: `docx` ⇒ `word-*`, `xlsx` ⇒ `excel-*`.

Несовпадение — ошибка валидации manifest с путём `reference.application`:
`office-desktop` с `word-online` не станет ни тем, ни другим. Пару проверяют
и схема (ветки `allOf` по `provider` и по формату), и семантика валидатора.

Автоматический захват (`.ps1` на Windows, `.sh` на macOS) пишет
`office-desktop` + `word-desktop`/`excel-desktop` + `automated`. Ручной захват
в Word Online / Excel Online пишет `office-online` + `*-online` + `manual` или
`semi-automated`.

```bash
pnpm visual-oracle reference import \
  --from <drop>/docx/docx-tables-long-table/print-layout \
  --corpus test-fixtures/strict-viewer/corpus.json
```

Manifest пишется в `test-fixtures/strict-viewer/<format>/<fixture-id>/<mode>/`.
Существующий manifest перезаписывается только с `--force`; прежний
`baselineId` при этом печатается — он нужен в baseline-only PR (§5).

Импорт падает, если:

- `source.path` не совпадает с corpus index или фикстуры/режима там нет;
- `capture.json` снят против другой версии корпуса или матрицы;
- `reference.provider`/`reference.application` вне enum или пара не совпадает
  (провайдер с семейством приложения, приложение с форматом);
- PNG не имеет валидного IHDR или его color type не RGB8/RGBA8;
- итоговый manifest не проходит schema и семантические проверки §4.

## 4. Проверить корпус

```bash
pnpm visual-oracle validate --corpus test-fixtures/strict-viewer/corpus.json
```

Проверяются corpus index, каждый `manifest.json` под корнем эталонов и
соответствие двух множеств: лишний manifest — всегда ошибка, пропущенный —
ошибка только у принятого корпуса (`baselineId` задан). Черновой корпус
(`baselineId: null`) печатает

```text
visual-oracle: draft corpus, N fixture/mode pair(s) still have no manifest
```

и выходит с кодом 0.

## 5. Прогнать оракул

```bash
pnpm visual-oracle run \
  --corpus test-fixtures/strict-viewer/corpus.json \
  --out visual-oracle-out
```

Прогон снимает каждый выбранный fixture/mode собственным вьюером, сравнивает
кадры с эталоном и пишет `report.json` и `report.html` в `--out`.

| Флаг | Смысл |
|---|---|
| `--corpus <path>` | corpus index (обязателен) |
| `--out <dir>` | каталог отчёта и артефактов (обязателен) |
| `--references <dir>` | корень эталонов; по умолчанию `corpus.storage.rootUri` |
| `--filter <expr>` | `tag=<tag>` или `id=<glob>`; повторяемый, условия объединяются по «И» |
| `--override <expr>` | `<fixtureId|fixtureId/captureId>=<cause>`; повторяемый |
| `--port <number>` | порт dev-сервера (по умолчанию 5178) |
| `--headed` | показать окно браузера (только для отладки) |

Причины для `--override`: `parse`, `model`, `layout`, `font`, `render`,
`harness`, `reference`. Переопределение меняет только `cause` и
`causeSource: "override"`; `resultType` остаётся вычисленным по правилам —
иначе `--override` стал бы способом погасить расхождение.

Сессия захвата (dev-сервер и Chromium) поднимается лениво: если ни у одной
выбранной фикстуры нет manifest, прогон завершается за миллисекунды и не
трогает браузер. Сбой одной фикстуры не прерывает остальные.

### Коды выхода

| Код | Условие |
|---:|---|
| 0 | все captures имеют `match` |
| 1 | есть `visual-diff` или `page-mismatch` |
| 2 | есть `missing-reference` или `capture-error`, но нет кода 3 |
| 3 | есть `harness-error`, повреждённый эталон или невалидный manifest |

Порядок проверок — от худшего к лучшему: сломанный контур перекрывает
неполный захват, а тот — измеренное расхождение. Прогон с одним
`harness-error` и одним `visual-diff` не должен выглядеть как обычная
регрессия рендера.

## 6. Прочитать отчёт

`report.json` — машинный: `engineRevision`, `baselineId`, версии корпуса и
матрицы, статус каждого capture, пути reference/actual/diff, метрики и
диагностика. `report.html` показывает те же изображения и сортирует худшие
результаты, не меняя машинный статус.

Пути в отчёте относительны каталога `--out`, поэтому отчёт переносим вместе с
артефактами. Эталоны лежат вне `--out`, и ссылки на них идут через `../` —
это ожидаемо.

`runtime.stableHashes`/`timeToStableMs` строки описывают её захват, не фикстуру:
у `captureBackend: "canvas"` это хеши записанного кадра (`stableHashes[1]` =
`actual.sha256`), у `"pdf"` — кадр-«привратник» до экспорта, хеш страницы только
в `actual.sha256`. `runtime.json` рядом с PNG — значения фикстуры (последний
кадр для canvas), не постраничное доказательство: строка с чужими хешами
выдала бы `match` за доказанный.

## 7. Обновление baseline

Обновление — отдельный PR, не смешанный с правкой движка, компаратора или
матрицы (§5). Запрещены `--update-snapshots` и аналоги, копирование `actual/`
в `reference/`, автоматические маски по найденным отличиям, повышение допуска
ради зелёного результата и удаление страницы из корпуса ради устранения
failure.

## 8. CI

`.github/workflows/visual-oracle.yml` гоняет typecheck, тесты, сборку,
`validate` и `run` на каждый PR, затрагивающий просмотрщик, движок или
корпус. Пока `baselineId: null`, шаг `run` не гейтит джобу: `missing-reference`
— ожидаемое состояние доснимаемого корпуса. Как только baseline принят, тот же
шаг становится жёстким. Отчёт выгружается артефактом всегда, в том числе
после падения.
