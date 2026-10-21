# @meerrec/react

## 1.2.0

### Minor Changes

- 16b9b1c: Обёртки фреймворков, телеметрия и рантаймы CLI

  - Новые пакеты `@meerrec/react`, `@meerrec/vue` и `@meerrec/svelte`:
    компоненты `XlsxViewer` и хуки/composable/store поверх `@meerrec/core`.
  - `@meerrec/core` — телеметрия на OpenTelemetry (ADR-0037) и обновлённые
    просмотрщики DOCX/XLSX.
  - `@meerrec/cli` — рантаймы Deno и Bun, общий пул воркеров.

- Первая публикуемая версия: обёртка `useXlsxViewer`/`useDocxViewer` и компоненты
  `XlsxViewer`/`DocxViewer` поверх `@meerrec/core` (host-элемент, вьюер создаёт
  свой canvas). Версия 1.1.0 — lockstep с npm-пятёркой и крейтами (гейт
  `release.yml`).

### Patch Changes

- Updated dependencies [16b9b1c]
  - @meerrec/core@1.2.0
