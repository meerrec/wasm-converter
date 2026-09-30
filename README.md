# doc-converter

High-performance OOXML (DOCX/XLSX) viewer & PDF exporter — Rust/WASM + OffscreenCanvas.

## Статус

**Фаза 1 завершена:** workspace, CI, RPC Main↔Worker, крейты-заглушки.

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
