# Спринт 2 (квартальный план): TextLayer, bidi и a11y-фундамент

**Файл:** `docs/sprint-2/plan.md`
**Версия:** 1.0 (22.10.2026)
**Длительность:** 10 рабочих дней
**Команда:** R1, R2 (Rust), TS, QA, DevOps
**Статус:** план утверждён, реализован в объёме [1]–[6]
**Предшествует:** Спринт 1 квартального плана (публикация, лимиты, fuzz) —
в репозитории не подтверждён, см. `report.md`
**Блокирует:** Спринт 3 (поиск), Спринт 4 (выделение `doc-converter-text`)

## 1. Цель

Превратить canvas-only просмотр DOCX в доступный документ: позиционированный
текстовый слой, нативное выделение, ARIA-семантика и явный bidi (UAX #9).
Три ключевых слова:

1. **Доступно** — текст читается скринридером и выделяется без canvas.
2. **Один источник геометрии** — TextLayer не перемеряет layout.
3. **Без регрессий** — canvas, PDF и DisplayList v4 не меняются; бюджеты
   контролируются, рост только осознанный.

## 2. Definition of Ready

- `main` = `1fb0072`, ветка `sprint/2-a11y`, отдельный worktree.
- RTL-фикстуры и 20/200-страничные a11y-фикстуры в репозитории.
- ADR-0040–0042 в статусе «Принято».
- Флаг `a11y` по умолчанию выключен; публичный API расширяется только
  `a11y`, `getTextLayer`, `getFullTextLayer`.

## 3. Решения по скоупу

1. **DisplayList не меняется.** TextLayer — параллельный выход layout; canvas
   получает RLM и правое выравнивание, `DL_VERSION` остаётся 4.
2. **`rustybuzz` в Спринте 2 не подключается** — бюджет; canvas и DOM шейпят
   строку сами. Точная shaped-ширина RTL не гарантируется, bbox — на уровне
   строки. PDF-RTL — Спринт 3.
3. **`unicode-bidi` без UCD-таблицы**: `hardcoded-data` выключена, классы
   символов — компактный классификатор. Итог: +7,05 КБ к wasm-docx, бюджет
   поднят 410 → 420 КБ (ADR-0040).
4. **Визуальные spans — построчные**, не по фрагментам: браузер шейпит строку
   целиком; фрагменты остаются метаданными для семантики и copy.
5. **Полное семантическое дерево** строится один раз, `user-select: none`;
   визуальный слой виртуализирован по страницам окна ±1.
6. **XLSX** — только минимальная обвязка (role/aria-hidden), DOM-сетка позже.

## 4. Фазы и слайсы

| Слайс | Что делает | Проверка | Выход |
|---|---|---|---|
| S1. Rust: фрагменты | `LineFragment` в `Line`/`CellLine`, ссылки/поля в layout, `w:cs`/`szCs` | `cargo test -p doc-converter-docx` | фрагменты строк |
| S2. Rust: TextLayer | `text_layer.rs`, табличный контекст, heading | unit + RTL-фикстуры | TextLayer API |
| S3. Rust: bidi | `w:bidi/w:rtl/w:bidiVisual`, уровни, RLM, зеркалирование колонок | RTL/числа/таблицы | bidi в layout и canvas |
| S4. WASM | `docx_get_text_layer`, `docx_get_full_text_layer`, wire | `wasm-pack test` | API заморожен |
| S5. TS: DOM-слой | прозрачные построчные spans, виртуализация, transform | vitest + e2e | выделение |
| S6. TS: ARIA и copy | semantic tree, TSV, Ctrl+A | axe + selection | a11y-флаг |
| S7. QA | axe, pixel-diff, RTL, perf, cross-browser | Playwright | DoD |
| S8. Docs | ADR, checklist, cookbook, ROADMAP | ревью | артефакты |

Зависимости: API TextLayer — к концу S4; bidi не блокирует TS (runs уже несут
`readingOrder`/`bidiLevel`); ARIA требует метаданных S2–S3.

## 5. Риски

- **FPS при 5000+ spans** → виртуализация с первого дня; perf-фикстура 200 стр.
- **bidi плывёт** → RTL-фикстуры в тестах, логический `readingOrder` отдельно
  от визуального порядка.
- **Бюджет wasm** → UCD-таблица выключена; рост +7,05 КБ задокументирован.
- **NVDA/VoiceOver не автоматизируются** → manual gate и AX-проверки.
- **Полный axe по 109 фикстурам медленный** → курируемый набор 18 DOCX/11 XLSX;
  полный корпус — ночной прогон.

## 6. DoD

1. NVDA читает 20-страничный DOCX — **manual gate**, на macOS не воспроизводим.
2. VoiceOver — **manual gate** + AX-проверки как прокси.
3. axe-core: 0 violations на курируемых DOCX/XLSX — ✅.
4. Drag-select через 3 страницы — ✅.
5. Таблица копируется TSV — ✅.
6. RTL визуально и в `readingOrder` — ✅.
7. p95 ≥ 55 fps на 200-страничном DOCX — ✅ (Chromium).
8. Pixel-diff canvas — бит-в-бит ✅.
9. Бюджеты — ✅ (docx 417,07 / 420 КБ).
10. Chrome/Firefox/WebKit — ✅ для новых спек.
11. ADR + checklist — ✅.
12. Cookbook-рецепт — ✅.

## 7. Артефакты

- `docs/adr/0040-bidi-strategy.md`, `0041-text-layer-dom-overlay.md`,
  `0042-rtl-font-coverage.md`
- `docs/a11y-checklist.md`, `docs/cookbook/a11y.md`
- `docs/sprint-2/report.md`
- `crates/docx/src/text_layer.rs`, `crates/docx-wasm/src/text_api.rs`,
  `packages/core/src/render/text_layer.ts`
