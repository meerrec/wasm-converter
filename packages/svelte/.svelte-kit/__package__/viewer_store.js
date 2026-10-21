import { writable } from 'svelte/store';
import { createXlsxViewer } from '@meerrec/core';
/**
 * Store, который держит ручку вьюера XLSX отдельно от компонента — для
 * императивного сценария, где вьюером управляют вне Svelte-разметки
 * (компонент `XlsxViewer` хранит ручку внутри себя).
 *
 * @param options — настройки вьюера.
 */
export function createViewerStore(options) {
    const { subscribe, set } = writable(null);
    let current = null;
    /** Хост последнего `attach`; по нему отбрасываются запоздалые ответы. */
    let host = null;
    async function attach(el) {
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
    function destroy() {
        host = null;
        current?.destroy();
        current = null;
        set(null);
    }
    return { subscribe, attach, destroy };
}
