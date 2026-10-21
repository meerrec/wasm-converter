# @doc-converter/wasm

## 1.1.0

### Minor Changes

- 3caea59: Фаза 1 архитектурного аудита: подготовка публикации в npmjs.org (ADR-0027), wire-типы Rust → TS через ts-rs (ADR-0028) и рендер на main-потоке без OffscreenCanvas (ADR-0029). Появились опция `renderMode`, адресные ошибки IPC с `id`/`code`, коррелированный `open` и экспорт `init_painter_main`; `Viewport` и остальной публичный API не изменились.

## 1.0.0

### Major Changes

- c9a8236: Первый стабильный релиз: просмотр XLSX и DOCX в OffscreenCanvas-воркере
  (транспорты SAB и bytes), экспорт XLSX/DOCX в PDF, конвертация DOCX в
  Markdown/HTML/текст, CLI `convert`/`batch` с пулом воркеров. Подробности —
  в CHANGELOG пакетов и `docs/sprint-12/`.
