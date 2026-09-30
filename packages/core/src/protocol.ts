// Wire-протокол Main ↔ Worker. Источник истины — этот файл;
// Rust-сторона описывает только payload (JSON / SAB), не конверт.

export interface Viewport { x: number; y: number; w: number; h: number; scale: number }

export interface RenderConfig {
  showGrid: boolean;
  showHeaders: boolean;
  theme: 'light' | 'dark';
  dpr: number;
}

export type CellRef = { kind: 'cell'; sheet: number; row: number; col: number };
export type TextPos = { kind: 'text'; page: number; offset: number };
export type HitResult = CellRef | TextPos;

export interface PdfOptions {
  pageSize?: 'A4' | 'A3' | 'Letter' | 'Legal';
  orientation?: 'portrait' | 'landscape';
  margins?: { top: number; right: number; bottom: number; left: number };
  scale?: number;
  sheetIndex?: number;
}

export type WorkerRequest =
  | { id: number; type: 'init';      payload: { canvas: OffscreenCanvas } }
  | { id: number; type: 'resize';    payload: { width: number; height: number; dpr: number } }
  | { id: number; type: 'open';      payload: { format: 'docx' | 'xlsx'; bytes: ArrayBuffer } }
  | { id: number; type: 'render';    payload: { sheet?: number; page?: number; viewport: Viewport; config: RenderConfig } }
  | { id: number; type: 'hitTest';   payload: { x: number; y: number } }
  | { id: number; type: 'exportPdf'; payload: PdfOptions }
  | { id: number; type: 'exportPng'; payload: { dpi: number } }
  | { id: number; type: 'cancel';    payload: { targetId: number } }
  | { id: number; type: 'dispose' };

export interface FrameTimings { buildMs: number; paintMs: number }

export type WorkerResponse =
  | { id: number; type: 'ok';    result?: unknown }
  | { id: number; type: 'error'; message: string; stack?: string }
  | { id: number; type: 'ready'; meta: { pages?: number; sheets?: string[] } }
  | { id: number; type: 'pdf';   bytes: Uint8Array }
  | { id: number; type: 'png';   bytes: Uint8Array }
  | { id: number; type: 'hit';   ref: HitResult | null }
  | { id: number; type: 'tick';  frameId: number; timings: FrameTimings };
