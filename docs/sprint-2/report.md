# Отчёт Спринта 2 (квартальный план): TextLayer, bidi и a11y

**Файл:** `docs/sprint-2/report.md`
**Дата:** 22.10.2026
**Ветка:** `sprint/2-a11y`
**Статус:** реализовано [1]–[6]; два ручных DoD-пункта остаются за человеком

## Что сделано

### [1] Rust: TextLayer как выход layout

- `crates/docx/src/layout/paragraph.rs`: `LineFragment` (диапазон в тексте
  строки, x/width, стиль знака) и `Line::fragments`/`Line::align`/`Line::rtl`.
- `crates/docx/src/layout/engine.rs`: `LayoutItem::Paragraph` несёт фрагменты,
  физическое выравнивание, `rtl` и `heading_level`; `TableCellLayout` —
  `table_id`, `cell_id`, логические `row`/`col`, `grid_span`, `row_span`,
  `is_header`, `header_repeat`.
- `crates/docx/src/text_layer.rs`: `TextLayerWire` из готового `PageLayout`,
  `text_layer()` / `text_layer_page()`, сквозной `reading_order`, табличный
  контекст, дедупликация vMerge-продолжений.
- Починены ссылки и поля в теле абзаца (`collect_runs`): текст гиперссылок
  теперь попадает в canvas и TextLayer; неизвестные поля по-прежнему не
  разворачиваются — контракт `fields.rs` сохранён.
- Сегментация по скрипту: RTL-куски меряются `w:rFonts/@w:cs` и `w:szCs`.
- PDF-потребитель обновлён компиляционно, поведение LTR не менялось.

### [2] Rust: bidi

- `w:bidi`, `w:rtl`, `w:bidiVisual`, `w:cs`/`w:szCs` разбираются в модели,
  каскаде и стилях; `w:rtl` больше не попадает в `unknown`.
- `crates/docx/src/layout/bidi.rs`: UAX #9 через `unicode-bidi 0.3` с
  компактным классификатором классов символов (фича `hardcoded-data`
  выключена ради бюджета); база — `w:bidi` или первый сильный символ.
- `reading_order` логический; визуальный порядок даёт браузер. Уровни
  `bidi_level` считаются по фрагментам, LTR-кусок в RTL-абзаце получает
  чётный уровень.
- `crates/docx/src/render.rs`: RTL-строка получает RLM (`U+200F`) и правый
  якорь; `DisplayList` остаётся v4. `w:bidiVisual` зеркалит колонки в
  `TableFrame::column_x`, логический `col` не меняется.
- Тесты: арабский, иврит, смешанный абзац, числа в RTL, RTL-таблица,
  `bidiVisual`, RLM в кадре.

### [3] TS: positioned DOM-слой

- `packages/core/src/render/text_layer.ts`: прозрачный слой внутри `scroller`,
  `transform: translate(padLeft,0) scale(zoom)`; виртуализация по страницам
  окна ±1, LRU-кэш 8 страниц; обновление в том же `requestAnimationFrame`,
  что и canvas.
- Визуальные spans — построчные (canvas шейпит строку целиком); фрагменты
  остаются в данных для семантики и копирования.
- Выделение: drag по видимым страницам, кастомный `copy`, Ctrl+A — весь
  документ; таблица — TSV по логическим row/col; строки одного абзаца
  склеиваются пробелом.
- `DocxViewerHandle`: `getTextLayer(page)`, `getFullTextLayer()`;
  `DocxViewerOptions.a11y` (по умолчанию выключен).
- XLSX: `a11y: true` ставит `role=document` и `aria-hidden` на canvas;
  DOM-сетка — Спринт 3+.

### [4] ARIA

- `role=document`, `role=paragraph`, `role=heading aria-level`,
  `role=table/row/cell/columnheader`, `aria-rowindex/colindex/rowspan/colspan`;
  canvas скрыт, визуальный слой `aria-hidden`, семантическое дерево
  `user-select: none`.
- Повторные `w:tblHeader` на страницах-продолжениях дедуплицируются.
- Построчные spans не ломают чтение: семантика строится из полного слоя.

### [5] QA

- Новые e2e: `a11y.spec.ts`, `selection.spec.ts`, `rtl.spec.ts`,
  `pixel_diff.spec.ts`, `perf.spec.ts`, `axe.spec.ts`.
- axe-core: 0 violations на 18 DOCX и 11 XLSX (курируемый набор по категориям);
  полный корпус 109 DOCX прогоняется отдельно — пакетный axe на нём занимает
  десятки минут и не должен блокировать каждый PR.
