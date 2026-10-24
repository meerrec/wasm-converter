# @meerrec/vue

## 2.0.2

### Patch Changes

- внутренние зависимости больше не публикуются как workspace:* — npm install ставит пакеты без ошибки “Unsupported URL Type”.

## 2.0.1

### Patch Changes

- исправлена запись кадра в SAB-слот: copy_from требовал равенства длин и
  паниковал на кадре меньше слота (теперь через subarray).

## 2.0.0

### Major Changes

- Мажорный релиз 2.0.0 — lockstep для всех десяти пакетов.
- Дедупликация: общие примитивы core (`ns`/`xml`/`units`) вынесены в крейт
  `doc-converter-wasm-common`, общий `WorkerCore`/`viewer_core` в TypeScript;
  удалён мёртвый код.
- Оптимизации: кэши раскладки и форматов XLSX (5,63 → 3,98 мс на условном
  форматировании), SAB-транспорт — три → две копии кадра, painter без
  аллокаций на команду, PDF быстрее за счёт владения контентом (6,08 → 5,03
  мс), кэш блобов диаграмм PPTX.

## 1.4.3

### Patch Changes

- 49c896c: DOCX: подсветка текста и высокий подвал, который больше не накрывает тело.

  - Подсветка (`w:highlight` и заливка знака `w:rPr/w:shd`) рисуется фоном под
    текстом — в просмотрщике и в PDF. Палитра `ST_HighlightColor` — шестнадцать
    цветов Word; заливка знака берёт цвет из `w:fill`.
  - Нижний колонтитул выше нижнего поля поднимает низ полосы набора: раньше
    таблица в конце страницы печаталась поверх подписи в подвале.

- Updated dependencies [49c896c]
  - @meerrec/core@1.4.3

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
  - @meerrec/core@1.4.2

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
  - @meerrec/core@1.4.1

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
  - @meerrec/core@1.4.0

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
  - @meerrec/core@1.3.0

## 1.2.0

### Minor Changes

- 16b9b1c: Обёртки фреймворков, телеметрия и рантаймы CLI

  - Новые пакеты `@meerrec/react`, `@meerrec/vue` и `@meerrec/svelte`:
    компоненты `XlsxViewer` и хуки/composable/store поверх `@meerrec/core`.
  - `@meerrec/core` — телеметрия на OpenTelemetry (ADR-0037) и обновлённые
    просмотрщики DOCX/XLSX.
  - `@meerrec/cli` — рантаймы Deno и Bun, общий пул воркеров.

- Первая публикуемая версия: composable `useXlsxViewer` и компонент `XlsxViewer`
  поверх `@meerrec/core`. Версия 1.1.0 — lockstep с npm-пятёркой и крейтами.

### Patch Changes

- Updated dependencies [16b9b1c]
  - @meerrec/core@1.2.0
