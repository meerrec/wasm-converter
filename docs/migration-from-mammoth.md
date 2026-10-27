# Миграция с mammoth.js на doc-converter

Для тех, кто конвертирует DOCX → HTML / Markdown / текст через `mammoth`.
Обе библиотеки делают одно и то же (разбор DOCX, экспорт в текст), но
doc-converter — Rust/WASM под капотом, с PDF-экспортом, просмотрщиком и
батч-CLI; у mammoth есть то, чего у нас нет (пользовательские трансформы
и стилевые карты) — подробности ниже.

## Сопоставление на бегу

| Действие | mammoth | doc-converter |
|---|---|---|
| DOCX → HTML | `mammoth.convertToHtml({buffer})` | CLI `convert x.docx -o y.html`; просмотрщик `viewer.exportHtml()`; программно `convertFile` |
| DOCX → текст | `mammoth.extractRawText({buffer})` | CLI `convert x.docx -o y.txt`; `viewer.exportText()` |
| DOCX → Markdown | `convertToMarkdown` — deprecated (см. ниже) | штатная цель: CLI `convert x.docx -o y.md`; `viewer.exportMarkdown()` |
| DOCX → PDF | — | CLI / просмотрщик `exportPdf()` (свой PDF-бэкенд, [ADR-0022](adr/0022-own-pdf-backend.md)) |
| Просмотр документа в браузере | — | `createDocxViewer` (рендер в Web Worker) |
| Пользовательские трансформы | `transformDocument` | нет |
| Стилевые карты | `styleMap` | нет — экспорт фиксированный |
| Встраивание картинок в HTML | да (base64 в HTML) | нет — `<img>` без данных |
| Сообщения/предупреждения | `result.messages` рядом с `value` | ошибки кидаются ([ADR-0016](adr/0016-docx-error-policy.md)) |

## Что мы умеем из того же

### `convertToHtml` → `exportHtml`

Тот же вход, тот же выход: байты `.docx` → HTML.

```js
// mammoth (Node; в браузере — { arrayBuffer })
const mammoth = require('mammoth');
const result = await mammoth.convertToHtml({ path: 'report.docx' });
console.log(result.value);      // HTML
console.log(result.messages);   // предупреждения рядом с результатом
```

```bash
# doc-converter: CLI
doc-converter convert report.docx -o report.html
```

В браузере — через просмотрщик:

```ts
const viewer = await createDocxViewer(host, { workerUrl, docxWasm });
await viewer.open(bytes);            // страницы документа
const html = await viewer.exportHtml();
```

Программно в Node — `convertFile` из `@meerrec/cli` (то же, что зовёт
bin, без старта процесса):

```ts
import { convertFile } from '@meerrec/cli';

await convertFile({
  input: 'report.docx',
  output: 'report.html',
  format: 'html',
  pdf: {},
});
```

### `extractRawText` → `exportText`

У mammoth — «сырой» текст без форматирования; у нас — текстовый экспорт
разложенного документа. CLI: `convert report.docx -o report.txt`,
в просмотрщике — `viewer.exportText()`.

### Markdown

У mammoth Markdown есть (`convertToMarkdown`), но в его же README он
объявлен **deprecated**: рекомендуется генерировать HTML и конвертировать
его сторонней библиотекой. У нас Markdown — штатная поддерживаемая цель
экспорта: CLI `convert report.docx -o report.md` или
`viewer.exportMarkdown()`.

### Остальное, чего у mammoth нет

- **PDF-экспорт** ([ADR-0022](adr/0022-own-pdf-backend.md)).
- **Просмотрщик в браузере**: рендер целиком в Web Worker, основной поток
  получает готовый холст (см. [README](../README.md) «Просмотрщик DOCX»).
- **Батч-конвертация**: `doc-converter batch ./in/*.docx --out ./html
  --format html --workers 8` — файлы, каталоги, glob-ы, пул воркеров.
- **Один движок на все форматы**: PDF/Markdown/HTML/текст делают одни и те
  же Rust-крейты, поэтому результаты согласованы между собой.

## Чего у нас нет из mammoth

- **Пользовательские трансформы** (`transformDocument`,
  `mammoth.transforms.paragraph(...)`): у нас экспорт фиксированный,
  подстроить маппинг документа в HTML нельзя.
- **Стилевые карты** (`styleMap`): маппинг стилей Word → HTML у нас
  встроенный, не переопределяется.
- **`embedStyleMap`**: вшивание стилевой карты в `.docx` — редактирование
  выхода, вне границ v1 ([ADR-0018](adr/0018-editing-boundaries.md)).
- **Картинки в HTML**: наш HTML отдаёт `<img>` без данных; mammoth вклеивает
  картинки в base64. Если потребителю HTML нужны именно встроенные картинки —
  это аргумент в пользу mammoth (и заодно причина разницы в скорости, см.
  ниже).
- **Канал `messages`**: mammoth возвращает предупреждения рядом с
  результатом; мы кидаем ошибки — зашифрованные DOCX и `.doc` у нас
  фатальная ошибка, а не «значение с сообщениями»
  ([ADR-0016](adr/0016-docx-error-policy.md)).

## Сопоставление API

