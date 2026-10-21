# @meerrec/svelte

Svelte-обёртки над просмотрщиками [`@meerrec/core`](../core/README.md):
компонент `XlsxViewer.svelte` и store `createViewerStore(options)`. Обёртки
тонкие: разметку, стили, воркер и холст делает фабрика core, пакет только
держит жизненный цикл (создание в `onMount`, `destroy()` в `onDestroy`).

> **Отклонение от плана T4.2:** фабрика `createXlsxViewer` принимает
> хост-элемент (`HTMLElement`), в который сама монтирует прокрутку и холст.
> Поэтому компонент рендерит **host-div** (`bind:this`), а не `<canvas>`.

Пакет публикует исходник `.svelte` (легаси-синтаксис, компилируется и под
Svelte 4, и под Svelte 5): потребитель компилирует его своим компилятором —
поэтому peer-диапазон `svelte: ^4 || ^5` честный для обеих мажорных версий.

## Установка

```bash
pnpm add @meerrec/core @meerrec/svelte
# npm i @meerrec/core @meerrec/svelte
```

`svelte` — peerDependency: `^4 || ^5`.

## Использование

```svelte
<script lang="ts">
  import { XlsxViewer } from '@meerrec/svelte';
</script>

<div style="width: 100%; height: 80vh">
  <XlsxViewer options={{ zoom: 1.2 }} />
</div>
```

Императивно, через store:

```ts
import { createViewerStore } from '@meerrec/svelte';

const store = createViewerStore({ zoom: 1.2 });
await store.attach(document.getElementById('viewer')!);
// ...
store.destroy();
```

Хост обязан иметь размеры: фабрика кладёт в него прокрутку и холст на 100 %
ширины и высоты. При отсутствии `OffscreenCanvas` фабрика сама переходит в
main-режим (`renderMode: 'auto'` по умолчанию) — настройка не нужна.

## API

- `XlsxViewer` — компонент с пропсом `options?: XlsxViewerOptions`; рендерит
  host-div, вьюер создаёт в `onMount`, уничтожает в `onDestroy`.
- `createViewerStore(options?)` → `{ subscribe, attach(host), destroy() }` —
  store ручки вьюера (`XlsxViewerHandle | null`) для императивного сценария;
  повторный `attach` уничтожает прежнюю ручку.
- Типы `XlsxViewerOptions`/`XlsxViewerHandle` реэкспортируются из
  `@meerrec/core`.

## Демо

Демо-страницы Svelte нет намеренно: приложение `examples/viewer` написано на
React, и интеграция Vue/Svelte в него была бы лишней зависимостью демо.
Покрытие — vitest + `svelte-check --tsconfig ./tsconfig.json`.

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
