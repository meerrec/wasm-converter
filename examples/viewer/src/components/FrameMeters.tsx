import { useEffect, useRef, type ReactElement } from 'react';
import type { DocxViewerHandle, XlsxViewerHandle } from '@meerrec/core';

export interface FrameMetersProps {
  /** Ручка активного вьюера; `null` — вьюер ещё не создан. */
  handle: DocxViewerHandle | XlsxViewerHandle | null;
}

/** Записать значение в ячейку: её ещё может не быть на странице. */
function setText(element: HTMLElement | null, value: string): void {
  if (element) element.textContent = value;
}

/**
 * Метрики кадров в подвале: команд, сборка DisplayList, рисование, номер кадра.
 *
 * Числа меняются до 60 раз в секунду, поэтому пишутся прямо в DOM мимо
 * состояния React: `setState` на каждом кадре перерисовывал бы подвал, а с ним
 * и страницу, вхолостую. Компонент отдаёт фрагмент из четырёх `<span>`-ов —
 * рамку подвала и остальные его ячейки рисует страница.
 */
export function FrameMeters({ handle }: FrameMetersProps): ReactElement {
  const cmdsRef = useRef<HTMLElement | null>(null);
  const buildRef = useRef<HTMLElement | null>(null);
  const paintRef = useRef<HTMLElement | null>(null);
  const frameRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!handle) return undefined;
    return handle.onTick((stats) => {
      setText(cmdsRef.current, String(stats.cmds));
      // `buildMs` есть только у документов: у книги кадр собирается сразу в слот.
      setText(buildRef.current, `${(stats.buildMs ?? 0).toFixed(1)} мс`);
      setText(paintRef.current, `${stats.paintMs.toFixed(1)} мс`);
      setText(frameRef.current, String(stats.frameId));
    });
  }, [handle]);

  return (
    <>
      <span>команд: <b id="cmds" ref={cmdsRef}>0</b></span>
      <span>сборка: <b id="build" ref={buildRef}>—</b></span>
      <span>рисование: <b id="paint" ref={paintRef}>—</b></span>
      <span>кадр: <b id="frame" ref={frameRef}>0</b></span>
    </>
  );
}