| mammoth | doc-converter |
|---|---|
| `mammoth.convertToHtml({path}` \| `{buffer}` \| `{arrayBuffer})` | CLI `convert x.docx -o y.html`; `convertFile({ input, output, format: 'html', pdf: {} })`; `viewer.exportHtml()` |
| `mammoth.extractRawText(...)` | CLI `convert x.docx -o y.txt`; `viewer.exportText()` |
| `mammoth.convertToMarkdown(...)` (deprecated) | CLI `convert x.docx -o y.md`; `viewer.exportMarkdown()` |
| — | `viewer.exportPdf()`; CLI `convert x.docx -o y.pdf` |
| `options.transformDocument` | нет |
| `options.styleMap` | нет |
| `result.messages` | исключение (thiserror-ошибка) |
| нет батча | `batch ./in --out ./out --format html --workers 8` |
| нет просмотра | `createDocxViewer` |

## Производительность

Пара DOCX → HTML — единственная эквивалентная в нашем бенчмарке против
JS-библиотек: обе стороны дают HTML. Медиана **×5,3 в нашу пользу** по 8
сопоставимым файлам из 31, разброс ×3,7–6,8. Примеры (медианы по 5
повторам, меньше — быстрее):

| файл | наш CLI | mammoth | × |
|---|---|---|---|
| `wide_table.docx` (13 МиБ) | 225,3 мс | 835,1 мс | 3,7 |
| `mixed.docx` (12 МиБ) | 197,8 мс | 757,2 мс | 3,8 |
| `table_100_pages.docx` (42,7 КиБ) | 42,9 мс | 292,8 мс | 6,8 |
| `memory_50mib.docx` (57 МиБ) ⚠ | 28,2 мс | 997,0 мс | 35,3 ⚠ |

Источник: [docs/bench-js.md](bench-js.md) (прогон 2026-10-11; Apple M3 Pro,
Node v25.6.1, mammoth 1.13.0).

Оговорки, которые обязан знать читающий таблицу:

- ⚠ — файлы, где выходы расходятся больше чем в 2 раза, из сводного
  коэффициента исключены. На `memory_50mib.docx` mammoth отдаёт 78 МиБ HTML
  (картинки в base64) против наших 1,2 МиБ (`<img>` без данных), на
  `many_images.docx` — 20 МиБ против 13,5 КиБ: стороны делают разный объём
  работы. Честное число — ×5,3 по сопоставимым файлам.
- **Холодный старт.** У mammoth первый вызов 1151 мс против 1,6 мс в
  установившемся режиме (в первый вызов попали JIT и разбор крупного файла);
  у нас загрузка wasm-модуля ~70–80 мс и платится один раз. Наш `convert`
  как подпроцесс добавляет ~97 мс старта Node — в `batch` эта цена
  размазывается по файлам.
- **Корпус синтетический и мелкий**: медиана коммитнутого `.docx` — 3,3 КиБ,
  на таких файлах меряются накладные расходы вызова. Числа на 12–58 МиБ
  показательнее, но и они — сгенерированные документы, не реальные.
- p95 при 5 повторах — фактически максимум; условия замера не изолированы.

Абсолютные числа нашего движка: разбор DOCX 50 МБ — 356 мс, пагинация 150
страниц — 32,47 мс (источник: [README](../README.md) §«Производительность»).

## Ограничения

Переносится с mammoth на doc-converter, помни о границах (полный список —
[README](../README.md) §«Ограничения»):

- **Правка текста в рантайме есть в `main`, записи файла — нет**: текст run'а
  можно заменить (в опубликованных `2.0.2` правки ещё нет), но исходный файл не
  перезаписывается
  ([ADR-0043](adr/0043-runtime-text-edits.md),
  [ADR-0020](adr/0020-parser-writer-boundary.md)).
- **Сноски и комментарии** парсятся в модели, но в раскладку не попадают.
- **Обтекание текстом вокруг плавающих рисунков — нет**: рисунок вынимается
  из потока.
- **VML-фолбэки** (`mc:Fallback`) игнорируются — рисуется только `mc:Choice`
  ([ADR-0014](adr/0014-alternate-content.md)).
- **Strict OOXML** поддерживается наравне с Transitional
  ([ADR-0034](adr/0034-strict-ooxml.md)).
- **Макрофайлы `.docm`** поддерживаются как обычный DOCX, макросы не читаются
  ([ADR-0039](adr/0039-cancel-v1-non-goals.md)); зашифрованные DOCX и `.doc` —
  фатальная ошибка ([ADR-0016](adr/0016-docx-error-policy.md)).

## Когда остаться на mammoth

- Нужны **пользовательские трансформы или стилевые карты** — у нас экспорт
  фиксированный.
- Нужны **картинки, встроенные в HTML** (base64).
- Нужен **канал предупреждений** `messages` рядом с результатом, а не
  исключение.

Во всех остальных случаях — тот же вход, тот же выход, быстрее на
сопоставимых файлах, плюс PDF, просмотрщик и батч-конвертация в нагрузку.

## Ссылки

- [README](../README.md) — обзор, установка, просмотрщик DOCX, CLI.
- [docs/bench-js.md](bench-js.md) — бенчмарк против mammoth/exceljs/SheetJS,
  методика и оговорки.
- [docs/adr/](adr/) — принятые архитектурные решения (PDF-бэкенд — 0022,
  политика ошибок — 0016).
- `docs/cookbook/` — рецепты просмотра, экспорта и embed.
