# Эталоны strict-viewer из локального Microsoft Office

Скрипт: `scripts/strict-viewer-references.ps1` (Windows, COM Word/Excel) или
`scripts/strict-viewer-references-macos.sh` (macOS, Word/Excel через AppleScript;
одинаковые флаги, drop-ы и отчёт). Отчёт прогона: `<Out>/report.md`,
drop-ы — `<Out>/drop/`.

## Зачем

Корпус строгого просмотра (`test-fixtures/strict-viewer/corpus.json`) —
единственный источник истины о том, что должно быть на экране/странице.
Собственно эталонов (`reference/*.png` + `manifest.json`) в репозитории пока
нет: их снимает эта страница. Харнесс снимает
**кандидатов** в эталоны из документов корпуса автоматически, без
ручного шага: локальный Microsoft Word/Excel через COM экспортирует документ
в PDF, PDF растрируется `pdftoppm`, результат собирается в операторский drop
и импортируется в manifest командой `pnpm visual-oracle reference import`.

Способ захвата разрешён решением `accept-capture-automation`
([ADR-0056](../adr/0056-capture-automation-com.md)): с v1.1.0
[`baseline-policy.md`](../strict-viewer/baseline-policy.md) §1 и §3 п.5 эталон
снимается автоматически **либо** вручную в Office Online. Скрипт не заменяет
`docs/strict-viewer/reference-capture.md` (операторская инструкция для ручного
пути) и не принимает baseline: он готовит кандидатов, которые затем
проверяются и принимаются по политике.

> **Что скрипт не делает.** Принятие baseline не изменилось:
> reference отсюда — **кандидат, а не accepted baseline**. Принятие требует
> baseline-only PR, двух approvals и независимого повторного захвата вторым
> исполнителем (`baseline-policy.md` §3 п.10–11, §5).

## Что снимается и что нет

| Формат | Режим | Как | Результат |
|---|---|---|---|
| `docx` | `pdf` | Word: `Documents.Open(…, ReadOnly)` → `ExportAsFixedFormat(…, 17)` | `reference/page-N.png` + `reference/export.pdf` |
| `xlsx` | `print-preview` | Excel: `Workbooks.Open(…, ReadOnly)` → печатные страницы листа 0 | `reference/print-page-N.png` + `reference/export.pdf` |
| `docx` | `print-layout` | снимается с экрана Word Online | **Skip** (см. «Ограничения») |
| `xlsx` | `normal` | снимается с экрана Excel Online | **Skip** (см. «Ограничения») |

Режимы `print-layout` и `normal` — экранные: их нельзя получить из PDF-экспорта,
поэтому скрипт их пропускает с явной причиной в отчёте. Флаг
`-IncludeWebModes` меняет формулировку причины на «требует ручного захвата,
не реализовано»; снимать их скрипт всё равно не умеет. Эквивалент не
выдумывается.

Обращений к Office Online нет: ни URL, ни сетевых вызовов, ни учётных данных.

## Требования

- Windows 10/11: COM-автоматизация.
- Установленные Microsoft Word и Excel (`Word.Application`,
  `Excel.Application`). **Перед запуском закройте Word и Excel**: скрипт
  создаёт COM-экземпляры и завершает их через `Quit()` в `finally`, чтобы не
  закрыть пользовательские документы; COM-объекты освобождаются через
  `[Runtime.InteropServices.Marshal]::ReleaseComObject`.
- Poppler (`pdftoppm`) в `PATH`.
- Node.js 20+ и pnpm 9 в `PATH`: импорт manifest идёт через
  `tools/visual-oracle`.
- PowerShell 5.1+. Нативные вызовы (`pdftoppm`, `pnpm`) скрипт делает сам через
  `System.Diagnostics.Process` с файловыми потоками: `2>&1` в PowerShell 5.1
  превращает stderr в записи ошибок, а `pdftoppm -v` пишет баннер в stderr даже
  на успехе. Импорт идёт через `cmd.exe` — pnpm на Windows это `.cmd`-shim.
