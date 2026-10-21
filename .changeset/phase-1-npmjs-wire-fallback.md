---
'@meerrec/core': minor
'@meerrec/wasm': minor
---

Фаза 1 архитектурного аудита: подготовка публикации в npmjs.org (ADR-0027), wire-типы Rust → TS через ts-rs (ADR-0028) и рендер на main-потоке без OffscreenCanvas (ADR-0029). Появились опция `renderMode`, адресные ошибки IPC с `id`/`code`, коррелированный `open` и экспорт `init_painter_main`; `Viewport` и остальной публичный API не изменились.
