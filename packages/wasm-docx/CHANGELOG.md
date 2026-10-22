# @doc-converter/wasm-docx

## 1.4.0

### Minor Changes

- 43ebe72: XLSX → PDF: `fitToPage`, `fitToWidth` и `fitToHeight`.

  - `exportPdf({ fitToPage: true })` укладывает лист на одну страницу;
    явные `fitToWidth`/`fitToHeight` сильнее флага, `0` — без ограничения.
  - CLI: `--fit-to-page`, `--fit-to-width <n>`, `--fit-to-height <n>`.
  - Семантика — как у Excel: fit-поля перебивают ручной `scale`;
    для DOCX/PPTX поля не действуют.

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

## 1.2.0

### Minor Changes

- Релиз v1.1.0: синхронный подъём версии с `@meerrec/core` — гейт
  `release.yml` требует единую версию npm-пятёрки и крейта
  `doc-converter-core`. Содержимое пакета: метаданные и README для
  публикации в npmjs.org, пересборка биндингов против
  doc-converter-core 1.1.0 (zip 9, wire-типы через ts-rs).

## 1.0.0

### Major Changes

- c9a8236: Первый стабильный релиз: просмотр XLSX и DOCX в OffscreenCanvas-воркере
  (транспорты SAB и bytes), экспорт XLSX/DOCX в PDF, конвертация DOCX в
  Markdown/HTML/текст, CLI `convert`/`batch` с пулом воркеров. Подробности —
  в CHANGELOG пакетов и `docs/sprint-12/`.
