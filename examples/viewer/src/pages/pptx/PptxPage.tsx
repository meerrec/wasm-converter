import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import { createPptxViewer, type PptxViewerHandle } from '@meerrec/core';
import { clearViewer, publishViewer } from '../../lib/bridge.js';
import { downloadBlob, exportFileName } from '../../lib/download.js';
import { fetchFixture } from '../../lib/fixtures.js';
import {
  pptxWorkerUrl,
  resolvePptxPdfWasm,
  resolvePptxWasm,
} from '../../lib/viewer_urls.js';

/**
 * Состояние страницы PPTX: вьюер, навигация по слайдам, зум и экспорт.
 *
 * В отличие от страниц DOCX/XLSX, лента здесь минимальна: страница открывает
 * фикстуру сразу (детерминированный путь для e2e) и даёт кнопки навигации,
 * зума и трёх экспортов. Контракт id: `#viewer`, `#slide-info`, `#slide-prev`,
 * `#slide-next`, `#zoom`, `#export-pdf`, `#export-png`, `#export-text`.
 */
export function usePptxPage(): {
  hostRef: React.RefObject<HTMLDivElement | null>;
  ready: boolean;
  error: string;
  slideIndex: number;
  slideCount: number;
  zoom: number;
  busy: 'pdf' | 'png' | 'text' | null;
  showPrev(): void;
  showNext(): void;
  setZoom(zoom: number): void;
  exportPdf(): Promise<void>;
  exportPng(): Promise<void>;
  exportText(): Promise<void>;
} {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewerRef = useRef<PptxViewerHandle | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState('');
  const [slideIndex, setSlideIndex] = useState(0);
  const [slideCount, setSlideCount] = useState(0);
  const [zoom, setZoomState] = useState(1);
  const [busy, setBusy] = useState<'pdf' | 'png' | 'text' | null>(null);

  useEffect(() => {
    let cancelled = false;
    const run = async (): Promise<void> => {
      const host = hostRef.current;
      if (!host) return;
      try {
        const viewer = await createPptxViewer(host, {
          workerUrl: pptxWorkerUrl,
          pptxWasm: await resolvePptxWasm(),
          pptxPdfWasm: await resolvePptxPdfWasm(),
        });
        if (cancelled) {
          viewer.destroy();
          return;
        }
        viewerRef.current = viewer;
        publishViewer({ kind: 'pptx', viewer });
        const slides = await viewer.open(
          await fetchFixture('pptx', 'simple/two_slides.pptx'),
        );
        setSlideCount(slides.length);
        setSlideIndex(0);
        setReady(true);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    };
    void run();
    return () => {
      cancelled = true;
      clearViewer();
      viewerRef.current?.destroy();
      viewerRef.current = null;
    };
  }, []);

  const show = useCallback((index: number) => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    viewer.showSlide(index);
    setSlideIndex(viewer.slideIndex);
  }, []);

  const setZoom = useCallback((next: number) => {
    viewerRef.current?.setZoom(next);
    setZoomState(viewerRef.current?.zoom ?? next);
  }, []);

  const exportPdf = useCallback(async () => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    setBusy('pdf');
    try {
      const bytes = await viewer.exportPdf();
      downloadBlob(
        new Uint8Array(bytes) as unknown as BlobPart,
        exportFileName('pptx', 'pdf'),
        'application/pdf',
      );
    } finally {
      setBusy(null);
    }
  }, []);

  const exportPng = useCallback(async () => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    setBusy('png');
    try {
      const bytes = await viewer.exportPng();
      downloadBlob(bytes as unknown as BlobPart, exportFileName('pptx', 'png'), 'image/png');
    } finally {
      setBusy(null);
    }
  }, []);

  const exportText = useCallback(async () => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    setBusy('text');
    try {
      const text = await viewer.exportMarkdown();
      downloadBlob(
        new TextEncoder().encode(text) as unknown as BlobPart,
        exportFileName('pptx', 'md'),
        'text/markdown',
      );
    } finally {
      setBusy(null);
    }
  }, []);

  return {
    hostRef,
    ready,
    error,
    slideIndex,
    slideCount,
    zoom,
    busy,
    showPrev: () => show(slideIndex - 1),
    showNext: () => show(slideIndex + 1),
    setZoom,
    exportPdf,
    exportPng,
    exportText,
  };
}

/**
 * Страница PPTX («Презентация»): холст вьюера и панель управления.
 */
export default function PptxPage(): ReactElement {
  const page = usePptxPage();
  return (
    <div className="pptx-page">
      <div className="pptx-toolbar">
        <button id="slide-prev" type="button" onClick={page.showPrev} disabled={page.slideIndex <= 0}>
          ←
        </button>
        <span id="slide-info">
          {page.slideCount === 0 ? '—' : `${page.slideIndex + 1} / ${page.slideCount}`}
        </span>
        <button
          id="slide-next"
          type="button"
          onClick={page.showNext}
          disabled={page.slideIndex + 1 >= page.slideCount}
        >
          →
        </button>
        <label>
          Масштаб
          <select
            id="zoom"
            value={String(page.zoom)}
            onChange={(ev) => page.setZoom(Number(ev.target.value))}
          >
            {['0.5', '0.75', '1', '1.5', '2'].map((value) => (
              <option key={value} value={value}>
                {Math.round(Number(value) * 100)} %
              </option>
            ))}
          </select>
        </label>
        <button id="export-pdf" type="button" onClick={() => void page.exportPdf()} disabled={page.busy !== null || !page.ready}>
          PDF
        </button>
        <button id="export-png" type="button" onClick={() => void page.exportPng()} disabled={page.busy !== null || !page.ready}>
          PNG
        </button>
        <button id="export-text" type="button" onClick={() => void page.exportText()} disabled={page.busy !== null || !page.ready}>
          Текст
        </button>
        {page.error !== '' && <span id="status">{page.error}</span>}
      </div>
      <div id="viewer" ref={page.hostRef} />
    </div>
  );
}
