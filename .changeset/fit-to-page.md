---
"@meerrec/core": minor
"@meerrec/wasm": minor
"@meerrec/wasm-docx": minor
"@meerrec/wasm-pdf": minor
"@meerrec/cli": minor
---

XLSX → PDF: `fitToPage`, `fitToWidth` и `fitToHeight`.

- `exportPdf({ fitToPage: true })` укладывает лист на одну страницу;
  явные `fitToWidth`/`fitToHeight` сильнее флага, `0` — без ограничения.
- CLI: `--fit-to-page`, `--fit-to-width <n>`, `--fit-to-height <n>`.
- Семантика — как у Excel: fit-поля перебивают ручной `scale`;
  для DOCX/PPTX поля не действуют.
