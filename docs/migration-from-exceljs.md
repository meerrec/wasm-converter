# Миграция с exceljs на doc-converter

Для тех, кто читает XLSX через exceljs. Честно: **это разные инструменты**.
exceljs даёт объектную модель ячеек наружу (прочитал значение, изменил,
записал); doc-converter книгу разбирает, раскладывает, рисует и
экспортирует в PDF — модели ячеек наружу у него нет. Если задача —
«достать значение ячейки», exceljs подходит лучше. Если задача — просмотр,
PDF/PNG или батч-конвертация — смотри ниже.

## Сопоставление на бегу

| Действие | exceljs | doc-converter |
|---|---|---|
| Прочитать значение ячейки | `workbook.xlsx.readFile(...)`; `sheet.getCell('C3').value` | нет — объектная модель наружу не отдаётся |
| Пройтись по строкам/ячейкам | `sheet.eachRow(...)`, `row.values` | нет |
| Редактировать книгу | да (чтение и запись) | нет — только чтение ([ADR-0018](adr/0018-editing-boundaries.md)) |
| Просмотр XLSX в браузере | — | `createXlsxViewer` (рендер в Web Worker) |
| XLSX → PDF | — | CLI `convert x.xlsx -o y.pdf`; `viewer.exportPdf()` (свой PDF-бэкенд, [ADR-0022](adr/0022-own-pdf-backend.md)) |
| XLSX → PNG | — | `viewer.exportPng()` |
| Батч-конвертация | — | `doc-converter batch ./in/*.xlsx --out ./pdf --workers 4` |

## Разная работа

### exceljs: объектная модель

```js
const ExcelJS = require('exceljs');
const workbook = new ExcelJS.Workbook();
await workbook.xlsx.readFile('book.xlsx');
const sheet = workbook.getWorksheet(1);
const cell = sheet.getCell('C3');
console.log(cell.value);
```

Это и есть основное применение exceljs: данные книги как структура данных
в памяти приложения, с записью обратно. Этого у doc-converter нет и в v1 не
планируется ([ADR-0018](adr/0018-editing-boundaries.md),
[ADR-0020](adr/0020-parser-writer-boundary.md)).

### doc-converter: просмотр и экспорт

У нас нет отдельной операции «прочитать книгу в модель» — ни в CLI, ни в
публичном API. Есть операции, которые разбор уже включают:

- **Просмотрщик в браузере**: разбор, раскладка листа в пикселях и отрисовка
  целиком в Web Worker, основной поток получает готовый холст
  ([README](../README.md) «Просмотрщик XLSX»).
- **Экспорт PDF** всей книги или одного листа (`--sheet`).
- **Экспорт PNG** из просмотрщика.
- **Батч-конвертация** с пулом воркеров.

CLI:

```bash
doc-converter convert book.xlsx -o book.pdf          # вся книга
doc-converter convert book.xlsx -o book.pdf --sheet 1  # один лист, 0-based
doc-converter convert book.xlsx -o book.pdf --scale 0.9 --page-size A4
doc-converter batch ./in/*.xlsx --out ./pdf --workers 4
```

Программно в Node — `convertFile` из `@meerrec/cli` (то же, что зовёт
bin, без старта процесса):

```ts
import { convertFile } from '@meerrec/cli';

await convertFile({
  input: 'book.xlsx',
  output: 'book.pdf',
  format: 'pdf',
  pdf: { sheetIndex: 0 },
});
```

Просмотрщик в браузере:

```ts
const viewer = await createXlsxViewer(host, { workerUrl, pdfWasm });
await viewer.open(bytes);        // листы книги
viewer.showSheet(1);
viewer.setZoom(1.25);
const pdf = await viewer.exportPdf();
const png = await viewer.exportPng();
```

Полезные методы просмотрщика: `hitTest`, `hyperlinkAt`, `setView` (сетка,
заголовки, тёмная тема), `onTick` (метрики кадров).

Форматы экспорта: XLSX → только PDF (в CLI) и PNG (в просмотрщике);
Markdown/HTML/текст у нас есть для DOCX, для XLSX — нет.

## Сопоставление API

| exceljs | doc-converter |
|---|---|
| `new ExcelJS.Workbook()`; `workbook.xlsx.readFile(path)` | нет аналога: разбор — внутренний шаг `viewer.open(bytes)` / `convertFile` |
| `workbook.getWorksheet(i)`; `sheet.getCell('C3').value` | нет аналога |
| `sheet.eachRow((row, n) => ...)` | нет аналога |
| `workbook.xlsx.writeFile(path)` | нет (только чтение) |
| — | `createXlsxViewer(host, options)`: `open`, `showSheet`, `setZoom`, `hitTest`, `exportPdf`, `exportPng` |
| — | CLI `convert x.xlsx -o y.pdf` (флаги `--sheet`, `--scale`, `--page-size`, `--orientation`, `--margins`) |
| — | CLI `batch ./in --out ./pdf --workers N` |