- Id фикстур и `source.path` из корпуса скрипт проверяет сам: по ним строятся
  каталоги под `-Out`, которые он же удаляет перед повторным захватом. Слаг
  (буквы, цифры, `.`, `_`, `-`) и относительный путь без `..` — обязательны;
  иначе скрипт выходит с кодом 2, ничего не удаляя.
- Шрифтовой набор `crates/render/src/fonts` (четыре Carlito-сабсета, ADR-0051):
  его файлы и SHA-256 попадают в `environment.fonts.assets`.
- **Полные Carlito 1.104, установленные в системе** (ADR-0058). Сабсеты из
  репозитория подрезаны для рендера и для установки не годятся: без системы
  Word молча подменяет неизвестное семейство (наблюдалось
  `TimesNewRomanPSMT`), а с настоящим Calibri рисует Calibri. Установка —
  файлы из пришпиленного коммита `googlefonts/carlito` (`3a810cab…`), те же,
  что качает `scripts/subset-fonts.sh` (SHA-256 Regular `f6418f70…`):

  ```bash
  # macOS
  curl -fL "https://raw.githubusercontent.com/googlefonts/carlito/3a810cab78ebd6e2e4eed42af9e8453c4f9b850a/fonts/ttf/Carlito-Regular.ttf" \
    -o ~/Library/Fonts/Carlito-Regular.ttf
  ```

  Проверка обязательна до приёмки drop: PDF эталона обязан быть набран
  Carlito. macOS-скрипт проверяет это сам (`pdffonts`), Windows-скрипт —
  нет, поэтому там проверка ручная:

  ```bash
  pdffonts <drop>/docx/<fixture-id>/pdf/reference/export.pdf
  jq -r '.environment.fonts.package' <drop>/docx/<fixture-id>/pdf/manifest.json
  ```

В CI скрипт не входит: лицензии Microsoft Office и Windows-раннеры
недоступны, оракул ручной.

## Запуск

Из корня репозитория:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\strict-viewer-references.ps1 `
  -BrowserVersion 131.0.6778.86 `
  -Out strict-viewer-refs-out
```

Один документ с `-Filter` (glob по `id` фикстуры, синтаксис `-like`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\strict-viewer-references.ps1 `
  -BrowserVersion 131.0.6778.86 `
  -Filter 'docx-*' -KeepPdf
```

macOS — тот же захват через AppleScript (флаги в `--kebab-case`, смысл тот же):

```bash
scripts/strict-viewer-references-macos.sh \
  --browser-version 131.0.6778.86 \
  --filter 'docx-*' --keep-pdf
```

AppleScript-путь проверен на Word 16.115: документ адресуется по индексу,
созданному `open` (`active document` после `open` — `missing value`), `save as`
принимает только `POSIX file`, а `tell application` должен называть приложение
**литералом** — с переменной osascript не подхватывает словарь и `save as` не
компилируется (`-2741`). Холодный старт Word — до 30 с; ожидание задаётся
`--wait-timeout` (по умолчанию 120 с). Excel без установленного
`/Applications/Microsoft Excel.app` останавливает прогон кодом 2 до первого
захвата.

## Флаги

| Параметр | По умолчанию | Назначение |
| --- | --- | --- |
| `-Fixtures <dir>` | `test-fixtures` | Корень фикстур: относительно него разрешается `source.path` из корпуса |
| `-Corpus <path>` | `test-fixtures/strict-viewer/corpus.json` | Corpus index — единственный источник истины |
| `-Out <dir>` | `strict-viewer-refs-out` | Каталог drop-ов, логов и отчёта |
| `-Dpi <int>` | `144` | DPI растеризации. Не менять: схема manifest фиксирует `dpi: 144` |
| `-BrowserVersion <str>` | — | Версия Chromium для `environment.browser.version`; **обязателен**, если в отборе есть `docx.pdf` или `xlsx.print-preview` |
| `-FontPackage <str>` | `carlito-subset` | `environment.fonts.package` |
| `-FontVersion <str>` | `1.104` | `environment.fonts.version` |
| `-Operator <str>` | `$env:USERNAME`, затем имя машины | `reference.operator` |
| `-Filter <glob>` | — | Отбор фикстур по `id`; пустой отбор — ошибка употребления |
| `-KeepPdf` | выкл. | Не удалять `<Out>/work` с промежуточными PDF/PNG |
| `-IncludeWebModes` | выкл. | Пометить `print-layout`/`normal` как «требует ручного захвата» вместо обычного Skip |

