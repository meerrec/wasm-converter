# ADR-0045: PDF — бэкенд без знания форматов и отдельные адаптеры экспорта

**Статус:** Предложено
**Дата:** 23.10.2026
**Контекст:** аудит архитектуры v1.4.3
(`docs/analysis/architecture-audit-2026-10-23.md`, §3.2, §3.4, §3.5);
`crates/pdf` зависит от `docx` и `xlsx`; `pptx-pdf` использует публичные
модули `pdf`
**Связанные ADR:** [ADR-0007](0007-pdf-layout-boundary.md) (граница
раскладки PDF), [ADR-0011](0011-chart-geometry.md) (геометрия диаграмм),
[ADR-0022](0022-own-pdf-backend.md) (свой PDF-бэкенд),
[ADR-0033](0033-pptx-plugin.md) (PPTX-плагин),
[ADR-0044](0044-font-assets.md) (шрифты как ассет)

---

## Контекст

- ADR-0007 сознательно оставил раскладку PDF внутри `crates/pdf` и разрешил
  ему зависеть от `xlsx` (и позже от `docx`), чтобы canvas и PDF мерили текст
  одним кодом. Это сработало для двух форматов.
- Сейчас `crates/pdf` — одновременно бэкенд (writer/session/ops/content/
  fonts/image) и два форматных экспортёра:
  - XLSX: `painter.rs` (подготовка книги/листа, геометрия, пагинация) и
    `pagination.rs` — ~2 000 строк, специфичных для SpreadsheetML;
  - DOCX: `docx/mod.rs`, `docx/tables.rs`, `docx/images.rs`,
    `docx/floats.rs` — ~1 500 строк; диспетчер `LayoutItem` дублирует
    `docx/src/render.rs:242-333`.
- `pptx-pdf` зависит от `pdf` и использует его публичные `session`,
  `content`, `ops`, `fonts`, `image`, `chart`, `layout`, `styles`. Но
  `pptx-pdf-wasm` тянет `doc-converter-pdf`, а тот — `docx` и `xlsx`, хотя
  PPTX-пути нужен только бэкенд. Это лишний код в wasm-модуле.
- `pptx-pdf` почти построчно повторяет каркас экспорта DOCX
  (`pptx-pdf/src/lib.rs:100-178` против `pdf/src/docx/mod.rs:62-158`):
  раскладка, `collect_faces`, `collect_glyphs`, `EmbeddedFonts`, регистрация
  шрифтов, имена XObject, цикл страниц, `translate`/`finish`. Базовая линия
  текста и гарнитура при этом расходятся: DOCX — `typo_ascent_px`, PPTX —
  `registry.ascent`; раскладка мерит Carlito, canvas рисует
  `run.font`/«Calibri», PDF встраивает Carlito.
- Схлопывание границ DOCX реализовано в canvas (`docx/src/render/tables.rs`)
  и в PDF (`pdf/src/docx/tables.rs`) по-разному; PDF дополнительно гасит
  рёбра внутри vMerge, canvas — нет. Это уже визуальное расхождение.
- Добавление нового формата сегодня требует правки `crates/pdf`; граница
  «бэкенд ↔ формат» размыта.

## Решение

1. **`doc-converter-pdf` — только бэкенд.** В крейте остаются
   `pdf/{writer,session,ops,content,fonts,image}`, `chart`, `options`,
   `text`, `styles` (только форматно-нейтральные части) и геометрические
   примитивы. Зависимости: `core`, `render`, `lopdf`, `image`, `subsetter`,
   `skrifa`. Зависимостей от `docx`/`xlsx`/`pptx` нет.
2. **Форматные адаптеры — отдельные крейты:**
   - `doc-converter-xlsx-pdf`: `SheetLayout` → страницы → операции PDF;
     сюда переезжают `pdf/src/painter.rs` (XLSX-часть), `pagination.rs`,
     `styles.rs`, `border.rs`, `background.rs`.
   - `doc-converter-docx-pdf`: `Document`/`PageLayout` → операции PDF; сюда
     переезжают `pdf/src/docx/*`, `annot.rs`, `overlay.rs` (DOCX-часть).
   - `doc-converter-pptx-pdf` остаётся, но зависит от
     `doc-converter-pdf` без транзитивных `docx`/`xlsx`.
