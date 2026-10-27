# Web Worker + OffscreenCanvas

## COOP/COEP

Для `SharedArrayBuffer` нужны заголовки:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

На хостингах, которые не отдают свои заголовки (GitHub Pages), их ставит единственный service worker сайта — наш [`scripts/pages/sw.js`](../scripts/pages/sw.js), корневой (вендоренного `coi-serviceworker.js` в демо больше нет). Корневая область нужна, чтобы воркер видел навигации демо в `/viewer/`: регистрация из корня сайта, а не из каталога демо, оставляет воркеру всю область публикации. COEP он ставит всем ответам (его спрашивает и воркер-скрипт в своём собственном ответе), кэшей не держит, а включается на первом заходе, поэтому страница перезагружается. Изоляцию получают только браузеры, умеющие встраивать кадр в изолированную страницу — сегодня это Chromium; проверка стоит и на страницах, и в самом воркере. Firefox и Safari остаются без изоляции: вьюер едет на транспорте `bytes` (ADR-0021).

## Слои кадра

Путь кадра выбирается один раз и исполняется тремя слоями ([ADR-0044](adr/0044-unified-execution-path.md)):

- **ExecutionPlan** — [`execution/plan.ts`](../packages/core/src/execution/plan.ts): `resolveRenderTarget` выбирает место рендера, `resolveExecutionPlan` — транспорт. Допустимые планы — `worker+sab`, `worker+bytes`, `main+bytes`; `main+sab` не создаётся: в main-режиме кадры всегда едут буферами.
- **TransportAdapter** — [`transport_sab.ts`](../packages/core/src/execution/transport_sab.ts) и [`transport_array_buffer.ts`](../packages/core/src/execution/transport_array_buffer.ts): возит кадр (SAB-кольцо или два `ArrayBuffer`), переиспользуя `FramePool`, `FrameRelay` и кадровые циклы.
- **RenderAdapter** — [`worker/execution/`](../packages/core/src/worker/execution/) (`OffscreenRenderAdapter`, `ForwardingRenderAdapter`) и [`render/render_adapter.ts`](../packages/core/src/render/render_adapter.ts) (`MainRenderAdapter`): рисует в воркере или на main.

`auto` выбирает воркер при `transferControlToOffscreen`, иначе main; явные несовместимости (`worker` без OffscreenCanvas, `sab` без COOP/COEP, `main` + `sab`) — ошибка инициализации, а не подмена.

## Протокол

См. `packages/core/src/protocol.ts`.