Почему `-BrowserVersion` обязателен даже для PDF-режима: схема
`docs/strict-viewer/manifest.schema.json` объявляет `environment.browser`
обязательным для любого режима, а `name` в нём зафиксировано как `chromium`
с непустым `version`. Для `office-desktop` это **запись среды, а не вход
рендера**: PDF экспортирует локальный Word/Excel, браузер в кадре не
участвует, но значение записывается для воспроизводимости
(матрица §2.2: «браузер экспорта записывается»).

## Коды выхода

| Код | Условие |
| ---: | --- |
| `0` | Прогон состоялся: каждый режим либо `imported`, либо `skipped`; часть фикстур может быть skipped |
| `1` | Ни одного drop не импортировано |
| `2` | Нет предпосылок (не Windows, нет Office/COM, `pdftoppm`, Node/pnpm, корпуса, шрифтового набора, не задан `-BrowserVersion` при наличии PDF-режимов) или неверное употребление |

Системные утилиты и «нет предпосылок» сведены к `2`, как в
`scripts/oracle-msoffice.ps1` (`scripts/oracle-ssim.sh` в таком случае
выходит с кодом `3` — здесь этот код не используется).

Ошибка одной фикстуры (Office не открыл документ, pdftoppm упал, PNG не
прошёл проверку, импорт отказал) не роняет прогон: она попадает в отчёт со
статусом `failed` и причиной, остальные фикстуры продолжают сниматься.

## Что попадает в drop

```text
<Out>/drop/<format>/<fixture-id>/<mode>/
  capture.json                    # метаданные: корпус, источник, среда, режим, captures
  manifest.json                   # создаёт `pnpm visual-oracle reference import`
  reference/export.pdf            # исходный PDF эталона
  reference/page-N.png            # docx.pdf
  reference/print-page-N.png      # xlsx.print-preview
```

Последние три сегмента `--from` обязаны совпадать с
`<format>/<fixture-id>/<mode>` — это контракт импортёра, иначе manifest
уедет не в ту фикстуру.

`capture.json`:

- `corpus {version, matrixVersion}` — из corpus index; устаревшая версия
  ловится импортом.
- `source {path}` — ровно `fixtures[].source.path` из корпуса.
- `reference` — `provider: "office-desktop"`, `application: "word-desktop"`
  для DOCX / `"excel-desktop"` для XLSX, `applicationVersion` (из COM
  `Word.Version` / `Excel.Version`), `capturedAt` (UTC ISO 8601),
  `captureMethod: "automated"`, `operator`,
  `exportedPdf {uri: "reference/export.pdf"}`.
  - `provider: "office-online"` и `application: "*-online"` — ручной путь по
    `reference-capture.md`; `captureMethod` там `manual` или
    `semi-automated`. Семейство приложения обязано совпадать с провайдером,
    приложение — с форматом (`docx` ⇒ `word-*`, `xlsx` ⇒ `excel-*`);
    несовпадение — ошибка импорта, а не приведение к допустимому значению.
    Проверяет это схема manifest (`provider` и пара — её ветки `allOf`),
    а не отдельный список в импортёре; `schemaVersion` итогового manifest —
    `1.1.0`. Способ съёмки: `manual` (всё сделал человек),
    `semi-automated` (человек вёл сессию), `automated` (каждый шаг — скрипт).
- `environment` — `os`, `browser {name: "chromium", version}`,
  `applicationZoomPercent: 100`, `browserZoomPercent: 100`, `locale: "ru-RU"`,
  `timezone: "UTC"`, непустой `fonts {package, version, assets[{family,
  style, weight, sha256}]}`.
  - `docx.pdf`: плюс `rasterizer {name: "pdftoppm", version, dpi: 144}`;
    `viewportCss`/`deviceScaleFactor`/`colorScheme` запрещены схемой.
  - `xlsx.print-preview`: плюс `viewportCss {1440, 900}`,
    `deviceScaleFactor: 1`, `colorScheme: "light"`; `rasterizer` запрещён
    схемой и семантикой режима.
