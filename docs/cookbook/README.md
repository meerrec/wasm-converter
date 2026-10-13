# Cookbook: рецепты по API doc-converter

Сборник рабочих примеров для v1.0.0. Каждый рецепт — **задача**, **код на
TypeScript** (сверен с публичным API `@doc-converter/core`,
`@doc-converter/cli` и биндингами wasm-пакетов) и одно предложение
**«под капотом»** — почему это работает именно так и где искать подробности.

## Домены

| Страница | Содержимое |
|---|---|
| [xlsx.md](xlsx.md) | Просмотр XLSX: открытие книги, листы, зум, hitTest, гиперссылки, тема, DPR |
| [docx.md](docx.md) | Просмотр DOCX: открытие, страницы, зум и зазор, изображения, колонтитулы, колонки |
| [transports.md](transports.md) | Транспорты кадра: `sab` / `bytes` / `auto`, COOP/COEP, embed без SAB |
| [export.md](export.md) | Экспорт: PNG, PDF, ленивый PDF-модуль, Markdown/HTML/текст |
| [cli.md](cli.md) | CLI: `convert`, `batch`, опции PDF, коллизии имён |
| [integration.md](integration.md) | Интеграция: Vite, WorkerPool и convertFile в Node, ошибки инициализации |

## Числа, на которые опираются рецепты

Свежие замеры Спринта 12 — [../sprint-12/budgets.md](../sprint-12/budgets.md),
таблица размеров ассетов в [README](../../README.md):

| Ассет | Факт (gzip) | Бюджет |
|---|---|---|
| `@doc-converter/core` worker.js / index.js | 7,17 / 3,34 КБ | 550 / 200 КБ |
| `wasm (viewer)` — XLSX-просмотрщик | 464,2 КБ | 480 КБ |
| `wasm (docx)` — ленивый модуль DOCX | 384,4 КБ | 400 КБ |
| `wasm (pdf)` — ленивый модуль PDF | 1,01 МБ | 1,05 МБ |

Память: PDF-экспорт ≤ 40 МиБ (факт 28,41 МиБ на книге в 500 страниц), разбор
DOCX ≤ 200 МиБ (факт 155,48 МиБ на файле 57 МиБ с 75 картинками). Браузеры:
Chrome 90+, Firefox 96+, Safari 16.4+.

## Рецепты

Всего рецептов: **46**.

