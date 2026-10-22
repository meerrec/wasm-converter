# @meerrec/wasm-pptx-pdf

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

- Первая публикуемая версия: ленивый модуль PDF-экспорта PPTX (`export_pptx_pdf`,
  ADR-0033; просмотрщик и экспортёр разнесены, как у DOCX). Версия 1.1.0 —
  lockstep.
