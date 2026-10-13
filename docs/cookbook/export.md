# Рецепты: экспорт

Экспорт считается в воркере — там же, где лежит разобранная книга или
документ. PDF-путь идёт через ленивый модуль `@doc-converter/wasm-pdf`
(собственный бэкенд, [ADR-0022](../adr/0022-own-pdf-backend.md)), который
воркер грузит при первом экспорте.

---

### <a id="ex-01"></a>Рецепт 1. PNG текущего кадра

**Задача.** Скачать картинку того, что сейчас на экране.

```ts
const png: Uint8Array = await viewer.exportPng();

const blob = new Blob([png], { type: 'image/png' });
const url = URL.createObjectURL(blob);
const a = document.createElement('a');
a.href = url;
a.download = 'frame.png';
a.click();
URL.revokeObjectURL(url);
```

**Под капотом.** PNG — снимок текущего кадра, собранный в воркере; он же
используется в e2e-тестах просмотрщика, чтобы проверить, что нарисовалось
на самом деле.

---

### <a id="ex-02"></a>Рецепт 2. PDF одного листа

**Задача.** Экспортировать текущий (или конкретный) лист в PDF.

```ts
import type { PdfOptions } from '@doc-converter/core';

// Без настроек — весь текущий лист на A4.
const pdf = await viewer.exportPdf();

// Конкретный лист, независимо от того, что показано:
const opts: PdfOptions = { sheetIndex: 2 };
const pdfSheet2 = await viewer.exportPdf(opts);
```

**Под капотом.** Книга уже разобрана в воркере, поэтому PDF собирается без
повторного разбора; `sheetIndex` перекрывает показанный лист и выбирает
ровно один.

---

### <a id="ex-03"></a>Рецепт 3. PDF всей книги (allSheets)

**Задача.** Одна книга — один PDF со всеми листами.

```ts
const pdf = await viewer.exportPdf({ allSheets: true });
// Листы идут подряд, на каждый — закладка (PDF outline). sheetIndex в этом
// режиме не участвует: он выбирает один лист.
```

**Под капотом.** Режим `allSheets` договаривается с `sheetIndex` явно в
типе `PdfOptions`, чтобы не было двух взаимоисключающих просьб; страницы
листа считаются пейджингом XLSX (`docs/sprint-12/xlsx-pagination.md`).

---

### <a id="ex-04"></a>Рецепт 4. Параметры PDF: размер, ориентация, поля, масштаб

**Задача.** A3 альбомный с полями и уменьшенным масштабом.

```ts
import type { PdfOptions } from '@doc-converter/core';

const opts: PdfOptions = {
  pageSize: 'A3',      // 'A4' | 'A3' | 'Letter' | 'Legal'
  orientation: 'landscape',
  margins: { top: 15, right: 15, bottom: 15, left: 15 },
  scale: 0.8,          // масштаб содержимого
};
const pdf = await viewer.exportPdf(opts);
```

**Под капотом.** Эти поля читает XLSX-путь (лист разбивается на страницы
заданного размера); у DOCX они не влияют — страницы берутся из документа
вместе с размерами и полями `w:sectPr`.

---

### <a id="ex-05"></a>Рецепт 5. DOCX → PDF из просмотрщика

**Задача.** Экспортировать документ в PDF, не меняя его разбивку.

```ts
const pdf = await docxViewer.exportPdf();
// Без настроек — страницы раскладки со своими размерами и полями из w:sectPr.
// pageSize/orientation/margins/scale/sheetIndex не влияют (PdfOptions общий,
// но DOCX читает не все поля).
```

**Под капотом.** PDF рисует тот же движок, что и экран: страницы уже
разложены пагинатором, и PDF не пересчитывает раскладку, а печатает её —
поэтому число страниц PDF совпадает с просмотрщиком.

---

### <a id="ex-06"></a>Рецепт 6. Ленивый PDF-модуль: как отдать URL

**Задача.** Подключить `@doc-converter/wasm-pdf` так, чтобы страница не
платила за него до первого экспорта.

```ts
const viewer = await createXlsxViewer(host, {
  workerUrl,
  pdfWasm: {
    // JS-обвязка: воркер импортирует её динамически по первому exportPdf.
    module: new URL(
      '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
      import.meta.url,
    ).href,
    // Бинарь: сборщик копирует JS как есть и не кладёт рядом .wasm,
    // поэтому второй URL передаётся явно.
    binary: new URL(
      '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm',
      import.meta.url,
    ).href,
  },
});
await viewer.open(bytes);
const pdf = await viewer.exportPdf(); // модуль грузится только здесь
```

**Под капотом.** Оба URL приходят с main-потока, потому что воркер собран
бандлом и относительный путь из него ничего не значит; без `binary`
wasm-bindgen ищет `.wasm` рядом с модулем — а там его нет.

---

### <a id="ex-07"></a>Рецепт 7. DOCX → Markdown/HTML/текст

**Задача.** Выгрузить документ в три текстовых формата.

```ts
const md = await docxViewer.exportMarkdown(); // заголовки, списки, таблицы, ссылки
const html = await docxViewer.exportHtml();   // самостоятельная страница со встроенными стилями
const text = await docxViewer.exportText();   // содержимое без разметки

await navigator.clipboard.writeText(md); // например, в буфер
```

**Под капотом.** Все три выгрузки — функции ленивого DOCX-модуля
(`docx_export_markdown`/`docx_export_html`/`docx_export_text` в
`packages/wasm-docx/pkg/*.d.ts`), поэтому они не тянут PDF-модуль и не
требуют `pdfWasm`.

---

### <a id="ex-08"></a>Рецепт 8. Экспорт без pdfWasm: внятная ошибка

**Задача.** Не оставить кнопку «PDF» молча сломанной, если URL модуля не задан.

```ts
const viewer = await createXlsxViewer(host, { workerUrl }); // pdfWasm не задан
await viewer.open(bytes);

try {
  await viewer.exportPdf();
} catch (error) {
  // Воркер отвечает внятной ошибкой, а не пустым файлом:
  console.error(error);
  statusLabel.textContent = 'PDF-экспорт недоступен: передайте pdfWasm';
}
```

**Под капотом.** Ленивость — выбор хозяина страницы: воркер не знает, где
лежит модуль, пока URL не передали с main-потока; контракт —
`PdfModuleUrls { module, binary? }` в `packages/core/src/protocol.ts`.

---

Ссылки: [ADR-0022](../adr/0022-own-pdf-backend.md) — собственный PDF-бэкенд
(lopdf, subsetter, skrifa), бюджеты — `docs/sprint-12/budgets.md`.
