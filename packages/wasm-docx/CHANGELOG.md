# @doc-converter/wasm-docx

## 1.2.0

### Minor Changes

- Релиз v1.1.0: синхронный подъём версии с `@meerrec/core` — гейт
  `release.yml` требует единую версию npm-пятёрки и крейта
  `doc-converter-core`. Содержимое пакета: метаданные и README для
  публикации в npmjs.org, пересборка биндингов против
  doc-converter-core 1.1.0 (zip 9, wire-типы через ts-rs).

## 1.0.0

### Major Changes

- c9a8236: Первый стабильный релиз: просмотр XLSX и DOCX в OffscreenCanvas-воркере
  (транспорты SAB и bytes), экспорт XLSX/DOCX в PDF, конвертация DOCX в
  Markdown/HTML/текст, CLI `convert`/`batch` с пулом воркеров. Подробности —
  в CHANGELOG пакетов и `docs/sprint-12/`.
