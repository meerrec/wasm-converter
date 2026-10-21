import { onMounted, onUnmounted, shallowRef, type Ref, type ShallowRef } from 'vue';
import { createXlsxViewer, type XlsxViewerHandle, type XlsxViewerOptions } from '@meerrec/core';

/**
 * Composable вьюера XLSX для Vue: создаёт `createXlsxViewer` в `onMounted` и
 * уничтожает его в `onUnmounted`.
 *
 * Отклонение от плана T4.2: фабрика `@meerrec/core` принимает хост-элемент
 * (`HTMLElement`), в который сама монтирует область прокрутки и холст, —
 * поэтому composable держится за хост, а не за `<canvas>`. При отсутствии
 * `OffscreenCanvas` фабрика сама переходит в main-режим
 * (`renderMode: 'auto'` по умолчанию).
 *
 * @param hostRef — хост (обычно div), занимающий всю отведённую вьюеру площадь.
 * @param options — настройки вьюера; читаются в момент монтирования.
 * @returns shallowRef с ручкой вьюера; `null`, пока вьюер не создан
 * (или если хоста на момент монтирования нет).
 */
export function useXlsxViewer(
  hostRef: Ref<HTMLElement | null>,
  options?: XlsxViewerOptions,
): Readonly<ShallowRef<XlsxViewerHandle | null>> {
  const viewer = shallowRef<XlsxViewerHandle | null>(null);
  let disposed = false;

  onMounted(() => {
    const host = hostRef.value;
    if (!host) return;
    void createXlsxViewer(host, options)
      .then((handle) => {
        if (disposed) {
          // Компонент размонтирован, пока фабрика поднимала воркер: ручку
          // никто не увидит — сразу уничтожаем.
          handle.destroy();
          return;
        }
        viewer.value = handle;
      })
      .catch((e: unknown) => {
        if (disposed) return;
        console.error('[meerrec/vue]', e instanceof Error ? e.message : String(e));
      });
  });

  onUnmounted(() => {
    disposed = true;
    viewer.value?.destroy();
    viewer.value = null;
  });

  return viewer;
}
