import { useRef, type ReactElement, type RefObject } from 'react';
import { useDocxViewer } from './use_docx_viewer.js';
import type { DocxViewerOptions } from '@meerrec/core';

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
  const innerRef = useRef<HTMLDivElement | null>(null);
  const hostRef = props.hostRef ?? innerRef;
  useDocxViewer(hostRef, props.options);
  return <div ref={hostRef} />;
}
