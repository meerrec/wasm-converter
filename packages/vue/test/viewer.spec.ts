import { flushPromises, mount } from '@vue/test-utils';
import { defineComponent, h, ref } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { XlsxViewerHandle, XlsxViewerOptions } from '@meerrec/core';

// Мок фабрики core: обёртка не трогает байты и воркеры, поэтому проверяется
// только жизненный цикл — создание в onMounted, проброс опций и destroy.
const mocks = vi.hoisted(() => ({
  createXlsxViewer: vi.fn(),
}));

vi.mock('@meerrec/core', () => ({
  createXlsxViewer: mocks.createXlsxViewer,
}));

import { XlsxViewer, useXlsxViewer } from '../src/index.js';

function fakeHandle(): XlsxViewerHandle {
  return {
    transport: 'bytes',
    renderMode: 'main',
    sheets: [],
    sheetIndex: 0,
    zoom: 1,
    open: vi.fn(),
    showSheet: vi.fn(),
    setZoom: vi.fn(),
    scrollTo: vi.fn(),
    setView: vi.fn(),
    hitTest: vi.fn(),
    hyperlinkAt: vi.fn(),
    setCellText: vi.fn(),
    exportPng: vi.fn(),
    exportPdf: vi.fn(),
    onTick: vi.fn(),
    destroy: vi.fn(),
  };
}

describe('@meerrec/vue', () => {
  beforeEach(() => {
    mocks.createXlsxViewer.mockReset();
  });

  it('XlsxViewer создаёт вьюер в host-div и уничтожает при размонтировании', async () => {
    const handle = fakeHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);
    const options: XlsxViewerOptions = { zoom: 2 };
    const wrapper = mount(XlsxViewer, { props: { options } });

    await flushPromises();
    expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1);
    const [host, passed] = mocks.createXlsxViewer.mock.calls[0] as [
      HTMLElement,
      XlsxViewerOptions,
    ];
    expect(host).toBeInstanceOf(HTMLElement);
    expect(host.tagName).toBe('DIV');
    // Vue оборачивает пропсы в реактивный прокси — сравнение по содержимому.
    expect(passed).toStrictEqual(options);

    wrapper.unmount();
    expect(handle.destroy).toHaveBeenCalledTimes(1);
  });

  it('composable отдаёт ручку после монтирования (null до создания)', async () => {
    const handle = fakeHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);

    const Probe = defineComponent({
      setup() {
        const hostRef = ref<HTMLDivElement | null>(null);
        const viewer = useXlsxViewer(hostRef, {});
        return () => h('div', { ref: hostRef }, viewer.value === null ? 'null' : 'ready');
      },
    });

    const wrapper = mount(Probe);
    expect(wrapper.text()).toBe('null');
    await flushPromises();
    expect(wrapper.text()).toBe('ready');
  });

  it('компонент отдаёт ручку через expose', async () => {
    const handle = fakeHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);

    const wrapper = mount(XlsxViewer);
    await flushPromises();
    // Публичный прокси инстанса разворачивает ref из `expose`: `viewer` — ручка.
    const exposed = wrapper.vm as unknown as { viewer: XlsxViewerHandle | null };
    expect(exposed.viewer).toBe(handle);
  });

  it('размонтирование до создания не оставляет воркер без владельца', async () => {
    const handle = fakeHandle();
    let resolveCreate!: (handle: XlsxViewerHandle) => void;
    mocks.createXlsxViewer.mockImplementation(
      () => new Promise<XlsxViewerHandle>((resolve) => (resolveCreate = resolve)),
    );

    const wrapper = mount(XlsxViewer);
    expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1);
    wrapper.unmount();
    resolveCreate(handle);
    await flushPromises();
    expect(handle.destroy).toHaveBeenCalledTimes(1);
  });
});
