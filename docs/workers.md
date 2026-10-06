# Web Worker + OffscreenCanvas

## COOP/COEP

Для `SharedArrayBuffer` нужны заголовки:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Путь без SAB не реализован: воркер безусловно зовёт `alloc_sab`, а `initOffscreen` ждёт `ready` именно с `sab` (см. [`adr/0004-sab-acceleration.md`](adr/0004-sab-acceleration.md), раздел «Где путь без SAB обрывается»). SAB-путь — единственный собранный.

На хостингах, которые не отдают свои заголовки (GitHub Pages), их подставляет вендоренный [`examples/viewer-xlsx/public/coi-serviceworker.js`](../examples/viewer-xlsx/public/coi-serviceworker.js); он включается на первом заходе, поэтому страница перезагружается.

## Протокол

См. `packages/core/src/protocol.ts`.
