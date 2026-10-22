---
"@meerrec/core": minor
---

DOCX: доступный TextLayer, bidi UAX #9 и флаг `a11y`.

- `createDocxViewer(host, { a11y: true })` включает прозрачный DOM-слой с
  нативным выделением и ARIA-деревом; по умолчанию выключен.
- Новые методы `getTextLayer(page)` и `getFullTextLayer()` отдают
  позиционированные runs с bbox, стилем, табличным контекстом и bidi-уровнями.
- `createXlsxViewer` принимает `a11y` для минимальной обвязки (`role=document`,
  canvas `aria-hidden`).
- Бюджет `wasm (docx)` поднят 410 → 420 КБ: UAX #9 и TextLayer (ADR-0040/0041).
