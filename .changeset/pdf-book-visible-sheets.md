---
"@meerrec/core": patch
"@meerrec/wasm": patch
"@meerrec/wasm-docx": patch
"@meerrec/wasm-pdf": patch
"@meerrec/wasm-pptx": patch
"@meerrec/wasm-pptx-pdf": patch
"@meerrec/cli": patch
"@meerrec/react": patch
"@meerrec/vue": patch
"@meerrec/svelte": patch
---

PDF-экспорт XLSX: «вся книга» печатает только видимые листы, без белых страниц.

- `exportPdf({ allSheets: true })` пропускает скрытые и очень скрытые листы —
  как Excel и LibreOffice. Явно выбранный через `sheetIndex` лист печатается
  независимо от состояния.
- Пустые ячейки с остатками форматирования (шрифт, формат числа) больше не
  создают страницу: раньше такие ячейки за концом данных давали полностью
  белые страницы. Формулы остаются содержимым листа, как и раньше.
