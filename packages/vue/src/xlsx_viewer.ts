import { defineComponent, h, ref, type PropType } from 'vue';
import { useXlsxViewer } from './use_xlsx_viewer.js';
import type { XlsxViewerOptions } from '@meerrec/core';

/**
 * Тонкая обёртка вьюера XLSX для Vue.
 *
 * Компонент собран через `defineComponent` с render-функцией (без SFC, чтобы
 * сборка оставалась tsup-простой) и намеренно «пустой»: рендерит только
 * host-div и отдаёт ручку вьюера composable {@link useXlsxViewer}. Всю
 * разметку, стили и поведение делает `createXlsxViewer` из `@meerrec/core`.
 *
 * Хост обязан иметь размеры: фабрика кладёт в него прокрутку и холст на 100 %
 * ширины и высоты.
 *
 * Через `expose` наружу отдаётся `viewer` (shallowRef с ручкой вьюера) —
 * родитель может забрать его `template ref`-ом.
 */
export const XlsxViewer = defineComponent({
  name: 'MeerrecXlsxViewer',
  props: {
    /** Настройки вьюера — см. `XlsxViewerOptions` в `@meerrec/core`. */
    options: {
      type: Object as PropType<XlsxViewerOptions>,
      required: false,
      default: undefined,
    },
  },
  setup(props, { expose }) {
    const hostRef = ref<HTMLElement | null>(null);
    const viewer = useXlsxViewer(hostRef, props.options);
    expose({ viewer });
    return () => h('div', { ref: hostRef });
  },
});
