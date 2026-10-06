# Спринт 6 «Базовый PDF» — план и трекинг

- Ветка: `sprint/6-basic-pdf` · issue [#3](https://github.com/meerrec/wasm-converter/issues/3) · PR [#11](https://github.com/meerrec/wasm-converter/pull/11)
- Обновлено: 2026-10-06
- Прогресс: ✅ 23 · 🔄 2 · ⬜ 1 из 26
- Статусы: ⬜ не начато · 🔄 в работе · ✅ закрыто

**Цель:** экспорт XLSX-листа в PDF с корректной типографикой.

**DoD (ROADMAP §6):**

- PDF открывается в Chrome/Firefox/Adobe, `qpdf --check` проходит.
- Кириллица корректна: `pdftotext` возвращает исходный текст.
- 1000 ячеек → PDF < 200 КБ (с subsetting).
- 10 страниц < 300 мс.
- Playwright расширен на Firefox + WebKit.
- Точки переноса canvas и PDF совпадают.

## Решения

1. **Новый крейт `crates/layout-core` не создаётся.** Противоречит ADR-0003 и ROADMAP §6. PDF опирается на модель `xlsx` и `render::{FontRegistry, text_measure}` (`break_lines`, `measure_text`) — совпадение точек переноса получается по построению, а не обеспечивается синхронизацией двух реализаций. Фиксируется в ADR-0007.
2. **Потоковой записи PDF в спринте 6 нет.** printpdf 0.8.2 копит документ в памяти. Вместо streaming — замер RSS на 100-страничной фикстуре (B4) и фиксация ограничения в ADR-0008; streaming — Спринт 7.
3. **Шрифты.** К `crates/render/src/fonts/carlito-subset.ttf` (regular, 86,5 КБ, OFL) добавляются подрезанные Bold/Italic/BoldItalic — ~+270 КБ к WASM-бандлу (базовая линия до подключения PDF — 649 343 Б). Кириллица U+0400–U+04FF в regular покрыта (254 кодпоинта); U+2116 (№) отсутствует — проверить в полной версии Carlito и включить. Готово: № включён, набор — 465 кодпоинтов на начертание (C2, ADR-0009). Рост бандла `.size-limit.json` не контролирует (там только JS-бандлы), поэтому фиксируется замером.
4. **Минимальная вертикальная пагинация листа входит в спринт 6** — без неё не выполнить DoD «10 страниц < 300 мс». Полная пагинация (повтор заголовков, fit-to-width) — Спринт 7.
5. **E3 вне DoD: только capability detection с внятной ошибкой.** OffscreenCanvas в воркере есть в Chrome 69+/Firefox 105+/Safari 16.4+; fallback рендеринга на main thread — в бэклог.
6. **Бенчи — в `crates/pdf/benches/`** (корневой `benches/` пуст). Жёсткий ассерт ставится на размер PDF (детерминирован), время — бенч + порог с запасом.
7. **Трекер — этот файл.** В среде нет TodoWrite; статусы обновляются здесь.
8. **PDF-экспорт вынесен в отдельный лениво загружаемый wasm-модуль** — решение блокера B-1 (вариант 2). Крейт `crates/pdf-wasm` → `packages/wasm-pdf/pkg`; воркер грузит его динамическим `import()` по клику «Экспорт в PDF». Основной модуль вернулся к 649 522 Б raw / 275 521 Б gzip (-9) — в бюджете 300 КБ в `.size-limit.mjs`; PDF-модуль 4 166 980 Б raw / 1 756 515 Б gzip бюджета не имеет, платится по требованию. Ленивость доказана e2e `examples/viewer-xlsx/e2e/lazy-pdf.spec.ts`.

## Задачи

### A. Каркас и границы (предшествует остальным)

| ID | Задача | Артефакт | Статус |
| --- | --- | --- | --- |
| A1 | Аудит переиспользуемых слоёв раскладки и канвы | `docs/analysis/layout-audit.md`, commit bb3809c | ✅ |
| A2 | ADR-0007 «граница PDF-экспорта» (без отдельного крейта `layout-core`) | `docs/adr/0007-pdf-layout-boundary.md`, commit e9ef445 | ✅ |
| A3 | Скелеты модулей; `export_xlsx_sheet` отдаёт валидный PDF страницы | `crates/pdf/src/{styles,layout,fonts,text,border,background,painter,lib}.rs`, commit b934981 | ✅ |

### B. Рендерер и поток (∥ C)

| ID | Задача | Артефакт | Статус |
| --- | --- | --- | --- |
| B1 | Спайк по PDF-библиотеке | `docs/analysis/pdf-lib-spike.md`, commit 4b7f208 | ✅ |
| B2 | ADR-0008: printpdf, Identity-H/ToUnicode, непотоковость | `docs/adr/0008-pdf-library-and-encoding.md`, commit ebfb668 | ✅ |
| B3 | PdfRenderer: текст, линии, прямоугольники (ROADMAP §6) | `crates/pdf/src/painter.rs`, коммиты b934981, 7ab7a2d | ✅ |
| B4 | Замер памяти на 100-страничной фикстуре | протокол замера в ADR-0008 | 🔄 |
| B5 | RPC `exportPdf` → воркер → wasm; результат — transferable ArrayBuffer | `packages/core/src/worker/worker.ts`, коммиты c5b7402, eb5ffce | ✅ |
| B6 | Кнопка «Экспорт в PDF» + прогресс/ошибки в примере | `examples/viewer-xlsx/e2e/pdf.spec.ts`, commit f79e788 | ✅ |

### C. Шрифты и текст

| ID | Задача | Артефакт | Статус |
| --- | --- | --- | --- |
| C1 | Проверка: subsetting и Type0/Identity-H/ToUnicode уже есть в printpdf (`subsetter` 0.42 приходит транзитивно) | подтверждено спайком B1, закреплено в ADR-0009 | ✅ |
| C2 | Покрытие кириллицы: 254 кодпоинта блока U+0400–U+04FF; U+2116 (№) добавлен, набор — 465 кодпоинтов | `crates/render/src/fonts/`, commit 5bd5837 | ✅ |
| C3 | Тест `pdftotext <pdf> \| grep Привет` | CI (F3), commit 5411d00 | ✅ |
| C4 | Подрезанные Bold/Italic/BoldItalic | `crates/render/src/fonts/carlito-{bold,italic,bolditalic}-subset.ttf`, commit 5bd5837 | ✅ |
| C5 | ADR-0009: стратегия subsetting и кодирования | `docs/adr/0009-font-subsetting.md`, commit ebfb668 | ✅ |

### D. Производительность

| ID | Задача | Артефакт | Статус |
| --- | --- | --- | --- |
| D1 | Бенчи: `bench_pdf_size_1000_cells` < 200 КБ, `bench_pdf_time_10_pages` < 300 мс | `crates/pdf/benches/pdf.rs`, commit a0139a4 | ✅ |
| D2 | Гейт в тестах и CI | `crates/pdf/tests/budgets.rs`; `.size-limit.mjs` (300 КБ gzip на wasm), коммиты 5411d00, 01e9edb | ✅ |
| D3 | Профилирование | разбивка фаз `open`/`export`/`compress=false` в `benches/pdf.rs` | ✅ |
| D4 | Оптимизации (flate2 — уже в зависимостях pdf-крейта, кэш глифов) — по потребности | `crates/pdf/Cargo.toml` | ⬜ не потребовался: бюджеты сходятся |

### E. Браузеры

| ID | Задача | Артефакт | Статус |
| --- | --- | --- | --- |
| E1 | Проекты Firefox + WebKit (было только `chromium`, `chromium-dpr2`) | `examples/viewer-xlsx/playwright.config.ts`, commit e1117f6 | ✅ |
| E2 | Прогон расширенной матрицы | 22/22: chromium 7, firefox 7, webkit 7, chromium-dpr2 1 | ✅ |
| E3 | Capability detection OffscreenCanvas в воркере + внятная ошибка; fallback на main thread — бэклог | commit f79e788 | ✅ |
| E4 | e2e-экспорт PDF (проверка сигнатуры `%PDF-`) | `examples/viewer-xlsx/e2e/pdf.spec.ts` | ✅ |

### F. Тесты и приёмка

| ID | Задача | Артефакт | Статус |
| --- | --- | --- | --- |
| F1 | Тест совпадения точек переноса (DisplayList canvas vs PDF) через общий `render::text_measure::break_lines` | `crates/pdf/tests/export.rs::pdf_wraps_text_exactly_like_canvas`, commit 771a6b0 | ✅ |
| F2 | Кириллические фикстуры | `scripts/gen-fixtures.ts` + 3 фикстуры, `test-fixtures/xlsx/oracle.json` (113), commit 1a06421 | ✅ |
| F3 | `qpdf --check` + `pdftotext` в CI | `crates/pdf/tests/external.rs`, commit 5411d00 | ✅ |
| F4 | Чек-лист DoD в PR #11 | PR #11 (ведёт оркестратор) | 🔄 |

## Отклонения от исходной постановки

- Не создаём `crates/layout-core` — решение 1.
- Не делаем потоковую запись PDF — решение 2.
- Свой subsetting не пишем: он уже в printpdf (`subsetter` 0.42); `ttf-parser` в стеке снят (RUSTSEC-2026-0192), крейт pdf его намеренно не объявляет.
- Нумерация ADR 0007–0009 вместо 001–003: номера 0001–0006 заняты.
- Бенчи — в `crates/pdf/benches/`, не в корневом `benches/` (там только `.gitkeep`).
- E3 урезан до capability detection; fallback на main thread — в бэклог.
- В `examples/viewer-xlsx` кнопок экспорта не было — PDF-кнопка первая, она задаёт паттерн UI.
- PDF-экспорт вынесен в отдельный лениво загружаемый wasm-модуль `crates/pdf-wasm` — решение 8 (закрытие блокера B-1), в исходной постановке его не было.
