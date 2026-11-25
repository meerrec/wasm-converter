# `@meerrec/visual-oracle`

Офлайн-оракул строгого просмотрщика: снимает кадры собственным вьюером,
сравнивает их с принятым эталоном Office Online и пишет машинный и
человеческий отчёты. Сеть и Office в сравнении не участвуют.

Политика хранения и принятия baseline — [`docs/strict-viewer/baseline-policy.md`](../../docs/strict-viewer/baseline-policy.md),
операторская инструкция — [`docs/strict-viewer/reference-capture.md`](../../docs/strict-viewer/reference-capture.md).

## Установка и запуск

Из корня репозитория:

```bash
pnpm install
pnpm build:wasm                 # вьюер типизируется против wasm-bindgen деклараций
pnpm visual-oracle --help
```

`pnpm visual-oracle` — это `tsx tools/visual-oracle/src/cli.ts`; собранный
бинарь (`dist/cli.js`) ставится как `visual-oracle` из `package.json`.

## Команды

```bash
# Проверить corpus index и все manifest под корнем эталонов
pnpm visual-oracle validate --corpus test-fixtures/strict-viewer/corpus.json

# Операторский drop → schema-valid manifest
pnpm visual-oracle reference import \
  --from <drop>/docx/docx-tables-long-table/print-layout \
  --corpus test-fixtures/strict-viewer/corpus.json

# Захват, сравнение и отчёт
pnpm visual-oracle run \
  --corpus test-fixtures/strict-viewer/corpus.json \
  --out visual-oracle-out \
  --filter id=docx-tables-long-table

# Кадры нашего вьюера по одному документу — без эталона и manifest
pnpm visual-oracle capture \
  --doc test-fixtures/docx/tables/long_table.docx \
  --out visual-oracle-out
```

`run` принимает `--filter tag=<tag>` / `--filter id=<glob>` (повторяемый,
условия объединяются по «И»), `--override <fixtureId|fixtureId/captureId>=<cause>`,
`--references <dir>`, `--port <number>` и `--headed`.

## Снять кадры без эталона

`run` работает только от manifest: если эталона ещё нет, фикстура получает
`missing-reference` (`no reference manifest at <path>`) и кадр не снимается
вовсе. Чтобы просто посмотреть, что рисует наш движок по конкретному файлу,
есть отдельная подкоманда `capture` — она не читает corpus index, не ищет
эталон и ничего не сравнивает.

```bash
pnpm visual-oracle capture --doc <path.docx|path.xlsx> --out <dir> [options]
```

| Флаг | Смысл |
|---|---|
| `--doc <path>` | Документ `.docx` или `.xlsx`; формат определяется по расширению (обязателен) |
| `--out <dir>` | Каталог вывода (обязателен) |
| `--mode <mode>` | Режим вьюера; по умолчанию `print-layout` для DOCX и `normal` для XLSX |
| `--port <number>` | Порт dev-сервера (по умолчанию 5178) |
| `--headed` | Показать окно браузера (только для отладки) |

Допустимые режимы: для DOCX — только `print-layout`, для XLSX — только
`normal`. `pdf` и `print-preview` здесь не поддерживаются: они требуют
экспорта PDF и растеризации, то есть другого контура, а не снимка окна.

Коды выхода:

| Код | Условие |
|---:|---|
| 0 | PNG записаны |
| 2 | ошибка аргументов, файла документа нет, режим не поддерживается |
| 3 | захват упал: браузер, таймаут или ошибка вьюера |

Раскладка вывода повторяет `run`:

```text
<out>/<slug>/<mode>/
  page-1.png        # DOCX: по файлу на страницу
  sheet-0.png       # XLSX: по файлу на лист
  runtime.json      # уровень фикстуры: ревизия движка, транспорт, viewport, DPR, зум, шрифты
```

`runtime.json` описывает фикстуру целиком (для canvas-режимов — последний
кадр), а не отдельную страницу. Постраничное доказательство стабильности
живёт в `report.json` на каждой строке: у `captureBackend: "canvas"` это хеши
записанного кадра, и `stableHashes[1]` равен `actual.sha256` строки; у
`captureBackend: "pdf"` — кадр-«привратник» до экспорта, а хеш самой страницы
есть только в `actual.sha256`.

