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

DOCX: подсветка текста и высокий подвал, который больше не накрывает тело.

- Подсветка (`w:highlight` и заливка знака `w:rPr/w:shd`) рисуется фоном под
  текстом — в просмотрщике и в PDF. Палитра `ST_HighlightColor` — шестнадцать
  цветов Word; заливка знака берёт цвет из `w:fill`.
- Нижний колонтитул выше нижнего поля поднимает низ полосы набора: раньше
  таблица в конце страницы печаталась поверх подписи в подвале.
