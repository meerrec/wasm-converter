// Стабильный TS-контракт формата: дескриптор вьюера и запуск воркера.
//
// Новый формат не должен править `viewer_core`/`worker_core`: он приносит
// `PluginDescriptor` (метаданные + `ViewerSpec`) и тонкий воркер поверх
// `startPluginWorker`. Здесь только склейка; сам формат живёт в своём модуле и
// своём wasm-пакете.
//
// `unit` — единица навигации, а не тип модели: ODT отдаёт страницы, ODS и XLSB —
// листы, ODP — слайды. По ней интерфейс выбирает форму прокрутки и подписи, а
// `DocFormat` остаётся явным идентификатором для протокола.

import type { DocFormat } from './protocol.js';
import {
  createViewerBase,
  type ViewerBaseHandle,
  type ViewerBaseOptions,
  type ViewerSpec,
} from './render/viewer_core.js';

/** Единица навигации формата: лист, страница или слайд. */
export type PluginUnit = 'sheets' | 'pages' | 'slides';

/** Описание формата для общего каркаса просмотрщика. */
export interface PluginDescriptor<S, O extends ViewerBaseOptions> {
  /** Формат в конверте `open`/`open-stream`. */
  readonly format: DocFormat;
  /** Что считает документ: листы, страницы или слайды. */
  readonly unit: PluginUnit;
  /**
   * Собрать `ViewerSpec` под конкретные опции.
   *
   * Спецификация знает URL воркера, теги ответов, раскладку и форматные
   * операции; ядро не ветвится по `format`.
   */
  spec(options: O): ViewerSpec<S, O>;
}

/**
 * Создать вьюер по дескриптору: тот же `createViewerBase`, что у встроенных
 * форматов, поэтому поведение (транспорт, main-режим, запросы, destroy) не
 * дублируется.
 */
export function createPluginViewer<S, O extends ViewerBaseOptions>(
  descriptor: PluginDescriptor<S, O>,
  host: HTMLElement,
  options: O,
): Promise<ViewerBaseHandle<S, O>> {
  return createViewerBase(host, descriptor.spec(options), options);
}
