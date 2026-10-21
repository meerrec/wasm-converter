# @meerrec/vue

## 1.2.0

### Minor Changes

- 16b9b1c: Обёртки фреймворков, телеметрия и рантаймы CLI

  - Новые пакеты `@meerrec/react`, `@meerrec/vue` и `@meerrec/svelte`:
    компоненты `XlsxViewer` и хуки/composable/store поверх `@meerrec/core`.
  - `@meerrec/core` — телеметрия на OpenTelemetry (ADR-0037) и обновлённые
    просмотрщики DOCX/XLSX.
  - `@meerrec/cli` — рантаймы Deno и Bun, общий пул воркеров.

- Первая публикуемая версия: composable `useXlsxViewer` и компонент `XlsxViewer`
  поверх `@meerrec/core`. Версия 1.1.0 — lockstep с npm-пятёркой и крейтами.

### Patch Changes

- Updated dependencies [16b9b1c]
  - @meerrec/core@1.2.0
