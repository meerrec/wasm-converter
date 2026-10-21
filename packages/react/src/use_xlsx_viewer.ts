import { useCallback, type RefObject } from 'react';
import { createXlsxViewer, type XlsxViewerHandle, type XlsxViewerOptions } from '@meerrec/core';
import { useViewerLifecycle } from './use_viewer.js';

/**
 * Хук вьюера XLSX: создаёт `createXlsxViewer` на хосте и уничтожает его в
 * cleanup эффекта.
 *
 * Отклонение от плана T4.1: фабрика `@meerrec/core` принимает хост-элемент
 * (`HTMLElement`), в который сама монтирует область прокрутки и холст, —
 * поэтому хук держится за хост, а не за `<canvas>`. При отсутствии
 * `OffscreenCanvas` фабрика сама переходит в main-режим
 * (`renderMode: 'auto'` по умолчанию).
 *
 * @param hostRef — хост (обычно div), занимающий всю отведённую вьюеру площадь.
 * @param options — настройки вьюера; вьюер пересоздаётся при смене
 * идентичности объекта (новый объект = новые настройки).
 * @returns ручка вьюера; `null`, пока вьюер не создан (или если хоста нет).
 */
export function useXlsxViewer(
  hostRef: RefObject<HTMLElement | null>,
  options?: XlsxViewerOptions,
): XlsxViewerHandle | null {
  // `useCallback` с ключом `options`: новая идентичность настроек — новый
  // эффект, старый вьюер уничтожает cleanup.
  const create = useCallback((host: HTMLElement) => createXlsxViewer(host, options), [options]);
  return useViewerLifecycle(hostRef, create);
}
