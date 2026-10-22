export type {
  FrameTransport,
  RenderMode,
  ReadyInfo,
  Viewport,
  RenderConfig,
  PdfModuleUrls,
  DocxModuleUrls,
  PptxModuleUrls,
  PdfOptions,
  HitResult,
  CellRef,
  TextPos,
  PaintStats,
  SheetInfo,
  PageInfo,
  SlideInfo,
  HyperlinkInfo,
  TextLayer,
  TextRun,
} from './protocol.js';

export {
  supportsOffscreen,
  assertFallbackAvailable,
  type ViewerHandle,
  type ViewerOptions,
} from './render/offscreen';

export {
  createXlsxViewer,
  type XlsxViewerHandle,
  type XlsxViewerOptions,
} from './render/xlsx_viewer.js';

export {
  createDocxViewer,
  type DocxViewerHandle,
  type DocxViewerOptions,
} from './render/docx_viewer.js';

export {
  createPptxViewer,
  type PptxViewerHandle,
  type PptxViewerOptions,
} from './render/pptx_viewer.js';

export const VERSION = '0.1.0';
