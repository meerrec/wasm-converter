---
---

# doc-converter 2.0.0

Мажорный релиз 2.0.0 — lockstep для десяти npm-пакетов `@meerrec/*` и всех
крейтов workspace: версии подняты с 1.4.3 до 2.0.0. Публикация идёт отдельным
релизным коммитом; пустой changeset нужен только push-гейту `changeset status`
и остаётся маркером релиза.

## Breaking changes

Удалены мёртвые публичные API:

- `@meerrec/core`: legacy offscreen-блок — `createViewer`, `initOffscreen`,
  `supportsOffscreen`, `assertFallbackAvailable`, типы `ViewerHandle` и
  `ViewerOptions` — и константа `VERSION`.
- `@meerrec/wasm`, `@meerrec/wasm-docx`, `@meerrec/wasm-pptx`: Vec-экспорты
  `xlsx_build_display_list`, `docx_build_display_list`,
  `pptx_build_display_list`; остались `_into`/`_sab`-варианты.
- `doc-converter-core`: `Archive::new` и `Archive::from_segmented` теперь
  принимают `ZipLimits`.
- `doc-converter-docx`, `doc-converter-xlsx`: удалён публичный модуль `ns`.

## Дедупликация

- Общие примитивы OOXML — `core::ns`, `core::xml`, `core::units`.
- Новый крейт `doc-converter-wasm-common` (в workspace 15 крейтов): общие
  потоковый ввод и транспорты кадра DisplayList.
- Общий `viewer_core.ts`/`WorkerCore` для pptx-воркера и фабрик вьюеров.

## Оптимизации

Кэши `SheetLayout`/`NumberFormat`, префиксные суммы границ столбцов, intern
шрифта, SAB 3→2 копии кадра, painter без аллокаций на команду, владение
контентом PDF и `fonts_map` из `Session`, кэш блобов диаграмм PPTX.

Замеры criterion (до → после): `xlsx/conditional` 5,63 → 3,98 мс (−29%),
гистограмма 7,71 → 5,92 мс; `pdf/size_compressed` 6,08 → 5,03 мс (−17%),
`pdf/size_1000_cells` 3,03 → 2,54 мс (−16%).