- `view` — `docx.pdf`: `{format, mode, revisions: "accepted", comments:
  "hidden"}`; `xlsx.print-preview`: `{format, mode, activeSheet {index,
  name}, hiddenSheets: "exclude", showGridLines: false, showHeadings: false}`.
- `dynamicData` — `dates: "fixed-value"`, `fields: "cached-result"`,
  `formulaCache {policy: "cached-values-only", dirty: false,
  fullCalcOnLoad: false, forceFullCalc: false}`, `externalLinks:
  "cached-values-no-network"`.
- `captures` — по одному на страницу, номера непрерывны от 1:
  - `docx.pdf`: `{id: "page-N", kind: "docx-page", pageNumber: N, image:
    "reference/page-N.png"}`;
  - `xlsx.print-preview`: `{id: "page-N", kind: "xlsx-print-page",
    pageNumber: N, sheet {index, name}, image:
    "reference/print-page-N.png"}`.

Импортёр сам считает `source.sha256`/`bytes`, `image.sha256`/`bytes`/
`width`/`height`/`colorType`, `reference.exportedPdf.sha256` и `baselineId`
— эти поля в `capture.json` руками не пишутся.

Перед сборкой drop каждая PNG проверяется той же валидацией, что и импорт:
сигнатура PNG и чанк `IHDR`, bit depth 8, colorType 2 (`RGB8`) или 6
(`RGBA8`), размер не больше 4 МиБ. Нарушение — ошибка фикстуры с полным путём
к файлу. Растеризация идёт без `-gray`: эталону нужен цвет.

Импорт вызывается с `--references <Out>/drop`: manifest пишется рядом с
`capture.json`, а `image.uri` разрешается внутри drop, поэтому drop
самодостаточен. Перенос drop в `test-fixtures/strict-viewer/<format>/<id>/<mode>/`
(LFS, `reference/*.png` и `*.pdf`) — отдельный baseline-only PR.

## Ограничения

- `docx.print-layout` и `xlsx.normal` не снимаются: это экранные режимы
  веб-версии, PDF-экспорт их не даёт. Скрипт их пропускает; эквивалент не
  выдумывается. Снимать их — вручную по `reference-capture.md`.
- Эталон снимает локальный Word/Excel, а не Word Online/Excel Online
  (оговорка о политике выше). Числа SSIM/пиксельного сравнения с
  Office Online не тождественны.
- `revisions: "accepted"`, `comments: "hidden"` достигаются экспортом без
  разметки правок (`Item = wdExportDocumentContent`): правки не «принимаются»
  физически, а просто не попадают в PDF.
- В `environment.fonts` записан версионированный набор Carlito из
  репозитория, а фактический рендер Word/Excel использует системные шрифты.
  Расхождение осознанно и должно быть учтено при проверке кандидата.
- XLSX снимается только с листа с индексом 0 (первый лист книги); скрытый
  лист — ошибка фикстуры, а не повод печатать его.
- `showGridLines: false` и `showHeadings: false` объявлены для
  `print-preview`; если в книге включена печать сетки или заголовков,
  объявление разойдётся с изображением — это ловит сравнение, а не скрипт.
- Повторный захват вторым исполнителем и проверка стабильности эталона
  (baseline-policy §3 п.10–11) скриптом не выполняются: это часть принятия,
  а не снятия кандидата.
- Колонтитулы и разметка Word могут быть отключены настройками Word
  пользователя; скрипт выставляет только `Visible = $false` и
  `DisplayAlerts`, остальные настройки приложения не трогает.

## См. также

- `docs/strict-viewer/baseline-policy.md` — хранение и принятие baseline.
- `docs/strict-viewer/reference-capture.md` — ручной захват и импорт.
- `docs/strict-viewer/manifest.schema.json` — форма manifest.
- `docs/testing/ms-office-oracle.md` — другой MS Office-харнесс (SSIM
  против нашего PDF).
