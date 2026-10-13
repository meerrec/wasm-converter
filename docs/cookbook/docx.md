# Рецепты: просмотр DOCX

Просмотрщик DOCX устроен как XLSX (холст над областью прокрутки), но страницы
идут стопкой по вертикали. Парсер и пагинация — в ленивом модуле
`@doc-converter/wasm-docx`, который воркер грузит при открытии документа.

---

### <a id="docx-01"></a>Рецепт 1. Минимальное открытие документа

**Задача.** Открыть `.docx` и получить страницы.

```ts
// docx_worker.ts — свой воркер для документов, одна строка
import '@doc-converter/core/worker';

// main
import { createDocxViewer, type PageInfo } from '@doc-converter/core';
import workerUrl from './docx_worker.ts?worker&url'; // Vite

const viewer = await createDocxViewer(document.getElementById('viewer')!, {
  workerUrl,
  // Без docxWasm открытие вернёт внятную ошибку вместо страниц.
  docxWasm: {
    module: new URL(
      '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm.js',
      import.meta.url,
    ).href,
  },
});

const pages: PageInfo[] = await viewer.open(await file.arrayBuffer());
console.log(`страниц: ${viewer.pageCount}`);

viewer.destroy();
```

**Под капотом.** Парсер, пагинация и сборка стопки живут в ленивом
wasm-модуле, который воркер импортирует при первом `open` — поэтому DOCX
надо явно дать `docxWasm`, а байты модуля не платят за себя на страницах с
XLSX.

---

### <a id="docx-02"></a>Рецепт 2. Навигация по страницам

**Задача.** Поле «страница», переход по номеру, кнопки «вверх/вниз».

```ts
const viewer = await createDocxViewer(host, { workerUrl, docxWasm });
await viewer.open(bytes);

pageField.textContent = `${viewer.pageIndex + 1} / ${viewer.pageCount}`;

jumpButton.addEventListener('click', () => {
  // showPage прокручивает на верх страницы по номеру (ноль-базированно).
  viewer.showPage(Number(pageInput.value) - 1);
});

nextButton.addEventListener('click', () => {
  if (viewer.pageIndex < viewer.pageCount - 1) viewer.showPage(viewer.pageIndex + 1);
});

// Размеры страниц — в пикселях раскладки, до зума.
const { width, height } = viewer.pages[0]!;
```

**Под капотом.** `pageIndex` — не «последняя нарисованная», а страница,
занимающая верх области прокрутки, поэтому его можно читать в скролле и
сразу показывать в интерфейсе.

---

### <a id="docx-03"></a>Рецепт 3. Зум и зазор между страницами

**Задача.** Зум и регулировка белой полосы между страницами стопки.

```ts
const viewer = await createDocxViewer(host, {
  workerUrl,
  docxWasm,
  zoom: 1.25,    // начальный зум
  pageGap: 16,   // зазор в пикселях раскладки; значение по умолчанию — 16
});

await viewer.open(bytes);

viewer.setZoom(1); // обратно к ста процентам
```

**Под капотом.** `pageGap` уходит в кадр и по нему Rust ставит страницы друг
под другом — стопка в кадре обязана совпасть с распоркой в разметке, иначе
картинка разъедется с прокруткой, поэтому зазор задаётся опцией, а не CSS.

---

### <a id="docx-04"></a>Рецепт 4. Изображения документа

**Задача.** Убедиться, что картинки документа доезжают до экрана (и понять,
как они вообще попадают в кадр).

```ts
// На уровне просмотрщика ничего делать не надо: изображения рисуются внутри
// кадра автоматически. Страницы просто открываются:
const viewer = await createDocxViewer(host, { workerUrl, docxWasm });
await viewer.open(bytes); // картинки уже в стопке страниц
```

**Под капотом.** Байты медиа в список страниц не входят: воркер получает
`docx_images()`/`docx_image_bytes(id)` и регистрирует `ImageBitmap` до
первого кадра, чтобы не копировать разом всю медиа; плейсхолдер остаётся
только там, где байтов нет ([отчёт Спринта 11](../sprint-11/report.md)).

---

### <a id="docx-05"></a>Рецепт 5. Колонтитулы и нумерация PAGE/NUMPAGES

**Задача.** Понять, что колонтитулы уже работают — и чего они стоят.

```ts
// Колонтитулы рисуются рендером сами: выбор идёт по w:titlePg и
// w:evenAndOddHeaders с откатом к обычному, поля PAGE/NUMPAGES разворачиваются
// двухпроходной раскладкой. Со стороны API ничего включать не нужно:
const viewer = await createDocxViewer(host, { workerUrl, docxWasm });
await viewer.open(bytes);

const png = await viewer.exportPng(); // колонтитул виден в снимке кадра
const pdf = await viewer.exportPdf(); // и попадает в PDF (нужен pdfWasm)
```

**Под капотом.** Номера страниц честно считаются, а не берутся из кэша Word:
цена — 39,8 мс против 20,7 мс без `NUMPAGES` на 100-страничной фикстуре
при бюджете пагинации 500 мс ([ROADMAP](../../ROADMAP.md) §9).

---

### <a id="docx-06"></a>Рецепт 6. Многоколоночная раскладка

**Задача.** Проверить документ с колонками — и знать, что он отрисуется.

```ts
// w:cols с равными и неравными ширинами, w:br type="column", переполнение
// колонки перетекает в соседнюю, новая страница начинается с первой колонки —
// всё это раскладка делает сама, API не участвует.
const viewer = await createDocxViewer(host, { workerUrl, docxWasm });
await viewer.open(bytes);
const png = await viewer.exportPng(); // скриншот текущего кадра для проверки
```

**Под капотом.** Многоколоночность — последний снятый маркер
`TODO (Спринт 10)` в пагинаторе (Спринт 11), поэтому она идёт тем же путём,
что обычный текст: раскладка в воркере, кадр — в DisplayList.

---

### <a id="docx-07"></a>Рецепт 7. Большие документы: время и память

**Задача.** Оценить, потянет ли вкладка 50-мегабайтный документ.

```ts
const viewer = await createDocxViewer(host, { workerUrl, docxWasm });
const started = performance.now();
await viewer.open(bytes); // 50 МБ DOCX: ~356 мс разбор (Apple Silicon, release)
console.log(`open: ${(performance.now() - started).toFixed(0)} мс`);
```

**Под капотом.** Ориентиры Спринта 12: разбор 50 МБ — 356 мс, пагинация
150 страниц — 28,18 мс, пик памяти на файле 57 МиБ с 75 картинками —
155,48 МиБ при бюджете 200 МиБ ([budgets.md](../sprint-12/budgets.md)).

---

Ссылки: пагинация и текст — [ADR-0006](../adr/0006-docx-layout-and-text-wrapping.md),
политика ошибок DOCX — [ADR-0016](../adr/0016-docx-error-policy.md).