| # | Рецепт | Файл |
|---|---|---|
| 1 | Минимальное открытие книги | [xlsx.md](xlsx.md#xlsx-01) |
| 2 | Листы книги: переключение и размеры | [xlsx.md](xlsx.md#xlsx-02) |
| 3 | Зум и прокрутка | [xlsx.md](xlsx.md#xlsx-03) |
| 4 | hitTest: ячейка под курсором | [xlsx.md](xlsx.md#xlsx-04) |
| 5 | Гиперссылки: наведение и клик | [xlsx.md](xlsx.md#xlsx-05) |
| 6 | Сетка, заголовки и тёмная тема | [xlsx.md](xlsx.md#xlsx-06) |
| 7 | Метрики кадров через onTick | [xlsx.md](xlsx.md#xlsx-07) |
| 8 | DPR и ретина: что считается автоматически | [xlsx.md](xlsx.md#xlsx-08) |
| 9 | Минимальное открытие документа | [docx.md](docx.md#docx-01) |
| 10 | Навигация по страницам | [docx.md](docx.md#docx-02) |
| 11 | Зум и зазор между страницами | [docx.md](docx.md#docx-03) |
| 12 | Изображения документа | [docx.md](docx.md#docx-04) |
| 13 | Колонтитулы и нумерация PAGE/NUMPAGES | [docx.md](docx.md#docx-05) |
| 14 | Многоколоночная раскладка | [docx.md](docx.md#docx-06) |
| 15 | Большие документы: время и память | [docx.md](docx.md#docx-07) |
| 16 | Выбор транспорта: `auto` и `viewer.transport` | [transports.md](transports.md#tr-01) |
| 17 | Явный `sab` без заголовков — ошибка инициализации | [transports.md](transports.md#tr-02) |
| 18 | Явный `bytes`: гарантированно без SAB | [transports.md](transports.md#tr-03) |
| 19 | COOP/COEP: как отдать заголовки | [transports.md](transports.md#tr-04) |
| 20 | Embed без SAB: iframe, расширение, GitHub Pages | [transports.md](transports.md#tr-05) |
| 21 | Что даёт SAB: ring и отбрасывание кадров | [transports.md](transports.md#tr-06) |
| 22 | Диагностика dropped-кадров | [transports.md](transports.md#tr-07) |
| 23 | PNG текущего кадра | [export.md](export.md#ex-01) |
| 24 | PDF одного листа | [export.md](export.md#ex-02) |
| 25 | PDF всей книги (allSheets) | [export.md](export.md#ex-03) |
| 26 | Параметры PDF: размер, ориентация, поля, масштаб | [export.md](export.md#ex-04) |
| 27 | DOCX → PDF из просмотрщика | [export.md](export.md#ex-05) |
| 28 | Ленивый PDF-модуль: как отдать URL | [export.md](export.md#ex-06) |
| 29 | DOCX → Markdown/HTML/текст | [export.md](export.md#ex-07) |
| 30 | Экспорт без pdfWasm: внятная ошибка | [export.md](export.md#ex-08) |
| 31 | convert: DOCX → PDF | [cli.md](cli.md#cli-01) |
| 32 | convert: DOCX → Markdown/HTML/текст | [cli.md](cli.md#cli-02) |
| 33 | convert: XLSX → PDF и выбор листа | [cli.md](cli.md#cli-03) |
| 34 | Опции PDF в CLI | [cli.md](cli.md#cli-04) |
| 35 | batch: glob и пул воркеров | [cli.md](cli.md#cli-05) |
| 36 | batch: каталог рекурсивно | [cli.md](cli.md#cli-06) |
| 37 | Коллизии имён в batch | [cli.md](cli.md#cli-07) |
| 38 | Выходные коды и диагностика CLI | [cli.md](cli.md#cli-08) |
| 39 | Vite: воркер через `?worker&url` | [integration.md](integration.md#in-01) |
| 40 | Vite: COOP/COEP на dev-сервере | [integration.md](integration.md#in-02) |
| 41 | WorkerPool в Node | [integration.md](integration.md#in-03) |
| 42 | convertFile в Node без пула | [integration.md](integration.md#in-04) |
| 43 | Явный `sab` без заголовков: ловим ошибку и объясняем пользователю | [integration.md](integration.md#in-05) |
| 44 | supportsOffscreen и проверка окружения | [integration.md](integration.md#in-06) |
| 45 | Отдельный воркер на каждый вьюер | [integration.md](integration.md#in-07) |
| 46 | Кэш wasm-модулей blob-URL-ами | [integration.md](integration.md#in-08) |

## Источники фактов

- Публичный API: `packages/core/src/index.ts`, `packages/core/src/render/{xlsx_viewer,docx_viewer,offscreen}.ts`, `packages/core/src/protocol.ts`.
- CLI: `packages/cli/src/args.ts` (USAGE), `packages/cli/src/{convert,pool,run}.ts`.
- Биндинги: `.d.ts` в `packages/wasm/pkg`, `packages/wasm-docx/pkg`, `packages/wasm-pdf/pkg`.
- Транспорты: [README](../../README.md) «Встраивание: COOP/COEP», `docs/workers.md`, ADR [0021](../adr/0021-frame-transports.md) и [0004](../adr/0004-sab-acceleration.md).
- Числа: `docs/sprint-12/budgets.md`, `docs/sprint-11/report.md`, [ROADMAP.md](../../ROADMAP.md) §9.
