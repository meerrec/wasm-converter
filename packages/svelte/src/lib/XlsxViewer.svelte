<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { createXlsxViewer, type XlsxViewerHandle, type XlsxViewerOptions } from '@meerrec/core';

  /** Настройки вьюера — см. `XlsxViewerOptions` в `@meerrec/core`. */
  export let options: XlsxViewerOptions | undefined = undefined;

  /** Хост, в который фабрика монтирует прокрутку и холст. */
  let host: HTMLDivElement | null = null;
  let viewer: XlsxViewerHandle | null = null;
  let disposed = false;

  onMount(() => {
    if (!host) return;
    void createXlsxViewer(host, options)
      .then((handle) => {
        if (disposed) {
          // Компонент размонтирован, пока фабрика поднимала воркер: ручку
          // никто не увидит — сразу уничтожаем.
          handle.destroy();
          return;
        }
        viewer = handle;
      })
      .catch((e: unknown) => {
        if (disposed) return;
        console.error('[meerrec/svelte]', e instanceof Error ? e.message : String(e));
      });
  });

  onDestroy(() => {
    disposed = true;
    viewer?.destroy();
    viewer = null;
  });
</script>

<!-- Хост обязан иметь размеры: фабрика кладёт в него прокрутку и холст на
     100 % ширины и высоты. -->
<div bind:this={host}></div>