## Производительность

Наш бенчмарк против exceljs и SheetJS меряет **неэквивалентную пару**:
мы делаем разбор книги плюс раскладку, отрисовку, сериализацию PDF и запись
файла (десятки КиБ на выходе), а библиотеки только разбирают книгу в память.
Медиана «их время / наше время» — **×0,3** против exceljs и ×0,3 против
SheetJS: по этой паре мы медленнее, и это ожидаемо — наша операция строго
больше. Примеры (медианы по 5 повторам):

| файл | наш CLI (разбор + раскладка + PDF) | exceljs (только разбор в память) | × |
|---|---|---|---|
| `edge-many-rows.xlsx` (58,8 КиБ) | 37,5 мс | 20,7 мс | 0,6 |
| `size-50x50x1.xlsx` (13,2 КиБ) | 18,5 мс | 3,3 мс | 0,2 |
| `sheets-many.xlsx` (11,5 КиБ) | 1,2 мс | 2,1 мс | 1,7 |
| `comments-legacy.xlsx` (10,4 КиБ) | 1,0 мс | 1,9 мс | 1,9 |

Источник: [docs/bench-js.md](bench-js.md) (прогон 2026-10-11; Apple M3 Pro,
Node v25.6.1, exceljs 4.4.0).

Оговорки:

- Против exceljs мы медленнее на 17 из 24 файлов, на 7 мелких книгах полный
  наш экспорт уложился в то же время, что и чужой разбор; против SheetJS —
  на всех 24 (×0,1–0,6).
- **Сравнение некорректно** по объёму работы: наш полный экспорт в PDF
  укладывается в единицы-десятки миллисекунд на книгах, чей разбор занимает
  у SheetJS 1–15 мс — накладные расходы на раскладку и PDF сравнимы с самим
  разбором.
- Насколько быстр наш разбор XLSX **сам по себе**, из этих чисел не видно:
  операции «только разбор» у нас нет, и выделить её долю нечем.
- Корпус синтетический и мелкий (медиана коммитнутого `.xlsx` — 6,5 КиБ);
  p95 при 5 повторах — фактически максимум; условия замера не изолированы.

Абсолютные числа нашего движка: экспорт PDF книги на 500 страниц — 159,4 мс;
`batch` по 116 файлам, 4 воркера — 0,20 с (источник: [README](../README.md)
§«Производительность»).

## Когда остаться на exceljs

- Нужно **значение ячейки** или обход строк/столбцов как структуры данных.
- Нужна **запись**: собрать книгу, отредактировать существующую.
- Приложение само рисует данные — объектная модель нужна как модель, а не
  как картинка.

## Когда перейти на doc-converter

- Нужен **просмотр XLSX в браузере** без возни с рендером (разметку и холст
  собирает библиотека, рендер — в воркере).
- Нужен **PDF/PNG-экспорт** книг — exceljs этого не делает.
- Нужна **батч-конвертация**: пул воркеров, прогресс, 116 файлов за 0,20 с
  на 4 воркерах (README §«Производительность»).
- Не хочется тянуть JS-рантайм разбора в прод: разбор — Rust/WASM, один
  движок и для просмотра, и для экспорта.

## Ограничения

Границы, важные при миграции с exceljs (полный список — [README](../README.md)
§«Ограничения»):

- **Правка текста в рантайме есть, записи файла — нет**: значение
  ячейки можно заменить, но исходный
  файл не перезаписывается
  ([ADR-0043](adr/0043-runtime-text-edits.md),
  [ADR-0020](adr/0020-parser-writer-boundary.md)).
- **PPTX поддерживается** ([ADR-0033](adr/0033-pptx-plugin.md)); **XLSB и ODF**
  (`.ods`/`.odt`/`.odp`) — тоже ([ADR-0047](adr/0047-odf-plugin.md),
  [ADR-0048](adr/0048-xlsb-plugin.md)).
- **Strict OOXML** поддерживается наравне с Transitional
  ([ADR-0034](adr/0034-strict-ooxml.md)).

## Ссылки

- [README](../README.md) — обзор, просмотрщик XLSX, CLI, бюджеты.
- [docs/bench-js.md](bench-js.md) — бенчмарк против exceljs/SheetJS/mammoth,
  методика и оговорки.
- [docs/adr/](adr/) — принятые архитектурные решения (PDF-бэкенд — 0022,
  границы редактирования — 0018, 0020, 0043).
- `docs/cookbook/` — рецепты просмотра, экспорта и embed.
