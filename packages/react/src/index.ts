import { createElement, useRef, type ReactElement, type RefObject } from 'react';
import { createDocxViewer, createXlsxViewer } from '@meerrec/core';
import type {
  DocxViewerHandle,
  DocxViewerOptions,
  XlsxViewerHandle,
  XlsxViewerOptions,
} from '@meerrec/core';
import { useFormatViewer } from './use_viewer.js';

export type {
  XlsxViewerHandle,
  XlsxViewerOptions,
  DocxViewerHandle,
  DocxViewerOptions,
} from '@meerrec/core';

/** Пропсы {@link XlsxViewer}. */
export interface XlsxViewerProps {
  /**
   * Хост-элемент, в который монтируется вьюер; не задан — компонент создаёт
   * собственный div. Хост обязан иметь размеры: фабрика кладёт в него
   * прокрутку и холст на 100 % ширины и высоты.
   */
  hostRef?: RefObject<HTMLDivElement | null>;
  /** Настройки вьюера — см. `XlsxViewerOptions` в `@meerrec/core`. */
  options?: XlsxViewerOptions;
}

/** Пропсы {@link DocxViewer}. */
export interface DocxViewerProps {
  /**
   * Хост-элемент, в который монтируется вьюер; не задан — компонент создаёт
   * собственный div. Хост обязан иметь размеры: фабрика кладёт в него
   * прокрутку и холст на 100 % ширины и высоты.
   */
  hostRef?: RefObject<HTMLDivElement | null>;
  /** Настройки вьюера — см. `DocxViewerOptions` в `@meerrec/core`. */
  options?: DocxViewerOptions;
}

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
  return useFormatViewer(hostRef, createXlsxViewer, options);
}

/**
 * Хук вьюера DOCX: создаёт `createDocxViewer` на хосте и уничтожает его в
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
export function useDocxViewer(
  hostRef: RefObject<HTMLElement | null>,
  options?: DocxViewerOptions,
): DocxViewerHandle | null {
  return useFormatViewer(hostRef, createDocxViewer, options);
}

/** Пропсы общего {@link FormatViewer}. */
interface FormatViewerProps<O> {
  /**
   * Хост-элемент, в который монтируется вьюер; не задан — компонент создаёт
   * собственный div.
   */
  hostRef?: RefObject<HTMLDivElement | null> | undefined;
  /** Настройки вьюера; прозрачно уходят в фабрику. */
  options?: O | undefined;
  /** Фабрика формата: запуск вьюера на готовом хосте с настройками. */
  create: (host: HTMLElement, opts: O | undefined) => Promise<{ destroy(): void }>;
}

/**
 * Общий компонент тонкой обёртки над фабрикой `@meerrec/core` для любого
 * формата: рендерит только host-div и подключает {@link useFormatViewer}.
 * Всю разметку, стили и поведение делает фабрика формата.
 */
function FormatViewer<O>(props: FormatViewerProps<O>): ReactElement {
  const innerRef = useRef<HTMLDivElement | null>(null);
  const hostRef = props.hostRef ?? innerRef;
  useFormatViewer(hostRef, props.create, props.options);
  return createElement('div', { ref: hostRef });
}

/**
 * Тонкая обёртка вьюера XLSX для React.
 *
 * Компонент намеренно «пустой»: он рендерит только host-div и отдаёт ручку
 * вьюера хуку {@link useXlsxViewer}. Всю разметку, стили и поведение делает
 * `createXlsxViewer` из `@meerrec/core`.
 *
 * @param props.hostRef — внешний хост; пропущен — используется внутренний.
 * @param props.options — настройки вьюера.
 */
export function XlsxViewer(props: XlsxViewerProps): ReactElement {
  return createElement(FormatViewer<XlsxViewerOptions>, {
    ...(props.hostRef !== undefined ? { hostRef: props.hostRef } : {}),
    ...(props.options !== undefined ? { options: props.options } : {}),
    create: createXlsxViewer,
  });
}

/**
 * Тонкая обёртка вьюера DOCX для React.
 *
 * Компонент намеренно «пустой»: он рендерит только host-div и отдаёт ручку
 * вьюера хуку {@link useDocxViewer}. Всю разметку, стили и поведение делает
 * `createDocxViewer` из `@meerrec/core`.
 *
 * @param props.hostRef — внешний хост; пропущен — используется внутренний.
 * @param props.options — настройки вьюера.
 */
export function DocxViewer(props: DocxViewerProps): ReactElement {
  return createElement(FormatViewer<DocxViewerOptions>, {
    ...(props.hostRef !== undefined ? { hostRef: props.hostRef } : {}),
    ...(props.options !== undefined ? { options: props.options } : {}),
    create: createDocxViewer,
  });
}
