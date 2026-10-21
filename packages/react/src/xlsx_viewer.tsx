import { useRef, type ReactElement, type RefObject } from 'react';
import { useXlsxViewer } from './use_xlsx_viewer.js';
import type { XlsxViewerOptions } from '@meerrec/core';

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
  const innerRef = useRef<HTMLDivElement | null>(null);
  const hostRef = props.hostRef ?? innerRef;
  useXlsxViewer(hostRef, props.options);
  return <div ref={hostRef} />;
}
