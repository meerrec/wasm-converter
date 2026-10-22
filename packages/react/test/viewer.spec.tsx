import { render, waitFor } from '@testing-library/react';
import { useRef, type ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  DocxViewerHandle,
  DocxViewerOptions,
  XlsxViewerHandle,
  XlsxViewerOptions,
} from '@meerrec/core';

// Мок фабрик core: обёртки не трогают байты и воркеры, поэтому проверяется
// только жизненный цикл — создание, проброс опций и destroy.
const mocks = vi.hoisted(() => ({
  createXlsxViewer: vi.fn(),
  createDocxViewer: vi.fn(),
}));

vi.mock('@meerrec/core', () => ({
  createXlsxViewer: mocks.createXlsxViewer,
  createDocxViewer: mocks.createDocxViewer,
}));

import { DocxViewer, XlsxViewer, useDocxViewer, useXlsxViewer } from '../src/index.js';

function fakeXlsxHandle(): XlsxViewerHandle {
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

function fakeDocxHandle(): DocxViewerHandle {
  return {
    transport: 'bytes',
    renderMode: 'main',
    pages: [],
    pageCount: 0,
    pageIndex: 0,
    zoom: 1,
    open: vi.fn(),
    showPage: vi.fn(),
    setZoom: vi.fn(),
    scrollTo: vi.fn(),
    exportPng: vi.fn(),
    exportPdf: vi.fn(),
    exportMarkdown: vi.fn(),
    exportHtml: vi.fn(),
    exportText: vi.fn(),
    getTextLayer: vi.fn(),
    getFullTextLayer: vi.fn(),
    onTick: vi.fn(),
    destroy: vi.fn(),
  };
}

describe('@meerrec/react', () => {
  beforeEach(() => {
    mocks.createXlsxViewer.mockReset();
    mocks.createDocxViewer.mockReset();
  });

  it('XlsxViewer монтирует вьюер в host-div и уничтожает при размонтировании', async () => {
    const handle = fakeXlsxHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);
    const options: XlsxViewerOptions = { zoom: 2 };
    const { unmount } = render(<XlsxViewer options={options} />);

    await waitFor(() => expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1));
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

  it('XlsxViewer пересоздаёт вьюер при смене идентичности options', async () => {
    const first = fakeXlsxHandle();
    const second = fakeXlsxHandle();
    mocks.createXlsxViewer.mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    const optionsA: XlsxViewerOptions = { zoom: 1 };
    const optionsB: XlsxViewerOptions = { zoom: 3 };
    const { rerender } = render(<XlsxViewer options={optionsA} />);

    await waitFor(() => expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1));
    rerender(<XlsxViewer options={optionsB} />);

    await waitFor(() => expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(2));
    expect(first.destroy).toHaveBeenCalledTimes(1);
    const [, passed] = mocks.createXlsxViewer.mock.calls[1] as [
      HTMLElement,
      XlsxViewerOptions,
    ];
    expect(passed).toBe(optionsB);
  });

  it('useXlsxViewer отдаёт готовую ручку (null до создания)', async () => {
    const handle = fakeXlsxHandle();
    mocks.createXlsxViewer.mockResolvedValue(handle);
    let seen: XlsxViewerHandle | null = null;

    function Probe(): ReactElement {
      const hostRef = useRef<HTMLDivElement | null>(null);
      const viewer = useXlsxViewer(hostRef, {});
      seen = viewer;
      return <div ref={hostRef} />;
    }

    render(<Probe />);
    expect(seen).toBeNull();
    await waitFor(() => expect(seen).toBe(handle));
  });

  it('DocxViewer монтирует вьюер и уничтожает при размонтировании', async () => {
    const handle = fakeDocxHandle();
    mocks.createDocxViewer.mockResolvedValue(handle);
    const options: DocxViewerOptions = { pageGap: 24 };
    const { unmount } = render(<DocxViewer options={options} />);

    await waitFor(() => expect(mocks.createDocxViewer).toHaveBeenCalledTimes(1));
    const [host, passed] = mocks.createDocxViewer.mock.calls[0] as [
      HTMLElement,
      DocxViewerOptions,
    ];
    expect(host).toBeInstanceOf(HTMLElement);
    expect(passed).toBe(options);

    unmount();
    expect(handle.destroy).toHaveBeenCalledTimes(1);
  });

  it('useDocxViewer отдаёт готовую ручку', async () => {
    const handle = fakeDocxHandle();
    mocks.createDocxViewer.mockResolvedValue(handle);
    let seen: DocxViewerHandle | null = null;

    function Probe(): ReactElement {
      const hostRef = useRef<HTMLDivElement | null>(null);
      const viewer = useDocxViewer(hostRef, {});
      seen = viewer;
      return <div ref={hostRef} />;
    }

    render(<Probe />);
    await waitFor(() => expect(seen).toBe(handle));
  });

  it('размонтирование до создания не роняет незавершённый промис', async () => {
    const handle = fakeXlsxHandle();
    let resolveCreate!: (handle: XlsxViewerHandle) => void;
    mocks.createXlsxViewer.mockImplementation(
      () => new Promise<XlsxViewerHandle>((resolve) => (resolveCreate = resolve)),
    );
    const { unmount } = render(<XlsxViewer />);

    await waitFor(() => expect(mocks.createXlsxViewer).toHaveBeenCalledTimes(1));
    unmount();
    resolveCreate(handle);
    await waitFor(() => expect(handle.destroy).toHaveBeenCalledTimes(1));
  });
});