- Pixel-diff: canvas с `a11y` off/on совпадает бит-в-бит (строже допуска
  0,1 %).
- Perf: 200-страничный DOCX, p95 ≥ 55 fps, в DOM не больше 4 страниц и 400
  строк.
- RTL: направления и уровни, `bidiVisual`, чтение в трёх движках.
- `docs/a11y-checklist.md`: скрипты NVDA/VoiceOver, автоматические гейты,
  матрица ограничений.

### [6] DevOps

Playwright-матрица Chromium/Firefox/WebKit + DPR2 уже была; новые спеки в неё
включены. `axe-core` добавлен devDependency в `examples/viewer`. Бюджет
пересобран; лимит `wasm (docx)` 410 → 420 КБ.

## Метрики

| Что | Значение |
|---|---|
| Rust `cargo test --workspace --all-features` | все тесты зелёные |
| `cargo clippy --workspace --all-targets -- -D warnings` | зелёный |
| `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps` | зелёный |
| core vitest | 130/130 |
| react/vue/svelte/cli | 6/4/3/44 |
| `pnpm turbo run typecheck build` | 17/17 |
| `pnpm size-limit` | docx 417,07/420 КБ; viewer 476,27/480 КБ; остальные в бюджете |
| axe (курируемый набор) | 0 violations |
| pixel-diff | 0 отличий |
| perf 200 стр. | p95 ≥ 55 fps (Chromium) |

## DoD

| # | Критерий | Статус |
|---|---|---|
| 1 | NVDA читает 20-страничный DOCX | ⬜ manual gate: на macOS не воспроизводится, нужен Windows-инженер |
| 2 | VoiceOver | ⬜ manual gate: скрипт готов, прогон за человеком |
| 3 | axe: 0 violations | ✅ курируемый набор 18 DOCX + 11 XLSX |
| 4 | Drag-select через 3 страницы | ✅ |
| 5 | Таблица — TSV | ✅ |
| 6 | RTL визуально и в reading order | ✅ |
| 7 | p95 ≥ 55 fps на 200 стр. | ✅ Chromium; CI-прокси — число страниц/строк в DOM |
| 8 | Pixel-diff canvas ≤ 0,1 % | ✅ бит-в-бит |
| 9 | Бюджеты | ✅ с осознанным ростом docx 410 → 420 КБ |
| 10 | Chrome/Firefox/WebKit | ✅ новые спеки в трёх движках |
| 11 | ADR + checklist | ✅ ADR-0040–0042, `docs/a11y-checklist.md` |
| 12 | Cookbook | ✅ `docs/cookbook/a11y.md`, 4 рецепта |

## Отклонения и спорные решения

1. **Бюджет `wasm (docx)` +7,05 КБ (410 → 420).** Полная UCD-таблица
   `unicode-bidi` давала +13,2 КБ; выключили `hardcoded-data`, написали
   компактный классификатор. Отказ от `unicode-bidi` сломал бы смешанные
   RTL-абзацы. Решение зафиксировано в ADR-0040 и `.size-limit.mjs`.
2. **`rustybuzz` не подключён.** Точная shaped-ширина арабского не
   гарантируется; bbox — на уровне строки, визуальный порядок и DOM дают
   браузерный shaping. PDF-RTL — Спринт 3.
3. **NVDA/VoiceOver не автоматизированы.** DoD 1–2 остаются ручными; AX-дерево
   проверяется ролями и aria-атрибутами, это прокси, а не замена.
4. **Полный axe по 109 фикстурам вынесен из PR-гейта.** Курируемый набор
   покрывает категории; полный корпус — ночной прогон.
5. **Постановка расходится с репозиторием:** тег только `v1.2.0` (не 1.2.1),
   нет `docs/security.md`, 37 ADR (не 39), 109 DOCX-фикстур (не 18), лимиты
   ZIP в ADR-0015 другие, `ViewerOptions` — deprecated TS-интерфейс. Работа
   велась от фактического кода.

## Что уходит в Спринт 3

- `rustybuzz`/shaping и PDF-RTL.
- Пофрагментное выделение и `doc-converter-text`.
- Поиск по TextLayer.
- Полная XLSX DOM-сетка и frozen panes.
- Ночной axe по всему корпусу.
- iOS Safari (Спринт 6).
- Ручные NVDA/VoiceOver-прогоны.
