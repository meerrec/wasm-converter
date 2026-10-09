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

## Non-goals v1

Осознанно вне рамок v1 (каждый пункт закрыт ADR, полный список — `docs/sprint-8/plan.md` §7):

- Strict OOXML — только Transitional ([ADR-0017](docs/adr/0017-strict-vs-transitional.md)).
- Writer в DOCX не реализуется ([ADR-0020](docs/adr/0020-parser-writer-boundary.md)); command API, undo/redo, diff/patch — [ADR-0018](docs/adr/0018-editing-boundaries.md).
- Track changes (`w:ins`/`w:del`) парсятся как `Unknown`; VML (`mc:Fallback`) игнорируется ([ADR-0014](docs/adr/0014-alternate-content.md)).
- TOC: поле парсится, содержимое не генерируется; OMML, SmartArt, embedded OLE — `Unknown`.
- Encrypted DOCX, `.docm`, `.doc` — фатальная ошибка ([ADR-0016](docs/adr/0016-docx-error-policy.md)).
- Резолвинг каскада стилей, layout и pagination — Спринт 9.

## Лицензия

Apache-2.0
