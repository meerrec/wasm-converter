# doc-converter

High-performance OOXML (DOCX/XLSX) viewer & PDF exporter — Rust/WASM + OffscreenCanvas.

## Статус

**Фаза 1 завершена:** workspace, CI, RPC Main↔Worker, крейты-заглушки.

## Демо

Собранный `examples/viewer-xlsx`: <https://meerrec.github.io/wasm-converter/>. Публикуется workflow `Pages` при пуше в `main`.

При первом заходе страница перезагружается: GitHub Pages не отдаёт заголовки cross-origin isolation (COOP/COEP), их подставляет service worker.

## Быстрый старт

```bash
# Rust
cargo test --workspace

# TS (сначала биндинги: их .d.ts нужны typecheck'у пакета core)
pnpm install
pnpm build:wasm
pnpm turbo run typecheck test build
```

## Пакеты

| Пакет | Назначение |
|---|---|
| `@doc-converter/core` | worker RPC, OffscreenCanvas viewer, protocol |
| `@doc-converter/wasm` | wasm-bindgen exports (генерируется) |

## Лицензия

Apache-2.0
