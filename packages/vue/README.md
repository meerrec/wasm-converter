# @meerrec/vue

Vue-обёртки над просмотрщиками [`@meerrec/core`](../core/README.md): composable
`useXlsxViewer` и компонент `XlsxViewer` (через `defineComponent` с
render-функцией, без SFC — сборка остаётся tsup-простой). Обёртки тонкие:
разметку, стили, воркер и холст делает фабрика core, пакет только держит
жизненный цикл (создание в `onMounted`, `destroy()` в `onUnmounted`).

> **Отклонение от плана T4.2:** фабрика `createXlsxViewer` принимает
> хост-элемент (`HTMLElement`), в который сама монтирует прокрутку и холст.
> Поэтому composable держится за **host-div**, а не за `<canvas>`:
> `useXlsxViewer(hostRef, options?)`, где `hostRef` — `Ref<HTMLElement | null>`.

## Установка

```bash
pnpm add @meerrec/core @meerrec/vue
# npm i @meerrec/core @meerrec/vue
```

`vue` — peerDependency: `^3`.

## Использование

```vue
<script setup lang="ts">
import { ref } from 'vue';
import { useXlsxViewer } from '@meerrec/vue';

const hostRef = ref<HTMLDivElement | null>(null);
const viewer = useXlsxViewer(hostRef, { zoom: 1.2 });
</script>

<template>
  <div ref="hostRef" style="width: 100%; height: 80vh" />
</template>
```

То же через компонент:

```vue
<script setup lang="ts">
import { XlsxViewer } from '@meerrec/vue';
</script>

<template>
  <div style="width: 100%; height: 80vh">
    <XlsxViewer :options="{ zoom: 1.2 }" />
  </div>
</template>
```

Компонент отдаёт ручку вьюера через `expose` (`viewer` — shallowRef с
`XlsxViewerHandle | null`), забрать её можно template ref-ом.

Хост обязан иметь размеры: фабрика кладёт в него прокрутку и холст на 100 %
ширины и высоты. При отсутствии `OffscreenCanvas` фабрика сама переходит в
main-режим (`renderMode: 'auto'` по умолчанию) — настройка не нужна.

## API

- `useXlsxViewer(hostRef, options?)` →
  `Readonly<ShallowRef<XlsxViewerHandle | null>>` — вьюер создаётся в
  `onMounted` (если хост уже в DOM), `destroy()` вызывается в `onUnmounted`;
  `null`, пока вьюер не создан.
- `XlsxViewer({ options? })` — рендерит только host-div и подключает
  composable; наружу через `expose` отдаёт `viewer`.
- Типы `XlsxViewerOptions`/`XlsxViewerHandle` реэкспортируются из
  `@meerrec/core`.

## Демо

Демо-страницы Vue нет намеренно: приложение `examples/viewer` написано на
React, и интеграция Vue/Svelte в него была бы лишней зависимостью демо.
Покрытие — vitest + `tsc --noEmit`.

## Ссылки

- Корневой [README](https://github.com/meerrec/wasm-converter#readme) — установка, быстрый старт, бюджеты.
