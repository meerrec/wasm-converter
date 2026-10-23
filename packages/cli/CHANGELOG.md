# @doc-converter/cli

## 1.4.3

### Patch Changes

- 49c896c: DOCX: подсветка текста и высокий подвал, который больше не накрывает тело.

  - Подсветка (`w:highlight` и заливка знака `w:rPr/w:shd`) рисуется фоном под
    текстом — в просмотрщике и в PDF. Палитра `ST_HighlightColor` — шестнадцать
    цветов Word; заливка знака берёт цвет из `w:fill`.
  - Нижний колонтитул выше нижнего поля поднимает низ полосы набора: раньше
    таблица в конце страницы печаталась поверх подписи в подвале.

- Updated dependencies [49c896c]
  - @meerrec/wasm-docx@1.4.3
  - @meerrec/wasm-pdf@1.4.3

## 1.4.2

### Patch Changes

- 58ef75e: PDF-экспорт XLSX: «вся книга» печатает только видимые листы, без белых страниц.

  - `exportPdf({ allSheets: true })` пропускает скрытые и очень скрытые листы —
    как Excel и LibreOffice. Явно выбранный через `sheetIndex` лист печатается
    независимо от состояния.
  - Пустые ячейки с остатками форматирования (шрифт, формат числа) больше не
    создают страницу: раньше такие ячейки за концом данных давали полностью
    белые страницы. Формулы остаются содержимым листа, как и раньше.

- Updated dependencies [58ef75e]
  - @meerrec/wasm-docx@1.4.2
  - @meerrec/wasm-pdf@1.4.2

## 1.4.1

### Patch Changes

- d8c59e7: PDF-экспорт XLSX: перенос текста ячейки больше не ломается масштабом печати.

  Вёрстка мерила текст уменьшенным кеглем, а ширину ячейки брала в
  немасштабированных пикселях раскладки. При `fitToWidth`/`fitToPage`, листовом
  `scale` и CLI `--scale` перенос решался по ширине в несколько раз больше
  реальной: строки не переносились и наезжали на соседние ячейки. Теперь перенос
  считается при 100 %, как в canvas-пути, а масштаб применяется к координатам и
  кеглю при записи PDF.

- Updated dependencies [d8c59e7]
  - @meerrec/wasm-docx@1.4.1
  - @meerrec/wasm-pdf@1.4.1

## 1.4.0

### Minor Changes

- 43ebe72: XLSX → PDF: `fitToPage`, `fitToWidth` и `fitToHeight`.

  - `exportPdf({ fitToPage: true })` укладывает лист на одну страницу;
    явные `fitToWidth`/`fitToHeight` сильнее флага, `0` — без ограничения.
  - CLI: `--fit-to-page`, `--fit-to-width <n>`, `--fit-to-height <n>`.
  - Семантика — как у Excel: fit-поля перебивают ручной `scale`;
    для DOCX/PPTX поля не действуют.

### Patch Changes

- Updated dependencies [43ebe72]
  - @meerrec/wasm-docx@1.4.0
  - @meerrec/wasm-pdf@1.4.0

## 1.3.0

### Minor Changes

- 6b6f38d: DOCX: доступный TextLayer, bidi UAX #9 и флаг `a11y`.

  - `createDocxViewer(host, { a11y: true })` включает прозрачный DOM-слой с
    нативным выделением и ARIA-деревом; по умолчанию выключен.
  - Новые методы `getTextLayer(page)` и `getFullTextLayer()` отдают
    позиционированные runs с bbox, стилем, табличным контекстом и bidi-уровнями.
  - `createXlsxViewer` принимает `a11y` для минимальной обвязки (`role=document`,
    canvas `aria-hidden`).
  - Бюджет `wasm (docx)` поднят 410 → 420 КБ: UAX #9 и TextLayer (ADR-0040/0041).

### Patch Changes

- Updated dependencies [6b6f38d]
  - @meerrec/wasm-docx@1.3.0
  - @meerrec/wasm-pdf@1.3.0

## 1.2.0

### Minor Changes

- 16b9b1c: Обёртки фреймворков, телеметрия и рантаймы CLI

  - Новые пакеты `@meerrec/react`, `@meerrec/vue` и `@meerrec/svelte`:
    компоненты `XlsxViewer` и хуки/composable/store поверх `@meerrec/core`.
  - `@meerrec/core` — телеметрия на OpenTelemetry (ADR-0037) и обновлённые
    просмотрщики DOCX/XLSX.
  - `@meerrec/cli` — рантаймы Deno и Bun, общий пул воркеров.

## 1.1.0

### Minor Changes

- Релиз v1.1.0: синхронный подъём версии с `@meerrec/core` — гейт
  `release.yml` требует единую версию npm-пятёрки и крейта
  `doc-converter-core`. Содержимое пакета: LICENSE, README и метаданные
  (repository/homepage/bugs) для публикации в npmjs.org.

### Patch Changes

- Updated dependencies
- Updated dependencies
  - @meerrec/wasm-docx@1.1.0
  - @meerrec/wasm-pdf@1.1.0

## 1.0.0

### Major Changes

- c9a8236: Первый стабильный релиз: просмотр XLSX и DOCX в OffscreenCanvas-воркере
  (транспорты SAB и bytes), экспорт XLSX/DOCX в PDF, конвертация DOCX в
  Markdown/HTML/текст, CLI `convert`/`batch` с пулом воркеров. Подробности —
  в CHANGELOG пакетов и `docs/sprint-12/`.

### Patch Changes

- Updated dependencies [c9a8236]
  - @doc-converter/wasm-docx@1.0.0
  - @doc-converter/wasm-pdf@1.0.0
