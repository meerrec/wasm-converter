# Доступность: чек-лист приёмки

Спринт 2 добавил в просмотрщик DOCX доступное представление: прозрачный
DOM-слой (`TextLayer`) поверх canvas с семантическим деревом, выделением и
копированием. Автоматика ловит регрессии разметки, геометрии и порядка
чтения, но не заменяет прогон настоящим скринридером: поведение NVDA и
VoiceOver зависит от ОС, браузера и настроек речи. Этот документ фиксирует,
что проверяется машиной, что остаётся ручным, и честный статус ручных
прогонов.

## Что покрыто автоматикой

| Гейт | Где | Что проверяет |
|---|---|---|
| DOM и ARIA | `examples/viewer/e2e/docx/a11y.spec.ts` | `.dc-text-layer[role=document]`, `aria-hidden` на canvas и визуальном слое, уровни заголовков, 3 таблицы, `aria-rowindex`/`aria-colindex` |
| axe-core | `examples/viewer/e2e/axe.spec.ts` | 0 violations по WCAG 2 A/AA на 18 DOCX и 11 XLSX (chromium); полный список фикстур — в самом спеке |
| Выделение и копирование | `examples/viewer/e2e/docx/selection.spec.ts` | drag через три страницы в порядке чтения, таблица как TSV, Ctrl+A на весь документ |
| RTL | `examples/viewer/e2e/docx/rtl.spec.ts` | `direction`/`bidiLevel`, монотонный `readingOrder`, `w:bidiVisual` при логическом `col` |
| Pixel-diff | `examples/viewer/e2e/docx/pixel_diff.spec.ts` | кадр с `a11y: true` и без флага совпадает бит-в-бит |
| Производительность | `examples/viewer/e2e/docx/perf.spec.ts` | 200 страниц: p95 ≥ 55 fps, в DOM ≤ 4 страниц и ≤ 400 строк |
| Rust | `crates/docx/src/text_layer.rs`, `crates/docx-wasm/tests/text_api.rs` | runs и `pageMap`, сквозной `readingOrder`, табличный контекст, срезы страниц, ошибки диапазона |
| Прокси к AX-tree | `examples/viewer/e2e/docx/a11y.spec.ts` | роли и `aria-*`, из которых браузер строит accessibility tree; отдельного снимка AX-tree в наборе нет — проверяется источник |

Автоматика **не** проверяет: реальные объявления скринридера, работу
quick keys и ротора, VoiceOver на iOS, а также текст за пределами тела
документа (колонтитулы, сноски, комментарии).

## Ручной прогон NVDA (Windows)

**Статус: не выполнялось.** NVDA работает только на Windows; на macOS этот
прогон не воспроизводится. Требуется Windows-инженер.

### Подготовка

- Windows 10/11, NVDA актуальной стабильной версии.
- Chromium-браузер (Chrome или Edge).
- Демо из этого репозитория:

  ```bash
  pnpm install
  pnpm -F @meerrec/example-viewer dev
  ```

- Открыть адрес dev-сервера (по умолчанию Vite — `http://localhost:5173`)
  с флагами `?a11y=1#/docx`, например
  `http://localhost:5173/?a11y=1#/docx`.
- В списке фикстур выбрать `a11y/a11y_audit.docx` — 20 страниц с
  заголовками H1/H2, тремя таблицами (шапка, `gridSpan`, `vMerge`),
  списками, ссылкой и полем PAGE.
- Запустить NVDA (Ctrl+Alt+N) и убедиться, что включён режим обзора
  (browse mode).

### Сценарий

1. Перейти в документ (Tab до слоя или клик по тексту). NVDA должна
   объявить документ, а не «canvas» или «image».
2. Пройти по заголовкам клавишей **H**. Ожидание: первый заголовок —
   «Доступность документа» уровня 1, далее H2; всего не меньше 16
   заголовков (4 H1 + 12 H2 в фикстуре), уровни не пропущены.
3. Пройти по таблицам клавишей **T**. Ожидание: три таблицы; NVDA
   объявляет позицию строки/колонки, ячейки шапки — как заголовки колонок,
   объединённые ячейки — с учётом span.
4. Пройти списки стрелками вниз. Клавиша **L** списков не найдёт: ролей
   `list`/`listitem` в Спринте 2 нет. Это известное ограничение, а не
   регресс; важен порядок пунктов и отсутствие дублей.
5. Проверить ссылку и поле: текст «Полное руководство…» читается, поле
   PAGE развёрнуто в «Версия макета: 1». URL ссылки в TextLayer не
   несётся — это тоже известное ограничение.
6. Проверить отсутствие canvas-мусора: нет объявлений «graphic»,
   «canvas», безымянных изображений и повторного текста из
   `.dc-text-visual` (он `aria-hidden`).

### Критерии успеха

- Порядок чтения совпадает с логическим порядком документа, без дублей и
  перескоков между страницами.
- Роли объявляются: документ, заголовок с уровнем, таблица, строка,
  ячейка/заголовок колонки.
- Quick keys **H** и **T** находят ожидаемые элементы; **L** не находит
  списков — это ожидаемо.
- Canvas не попадает в речь скринридера.
- Нет ошибок в консоли браузера.

## Ручной прогон VoiceOver (macOS)

