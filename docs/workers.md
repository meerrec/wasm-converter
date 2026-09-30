# Web Worker + OffscreenCanvas

## COOP/COEP

Для `SharedArrayBuffer` нужны заголовки:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Без них движок работает на `ArrayBuffer` + transferables.

## Протокол

См. `packages/core/src/protocol.ts`.
