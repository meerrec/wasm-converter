---
'@meerrec/core': minor
---

XLSX: выделение ячеек и диапазонов как в Excel — мышь, клавиатура, полные строки,
столбцы и весь лист до `XFD1048576`, несмежные диапазоны, закрепления, объединённые
и скрытые ячейки. Добавлены `getSelection`/`setSelection`/`restoreSelection`,
`clearSelection`, `scrollCellIntoView` и `onSelectionChange`. Полный лист использует
виртуальную прокрутку, подсветка рисуется тем же DisplayList в worker/main режимах;
выделение не меняет книгу и не попадает в PDF (ADR-0055).