**Статус: ручной прогон (macOS, Спринт 2).** Автоматический прокси —
семантическое дерево в `e2e/docx/a11y.spec.ts`; полного снимка AX-tree в
наборе нет, поэтому ротор и объявления проверяются руками.

### Сценарий

1. Открыть то же демо с `?a11y=1#/docx` и фикстурой
   `a11y/a11y_audit.docx` в Safari.
2. Включить VoiceOver (Cmd+F5), открыть ротор **VO+U**.
3. В роторе **Headings**: не меньше 16 заголовков, уровни 1–2, порядок
   логический. В **Tables**: три таблицы. В **Links**: текст ссылки.
4. Войти в таблицу (VO+Shift+Down) и пройти ячейки (VO+стрелки): должны
   объявляться строка, колонка и заголовок колонки.
5. Прочитать документ подряд (VO+стрелка вправо): абзацы идут в порядке
   документа, визуальные spans не дублируют текст.
6. Проверить списки: пункты читаются как абзацы (ролей списка нет), без
   маркеров.
7. Убедиться, что canvas и `.dc-text-visual` не попадают в rotor и не
   объявляются.

Критерии успеха — те же, что у NVDA: порядок, роли, отсутствие
canvas-мусора; отсутствие списков в роторе — известное ограничение.

### Статус ручных проверок

| Проверка | Платформа | Статус |
|---|---|---|
| NVDA | Windows + Chromium | **Не выполнялось**: на macOS не воспроизводится; требуется Windows-инженер |
| VoiceOver | macOS + Safari | Ручной прогон (Спринт 2) |
| AX-tree (прокси) | Chromium, e2e | Автоматически: роли и `aria-*` семантического дерева |
| iOS Safari | iOS + VoiceOver | Не проверялось; Спринт 6 |

## Автоматические гейты

Из корня репозитория:

```bash
# DOM-слой, ARIA и семантическое дерево
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx/a11y.spec.ts --project=chromium

# axe-core: 0 violations на репрезентативном наборе DOCX и XLSX
pnpm -F @meerrec/example-viewer exec playwright test e2e/axe.spec.ts --project=chromium

# Выделение, TSV и Ctrl+A
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx/selection.spec.ts --project=chromium

# RTL: direction, readingOrder, bidiVisual
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx/rtl.spec.ts --project=chromium

# Кадр не меняется от включения слоя
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx/pixel_diff.spec.ts --project=chromium

# Виртуализация: 200 страниц, p95 ≥ 55 fps
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx/perf.spec.ts --project=chromium

# Все DOCX-спеки и axe разом
pnpm -F @meerrec/example-viewer exec playwright test e2e/docx e2e/axe.spec.ts --project=chromium

# Rust: сборка TextLayer и wasm-мост
cargo test -p doc-converter-docx text_layer
wasm-pack test --headless --chrome --release crates/docx-wasm
```

E2E сам поднимает демо на порту 5178. Для `wasm-pack test` нужен
`chromedriver` в `PATH` (см. `AGENTS.md`).

## Матрица известных ограничений

| Ограничение | Влияние | План |
|---|---|---|
| Списки без маркеров и без ролей `list`/`listitem` | копирование теряет маркеры, скринридер читает пункты как абзацы | бэклог: маркеры в сериализаторе и роли списка |
| Колонтитулы вне TextLayer | не читаются и не копируются вместе с телом | бэклог: расширение TextLayer |
| Сноски и комментарии вне TextLayer | недоступны скринридеру и выделению | бэклог: расширение TextLayer |
| URL гиперссылки не несётся в `TextRun` | текст ссылки доступен, адрес — нет | бэклог: поле ссылки в `TextRun` |
| Выделение мышью — только видимые страницы | drag не покрывает весь документ | Ctrl+A даёт весь текст; точное выделение — Спринт 4 |
| XLSX: только `role="document"` и `aria-hidden` на canvas | DOM-сетки ячеек нет, ячейки недоступны | отдельное решение после Спринта 2 |
| PDF-RTL | RTL-текст в PDF рисуется без bidi | Спринт 3 |
| Точная shaped-ширина арабского не гарантируется | возможен визуальный выход за строку | bbox строки стабилен; shaping — Спринт 3 |
| iOS Safari не проверялся | поведение VoiceOver на iOS неизвестно | Спринт 6 |
| NVDA не запускался | поведение на Windows не подтверждено | нужен Windows-инженер |

## Как включать

В коде:

```ts
import { createDocxViewer } from '@meerrec/core';

const viewer = await createDocxViewer(host, {
  workerUrl,
  docxWasm,
  a11y: true, // DOM-слой, ARIA и выделение
});
await viewer.open(bytes);
```

Флаг задаётся при создании вьюера; на лету он не переключается. В демо
доступный режим включается параметром `?a11y=1`:

```text
http://localhost:5173/?a11y=1#/docx
```

XLSX в Спринте 2 получает только минимальную обвязку:

```ts
import { createXlsxViewer } from '@meerrec/core';

const viewer = await createXlsxViewer(host, {
  workerUrl,
  a11y: true, // scroller[role=document], canvas[aria-hidden]; DOM-сетки нет
});
```

Подробности для приложений — в рецептах
[cookbook/a11y.md](cookbook/a11y.md); решения по слою и bidi —
[ADR-0041](adr/0041-text-layer-dom-overlay.md) и
[ADR-0040](adr/0040-bidi-strategy.md).
