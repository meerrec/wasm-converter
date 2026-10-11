import { useEffect, useRef, useState, type RefObject } from 'react';

/** Ручка вьюера — всё, что нужно хуку, чтобы убрать её за собой. */
export interface Destroyable {
  destroy(): void;
}

export interface UseViewerResult<H extends Destroyable> {
  /** Хост вьюера: его вешают на элемент страницы (`<main ref={hostRef} />`). */
  hostRef: RefObject<HTMLElement | null>;
  /** Готовая ручка; `null`, пока вьюер не создан. */
  viewer: H | null;
  /** Причина, по которой вьюер не поднялся, — для строки состояния. */
  error: string | null;
}

/**
 * Создаёт императивный вьюер ровно один раз за жизнь компонента и уничтожает
 * его при размонтировании.
 *
 * Создание отложено макротактом (`setTimeout(…, 0)`): в dev StrictMode
 * монтирует эффекты дважды — сразу за монтированием идёт размонтирование и
 * второе монтирование. Подними вьюер в теле эффекта, на один компонент
 * пришлось бы два воркера и две инициализации wasm, причём первый остался бы
 * жить: уничтожать его в cleanup ещё рано, вьюер не успел появиться. С
 * отложенным стартом первый эффект снимает свой таймер в cleanup, и воркер
 * поднимается один раз.
 *
 * @param create — запуск вьюера на готовом хосте.
 * @param onReady — вызывается с готовой ручкой: подписки, первичная загрузка.
 */
export function useViewer<H extends Destroyable>(
  create: (host: HTMLElement) => Promise<H>,
  onReady?: (handle: H) => void,
): UseViewerResult<H> {
  const hostRef = useRef<HTMLElement | null>(null);
  const [viewer, setViewer] = useState<H | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    /** Ручка, созданная к моменту размонтирования; за ней и убираем. */
    let created: H | null = null;

    const timer = setTimeout(() => {
      if (cancelled) return;
      // Хост рисует страница: адреса `#viewer` хватило бы только на то, чтобы
      // потерять связь с элементом, который React позже пересоздаст.
      const host = hostRef.current;
      if (!host) {
        setError('в разметке нет элемента #viewer');
        return;
      }
      void create(host)
        .then((handle) => {
          if (cancelled) {
            // Промис разрешился уже после размонтирования: ручку никто не
            // увидит, а без `destroy` здесь остался бы живой воркер и висячий
            // `#viewer > div` до перезагрузки страницы.
            handle.destroy();
            return;
          }
          created = handle;
          setViewer(handle);
          onReady?.(handle);
        })
        .catch((e: unknown) => {
          if (cancelled) return;
          setError(e instanceof Error ? e.message : String(e));
        });
    }, 0);

    return () => {
      cancelled = true;
      clearTimeout(timer);
      created?.destroy();
    };
    // Пустые зависимости намеренно: вьюер один на жизнь компонента, а не по
    // одному на каждую смену пропсов `create`/`onReady`.
  }, []);

  return { hostRef, viewer, error };
}
