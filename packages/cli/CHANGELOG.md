# @doc-converter/cli

## 1.1.0

### Minor Changes

- Релиз v1.1.0: синхронный подъём версии с `@meerrec/core` — гейт
  `release.yml` требует единую версию npm-пятёрки и крейта
  `doc-converter-core`. Содержимое пакета: LICENSE, README и метаданные
  (repository/homepage/bugs) для публикации в npmjs.org.

### Patch Changes

- Updated dependencies
- Updated dependencies
  - @meerrec/wasm-docx@1.1.0
  - @meerrec/wasm-pdf@1.1.0

## 1.0.0

### Major Changes

- c9a8236: Первый стабильный релиз: просмотр XLSX и DOCX в OffscreenCanvas-воркере
  (транспорты SAB и bytes), экспорт XLSX/DOCX в PDF, конвертация DOCX в
  Markdown/HTML/текст, CLI `convert`/`batch` с пулом воркеров. Подробности —
  в CHANGELOG пакетов и `docs/sprint-12/`.

### Patch Changes

- Updated dependencies [c9a8236]
  - @doc-converter/wasm-docx@1.0.0
  - @doc-converter/wasm-pdf@1.0.0