`<slug>` — имя файла без расширения, нижний регистр, прогоны
не-алфавитно-цифровых символов → `-`: `long_table.docx` → `long-table`.

Число кадров `capture` узнаёт от самого вьюера, не открывая документ
дважды: снимается первый кадр (`page-1` / `sheet-0`), а `captureFixture`
попутно возвращает `structure` — по нему и добираются остальные страницы
(листы — по `viewer.sheets`). Отдельного прохода «сначала узнаем, сколько
страниц, потом снимаем» нет: он стоил бы ещё одного открытия документа.

**Это не эталон.** `capture` пишет только в `--out` и никогда — в
`test-fixtures/strict-viewer/**`. Политика прямо запрещает встраивать Office в
продукт и тестовый runtime ([`baseline-policy.md`](../../docs/strict-viewer/baseline-policy.md)
§3 п.5) и требует, чтобы в capture/diff runtime не было Office URL, сетевых
вызовов и учётных данных Office (§4). Reference PNG снимается вручную в Word
Online / Excel Online и принимается отдельным PR; кадр нашего движка эталоном
не становится никогда.

## Коды выхода

| Код | Условие |
|---:|---|
| 0 | все captures имеют `match` |
| 1 | есть `visual-diff` или `page-mismatch` |
| 2 | ошибка аргументов, `missing-reference` или `capture-error` |
| 3 | `harness-error`, schema/semantic error, повреждённый артефакт |

Код считается из отчёта чистой функцией `exitCodeFor` (`src/run.ts`) по
таблице baseline-policy §6: сломанный контур перекрывает неполный захват, а
тот — измеренное расхождение.

## Устройство

```text
src/
  cli.ts            разбор аргументов и четыре подкоманды
  corpus.ts         corpus index: загрузка, валидация, фильтры, slug
  manifest.ts       JSON Schema + семантические проверки §4
  reference/import.ts  операторский drop → manifest, baselineId
  capture/          dev-сервер, Playwright, стабильный кадр, растеризация PDF
  capture_only.ts   `capture`: синтетический manifest → PNG без эталона
  diff/             декод/кодек PNG, сравнение, классификация результата
  report/           report.json и report.html
  run.ts            прогон: эталон → захват → сравнение → отчёт
```

Слои разделены по вопросу, на который они отвечают:

- **corpus** — что вообще должно быть проверено;
- **manifest** — в какой среде и с какими артефактами снят эталон;
- **capture** — что нарисовал наш движок;
- **diff** — совпало ли это с эталоном и почему нет;
- **report** — что из этого следует для сертификации.

`capture` не бросает исключений через свою границу: сбой одной фикстуры
возвращается размеченным результатом, и прогон идёт дальше. Исключение в
`run` означает поломку контура.

Сессия захвата создаётся лениво. Корпус без единого эталона (типичное
состояние до приёмки baseline) отрабатывает за миллисекунды, не поднимая ни
dev-сервер, ни Chromium.

## Сравнение

Профиль `visual-v1` (матрица §2.3): одинаковые размеры обязательны, иначе
`page-mismatch`; вне масок сравниваются все пиксели RGBA с допуском
`max(|ΔR|, |ΔG|, |ΔB|) <= 2` и точным alpha; `match` требует нуля пикселей
вне допуска. SSIM считается как диагностика и не смягчает правило нуля.

Структурная проверка идёт до пикселей: число страниц документа (или печатных
страниц листа) против числа captures в manifest. Расхождение — `page-mismatch`,
а не повод сравнивать кадры.

`match` доказан, только если стабильность подтверждена для того же кадра:
`runtime` строки берётся из её собственного захвата, а не из `runtime.json`
фикстуры.

## Тесты

```bash
pnpm -F @meerrec/visual-oracle typecheck
pnpm -F @meerrec/visual-oracle test
pnpm -F @meerrec/visual-oracle build
```

Тесты покрывают чистые части: corpus, manifest, импорт, PNG, сравнение,
классификацию, отчёт, коды выхода и разбор `--override`. Сам захват требует
Chromium и проверяется сквозным прогоном, а не юнит-тестами.
