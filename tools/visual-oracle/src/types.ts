// Общий контракт визуального оракула: типы, на которые опираются и захват
// кадров, и движок сравнения, и отчёт. Файл стабилен по составу полей —
// переименование или добавление поля ломает обе стороны, поэтому менять его
// можно только вместе с потребителями.

/** Тип результата сравнения одного capture. */
export type ResultType =
  | 'match'
  | 'visual-diff'
  | 'page-mismatch'
  | 'missing-reference'
  | 'capture-error'
  | 'harness-error';

/** Причина расхождения: где искать дефект. */
export type Cause = 'parse' | 'model' | 'layout' | 'font' | 'render' | 'harness' | 'reference';

/** Режим просмотра, для которого снят capture. */
export type ViewMode = 'print-layout' | 'pdf' | 'normal' | 'print-preview';

/** Формат источника. */
export type DocFormat = 'docx' | 'xlsx';

/** Прямоугольник в пикселях изображения. */
export interface Rect { x: number; y: number; width: number; height: number }

/** Артефакт-изображение с хешем и размерами. */
export interface ImageInfo { uri: string; sha256: string; width: number; height: number }

/** Фактические параметры захвата собственного вьюера. */
export interface RuntimeMetadata {
  engineRevision: string;
  transport: 'sab' | 'bytes';
  renderMode: 'worker' | 'main';
  viewport: { width: number; height: number };
  dpr: number;
  zoom: number;
  fonts: string[];
  fontSubstitutions: string[];
  /** FeatureSupportReport ещё не реализован в @meerrec/core — поле честно null. */
  featureSupport: null;
  timeToStableMs: number;
  stableHashes: [string, string];
  captureBackend: 'canvas' | 'pdf';
}

/** Метрики пиксельного сравнения. */
export interface DiffMetrics {
  diffPixels: number;
  diffRatio: number;
  maxChannelDelta: number;
  /** SSIM — только диагностика, не смягчает правило нуля расходящихся пикселей. */
  ssim: number | null;
}

/** Результат одного capture: эталон, факт, вердикт. */
export interface CaptureResult {
  fixtureId: string;
  format: DocFormat;
  mode: ViewMode;
  captureId: string;
  resultType: ResultType;
  cause: Cause;
  causeSource: 'rule' | 'override';
  causeEvidence: string;
  reference: ImageInfo | null;
  actual: ImageInfo | null;
  diffImage: string | null;
  metrics: DiffMetrics | null;
  regions: Rect[];
  runtime: RuntimeMetadata | null;
  error: { code: string; message: string } | null;
}

/** Машинный отчёт прогона. */
export interface RunReport {
  schemaVersion: '1.0.0';
  generatedAt: string;
  engineRevision: string;
  corpusId: string;
  corpusVersion: string;
  matrixVersion: string;
  baselineId: string | null;
  summary: { total: number } & Record<ResultType, number>;
  byCause: Record<Cause, number>;
  byTag: Record<string, { total: number; match: number }>;
  captures: CaptureResult[];
}
