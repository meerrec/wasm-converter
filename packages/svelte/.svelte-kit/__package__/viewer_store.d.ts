import { type Writable } from 'svelte/store';
import { type XlsxViewerHandle, type XlsxViewerOptions } from '@meerrec/core';
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
export declare function createViewerStore(options?: XlsxViewerOptions): XlsxViewerStore;
