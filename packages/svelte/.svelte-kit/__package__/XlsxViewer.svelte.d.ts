import { type XlsxViewerOptions } from '@meerrec/core';
interface $$__sveltets_2_IsomorphicComponent<Props extends Record<string, any> = any, Events extends Record<string, any> = any, Slots extends Record<string, any> = any, Exports = {}, Bindings = string> {
    new (options: import('svelte').ComponentConstructorOptions<Props>): import('svelte').SvelteComponent<Props, Events, Slots> & {
        $$bindings?: Bindings;
    } & Exports;
    (internal: unknown, props: Props & {
        $$events?: Events;
        $$slots?: Slots;
    }): Exports & {
        $set?: any;
        $on?: any;
    };
    z_$$bindings?: Bindings;
}
declare const XlsxViewer: $$__sveltets_2_IsomorphicComponent<{
    /** Настройки вьюера — см. `XlsxViewerOptions` в `@meerrec/core`. */ options?: XlsxViewerOptions | undefined;
}, {
    [evt: string]: CustomEvent<any>;
}, {}, {}, string>;
type XlsxViewer = InstanceType<typeof XlsxViewer>;
export default XlsxViewer;
