import { useEffect, useState, type RefObject } from 'react';

/**
 * Общий жизненный цикл тонких обёрток над фабриками `@meerrec/core`.
 *
 * Создание отложено макротактом (`setTimeout(…, 0)`): в dev-режиме StrictMode
 * монтирует эффекты дважды — сразу за монтированием идёт размонтирование и
 * второе монтирование. Подними вьюер в теле эффекта, на один компонент
 * пришлось бы два воркера и две инициализации wasm. С отложенным стартом
 * первый эффект снимает свой таймер в cleanup, и воркер поднимается один раз.
 *
 * @param hostRef — хост, в который фабрика смонтирует прокрутку и холст.
 * @param create — запуск вьюера на готовом хосте.
 * @returns ручка вьюера; `null`, пока вьюер не создан (или если хоста нет).
 */
export function useViewerLifecycle<H extends { destroy(): void }>(
  hostRef: RefObject<HTMLElement | null>,
  create: (host: HTMLElement) => Promise<H>,
): H | null {
  const [viewer, setViewer] = useState<H | null>(null);

  useEffect(() => {
    let cancelled = false;
    /** Ручка, созданная к моменту размонтирования; за ней и убираем. */
    let created: H | null = null;

    const timer = setTimeout(() => {
      if (cancelled) return;
      const host = hostRef.current;
      if (!host) return;
      void create(host)
        .then((handle) => {
          if (cancelled) {
            // Промис разрешился уже после снятия эффекта: ручку никто не
            // увидит, а без `destroy` остался бы живой воркер без владельца.
            handle.destroy();
            return;
          }
          created = handle;
          setViewer(handle);
        })
        .catch((e: unknown) => {
          if (cancelled) return;
          console.error('[meerrec/react]', e instanceof Error ? e.message : String(e));
        });
    }, 0);

    return () => {
      cancelled = true;
      clearTimeout(timer);
      created?.destroy();
      setViewer(null);
    };
  }, [hostRef, create]);

  return viewer;
}
