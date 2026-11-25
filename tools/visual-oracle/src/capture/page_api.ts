// Форма `window` внутри `page.evaluate`.
//
// Пакет собирается с `lib: ["ES2023"]` и `types: ["node"]`: DOM-типов здесь нет
// намеренно — оракул живёт в Node. Но код, который Playwright сериализует и
// исполняет в странице, видит настоящий `window`, поэтому его форма описана
// отдельно и узко: только те методы, которые сценарии действительно вызывают.
// Расширять этот интерфейс «на всякий случай» не нужно — каждая строка здесь
// обещание, что страница это умеет.

/** Ручка вьюера в странице: подмножество `DocxViewerHandle`/`XlsxViewerHandle`. */
export interface PageViewerHandle {
  readonly transport: 'sab' | 'bytes';
  readonly renderMode: 'worker' | 'main';
  readonly zoom: number;
  readonly pageCount?: number;
  readonly sheets?: readonly { index: number; name: string; hidden?: boolean }[];
  readonly sheetIndex?: number;
  showPage?(index: number): void;
  showSheet?(index: number): void;
  setZoom(zoom: number): void;
  scrollTo(x: number, y: number): void;
  setView?(options: { showGrid?: boolean; showHeaders?: boolean }): void;
  setSelection?(
    reference: string,
    options?: { add?: boolean; scrollIntoView?: boolean },
  ): Promise<void>;
  exportPng(): Promise<Uint8Array>;
  exportPdf(options?: { sheetIndex?: number }): Promise<Uint8Array>;
  onTick(handler: (stats: unknown) => void): () => void;
}

/** Мост страницы: `window.docConverter` из `examples/viewer/src/lib/bridge.ts`. */
export interface PageBridge {
  readonly kind: 'docx' | 'xlsx' | 'pptx';
  readonly viewer: PageViewerHandle;
}

/** Элемент, у которого сценарий читает значение или текст. */
export interface PageElement {
  readonly value?: string;
  readonly textContent: string | null;
}

/** Загруженный шрифт: имя семейства и состояние. */
export interface PageFontFace {
  readonly family: string;
  readonly status: string;
}

/** Набор шрифтов документа: перечисление и готовность загрузки. */
export interface PageFontSet extends Iterable<PageFontFace> {
  readonly ready: Promise<unknown>;
}

/** `window` в том объёме, в котором его трогают сценарии захвата. */
export interface PageWindow {
  docConverter?: PageBridge;
  devicePixelRatio: number;
  btoa(data: string): string;
  document: {
    fonts: PageFontSet;
    querySelector(selector: string): PageElement | null;
    querySelectorAll(selector: string): Iterable<PageElement>;
  };
  /** Отписка от тиков предыдущего ожидания. */
  __oracleUnsub?: () => void;
  /** Мост тиков в Node: его ставит `page.exposeBinding`. */
  __oracleTick?: () => void;
}
