import { writable, type Writable } from 'svelte/store';
import { createXlsxViewer, type XlsxViewerHandle, type XlsxViewerOptions } from '@meerrec/core';

/** Store вьюера: подписка на ручку + императивные `attach`/`destroy`. */
export interface XlsxViewerStore {
  /** Подписка на ручку вьюера (`null` — вьюер не создан или уничтожен). */
  subscribe: Writable<XlsxViewerHandle | null>['subscribe'];
  /**
   * Создать вьюер на хосте. Повторный вызов без `destroy` уничтожает прежнюю
   * ручку и поднимает новую; поздний ответ от старого хоста не затмит новый.
   */
  attach(host: HTMLElement): Promise<void>;
  /** Уничтожить вьюер и сбросить store. */
  destroy(): void;
}

/**
 * Store, который держит ручку вьюера XLSX отдельно от компонента — для
 * императивного сценария, где вьюером управляют вне Svelte-разметки
 * (компонент `XlsxViewer` хранит ручку внутри себя).
 *
 * @param options — настройки вьюера.
 */
export function createViewerStore(options?: XlsxViewerOptions): XlsxViewerStore {
  const { subscribe, set } = writable<XlsxViewerHandle | null>(null);
  let current: XlsxViewerHandle | null = null;
  /** Хост последнего `attach`; по нему отбрасываются запоздалые ответы. */
  let host: HTMLElement | null = null;

  async function attach(el: HTMLElement): Promise<void> {
    host = el;
    const handle = await createXlsxViewer(el, options);
    if (host !== el) {
      handle.destroy();
      return;
    }
    current?.destroy();
    current = handle;
    set(handle);
  }

  function destroy(): void {
    host = null;
    current?.destroy();
    current = null;
    set(null);
  }

  return { subscribe, attach, destroy };
}