3. **Общая покраска таблиц DOCX.** Схлопывание рёбер, заливка и штрихи
   выносятся в `docx` (например, `docx::paint::table`) и потребляются и
   canvas-путём, и `docx-pdf`. Фильтр рёбер внутри vMerge становится общим,
   а не PDF-only. Это устраняет расхождение и дублирование.
4. **Композиция wasm.** `pdf-wasm` собирает `xlsx-pdf` + `docx-pdf` +
   `pdf`; `pptx-pdf-wasm` — `pptx-pdf` + `pdf`. Возможное слияние
   `pptx-pdf-wasm` в `pdf-wasm` (ADR-0033 §7/§9 его предусматривал)
   оценивается отдельно: минус модуль/пакет и дедуп PDF-бэкенда против роста
   `pdf-wasm` для XLSX/DOCX-only потребителей. Решение — по замеру gzip и
   cold start, а не по умолчанию.
5. **Миграция поэтапно:**
   - Фаза A: вынести `pdf/src/docx/*` в `docx-pdf`, убрать `docx` из
     зависимостей `pdf`; `pdf-wasm` подключает адаптер.
   - Фаза B: вынести XLSX-часть (`painter`/`pagination`/`styles`) в
     `xlsx-pdf`, убрать `xlsx` из `pdf`.
   - Фаза C: общий `docx::paint::table`, переключить canvas и PDF, сверить
     SSIM.
   - Фаза D: замер слияния `pptx-pdf-wasm` с `pdf-wasm`.
7. **Общий каркас постраничного экспорта.** В бэкенде появляется generic
   `pdf::export` (источник страниц, регистрация faces/glyphs/images, цикл
   страниц, `translate`/`finish`), который используют `docx-pdf` и
   `pptx-pdf`; уникальными остаются только рисовальщики страницы/слайда.
   Заодно согласуются базовая линия (`typo_ascent_px` для обоих) и имя
   семейства для раскладки, canvas и PDF — сейчас они расходятся. Это
   убирает ~150–200 строк scaffold и риск SSIM-расхождения.
8. **Экспорт не только в `Vec`.** `export_to<W>` и честная передача
   `compress`/метаданных нужны и DOCX, и PPTX; `export_pptx_pdf` больше не
   теряет `PdfOptions`.
6. **Публичный API.** `PdfExporter` переезжает в адаптеры; `doc-converter-pdf`
   публикует только бэкенд-примитивы. Версии крейтов — lockstep, но это
   Breaking-изменение для прямых потребителей Rust-крейтов (они отдаются
   ассетами релиза, ADR-0026) — фиксируется в changelog.

## Рассмотренные варианты

- **Оставить как есть.** Работает, но каждый новый формат правит `pdf`,
  а `pptx-pdf-wasm` тащит лишние `docx`/`xlsx`. Отклонено как целевое
  состояние.
- **Feature-gates внутри `pdf`.** Меньший шаг, но крейт остаётся единым, а
  ленивость wasm всё равно требует отдельных бинарников; границы не
  становятся явными. Отклонено.
- **Общий `layout-core`/IR (как в ADR-0007).** Отклонено там же: canvas и
  PDF уже делят метрики и перенос, а полный IR не нужен.
- **Перенести `pptx-pdf` внутрь `crates/pdf` под feature `pptx`.** Меньший
  шаг, который убирает каретку scaffold и не заводит новый крейт. Отклонено
  как целевое состояние: бэкенд снова знает формат, а `pptx-pdf-wasm`
  продолжает тянуть `docx`/`xlsx`; допустим как временная мера фазы A–B.
- **Слить `pptx-pdf-wasm` в `pdf-wasm` сразу.** Не решается без замера:
  цена — PPTX-код в PDF-модуле; вынесено в фазу D.

## Последствия

- Новый формат добавляет адаптер, а не ветку в `pdf`; `pdf` перестаёт быть
  «крейтом, который знает все форматы».
- `pptx-pdf-wasm` перестаёт компилировать `docx`/`xlsx`; ожидаемое
  сокращение модуля — оценка, требует замера.
- Два новых крейта и два набора тестов; CI-матрица расширяется, зато
  границы проверяются компилятором.
- Общая покраска таблиц DOCX снимает расхождение vMerge и уменьшает
  дублирование; риск — SSIM-регрессии, поэтому фаза C идёт с оракулом.
- Публичный API `doc-converter-pdf` сужается; `PdfExporter` доступен из
  адаптеров. Это осознанный Breaking-изменение.
- Бюджеты PDF-модулей пересчитываются после фаз A–D; `size-limit` —
  обязательный гейт.
