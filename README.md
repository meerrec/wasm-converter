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

## Встраивание: COOP/COEP и SharedArrayBuffer

Кадр собирается и рисуется в Web Worker'е (`OffscreenCanvas`), и у движка два
равноправных транспорта для него:

| Транспорт | Когда | Что это |
|---|---|---|
| `sab` | страница отдана с COOP/COEP | ring из двух слотов в `SharedArrayBuffer` |
| `bytes` | всегда, в том числе без заголовков | два `ArrayBuffer`, которые воркер заполняет по очереди |

Для `SharedArrayBuffer` нужны заголовки cross-origin isolation — их ставит
тот, кто отдаёт **страницу** (библиотека их себе поставить не может):

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

При embed это часто не под контролем потребителя: iframe на чужом сайте,
расширение, Electron/webview, GitHub Pages. **Просмотрщик от этого не
перестаёт работать**: без `crossOriginIsolated` воркер сам выбирает транспорт
`bytes`. API, кадры и экспорт (PNG, PDF, Markdown/HTML/текст) одинаковы на
обоих путях — отличается транспорт, а не поведение; какой путь выбран, видно
в `viewer.transport`. Задать транспорт явно можно опцией
`transport: 'sab' | 'bytes' | 'auto'` (по умолчанию `auto`); явный `sab` без
заголовков — ошибка инициализации, а не тихая подмена.

Что даёт SAB: кадр кладётся в разделяемый буфер, ring держит глубину два
(пока painter рисует один кадр, сборщик готовит следующий) и сам отбрасывает
устаревшие кадры при отставании. Что теряется без него: разделяемой памяти
нет, кадр едет обычным `ArrayBuffer` — двойная буферизация при этом та же, а
кадр собирает ровно та же функция Rust и рисует тот же painter. Разница —
в скорости; числа и бюджеты для обоих путей фиксируются отдельно
(`ROADMAP.md` §9), здесь они не приводятся, пока не измерены.

Демо на GitHub Pages подставляет заголовки вендоренным
`coi-serviceworker.js`; если сервис-воркеры недоступны, страница поедет на
`bytes` — это ожидаемо, а не поломка.

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
