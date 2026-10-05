export type {
  WorkerRequest,
  WorkerResponse,
  Viewport,
  RenderConfig,
  PdfOptions,
  HitResult,
  CellRef,
  TextPos,
  FrameTimings,
  PaintStats,
  SheetInfo,
} from './protocol.js';

export { createRpc, type RpcHandle } from './rpc.js';
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

export const VERSION = '0.1.0';
