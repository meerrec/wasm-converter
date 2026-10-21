import { render } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { XlsxViewerHandle, XlsxViewerOptions } from '@meerrec/core';

// Мок фабрики core: обёртки не трогают байты и воркеры, поэтому проверяется
// только жизненный цикл — создание, проброс опций и destroy.
const mocks = vi.hoisted(() => ({
  createXlsxViewer: vi.fn(),
}));

vi.mock('@meerrec/core', () => ({
  createXlsxViewer: mocks.createXlsxViewer,
}));

import XlsxViewer from '../src/lib/XlsxViewer.svelte';
import { createViewerStore } from '../src/lib/viewer_store.js';

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
    exportPng: vi.fn(),
    exportPdf: vi.fn(),
    onTick: vi.fn(),
    destroy: vi.fn(),
  };
}

describe('@meerrec/svelte', () => {
  beforeEach(() => {
    mocks.createXlsxViewer.mockReset();
  });

  it('XlsxViewer создаёт вьюер в host-div и уничтожает при размонтировании', async () => {
    const handle = fakeHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);
    const options: XlsxViewerOptions = { zoom: 2 };

    const { unmount } = render(XlsxViewer, { props: { options } });
    await vi.waitFor(() => expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1));
    const [host, passed] = mocks.createXlsxViewer.mock.calls[0] as [
      HTMLElement,
      XlsxViewerOptions,
    ];
    expect(host).toBeInstanceOf(HTMLElement);
    expect(host.tagName).toBe('DIV');
    expect(passed).toBe(options);

    unmount();
    expect(handle.destroy).toHaveBeenCalledTimes(1);
  });

  it('createViewerStore: attach создаёт вьюер, destroy убирает его', async () => {
    const handle = fakeHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);
    const store = createViewerStore({ zoom: 3 });
    const host = document.createElement('div');

    await store.attach(host);
    expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1);
    expect(get(store)).toBe(handle);

    store.destroy();
    expect(handle.destroy).toHaveBeenCalledTimes(1);
    expect(get(store)).toBeNull();
  });

  it('createViewerStore: повторный attach уничтожает прежнюю ручку', async () => {
    const first = fakeHandle();
    const second = fakeHandle();
    mocks.createXlsxViewer.mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    const store = createViewerStore();

    await store.attach(document.createElement('div'));
    await store.attach(document.createElement('div'));
    expect(first.destroy).toHaveBeenCalledTimes(1);
    expect(get(store)).toBe(second);
  });
});
