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
} from './protocol.js';

export { createRpc, type RpcHandle } from './rpc.js';
export {
  supportsOffscreen,
  assertFallbackAvailable,
  type ViewerHandle,
  type ViewerOptions,
} from './render/offscreen';

export const VERSION = '0.1.0';
