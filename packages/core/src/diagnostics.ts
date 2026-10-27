// Единая диагностика браузера и CLI: один тип ошибки на все слои.
//
// Rust отдаёт `DiagnosticWire` из `crates/core/src/wire.rs` (генерируется в
// `generated/wire.ts`), воркер добавляет его к `error`/`ready`, а просмотрщик
// реджектит `ViewerError` с этим же объектом. CLI импортирует те же типы из
// `@meerrec/core`, поэтому `--json` и человекочитаемый вывод не расходятся с
// браузером.

import type {
  DiagnosticCodeWire,
  DiagnosticLayerWire,
  DiagnosticWire,
} from './generated/wire.js';
import type { DocFormat, WorkerErrorCode, WorkerErrorMsg } from './protocol.js';

/** Структурная диагностика — тот же объект, что приходит из Rust/воркера. */
export type Diagnostic = DiagnosticWire;
/** Код отказа: машинный, стабильный между браузером и CLI. */
export type DiagnosticCode = DiagnosticCodeWire;
/** Слой, на котором случился отказ. */
export type DiagnosticLayer = DiagnosticLayerWire;

/** Метрики одной конвертации/открытия; общая форма браузера и CLI. */
export interface ConversionStats {
  /** Формат входа. */
  format: DocFormat;
  /** Сколько байт пришло на вход. */
  bytesIn: number;
  /** Сколько байт получилось на выходе (0, если выход не байтовый). */
  bytesOut: number;
  /** Время операции, миллисекунды. */
  ms: number;
  /** Число единиц документа: листов, страниц или слайдов. */
  units?: number;
  /** Сколько воркеров участвовало (только batch CLI). */
  workers?: number;
}

/**
 * Ошибка просмотрщика с машинной диагностикой.
 *
 * Наследуется от `Error`, чтобы существующий код (`catch`, `rejects.toThrow`)
 * продолжал работать; `message` совпадает с `diagnostic.message`, а `code` и
 * `layer` доступны без разбора текста.
 */
export class ViewerError extends Error {
  readonly diagnostic: Diagnostic;

  constructor(diagnostic: Diagnostic) {
    super(diagnostic.message);
    this.name = 'ViewerError';
    this.diagnostic = diagnostic;
  }
}

/** Пара «код/слой», которой помечают ошибку без структурной информации. */
export interface DiagnosticFallback {
  code?: DiagnosticCode;
  layer?: DiagnosticLayer;
  format?: DocFormat;
  retryable?: boolean;
}

const CODE_PREFIX: Readonly<Record<string, DiagnosticCode>> = {
  '[cancelled]': 'cancelled',
  '[parse]': 'malformed-package',
  '[malformed]': 'malformed-package',
  '[layout]': 'layout-failed',
  '[render]': 'render-failed',
  '[transport]': 'transport-unavailable',
  '[export]': 'export-failed',
  '[limit]': 'limit-exceeded',
  '[unsupported]': 'unsupported-format',
  '[io]': 'io-failed',
};

/**
 * Привести любое исключение к `Diagnostic`.
 *
 * Сначала разбирается объект (Rust мог отдать `{ code, layer, message }`),
 * затем строковый префикс `[code]` — тот же, каким wasm помечает отмену
 * (ADR-0032). Всё остальное — `message` как есть с fallback-кодом: потерять
 * текст ошибки нельзя, даже если слой её не разметил.
 */
export function toDiagnostic(error: unknown, fallback: DiagnosticFallback = {}): Diagnostic {
  if (error instanceof ViewerError) return error.diagnostic;

  if (error instanceof Error) {
    return fromMessage(error.message, fallback);
  }
  if (typeof error === 'string') {
    return fromMessage(error, fallback);
  }
  if (isDiagnosticLike(error)) {
    const diagnostic: Diagnostic = {
      code: error.code,
      layer: error.layer,
      message: error.message,
      retryable: error.retryable ?? fallback.retryable ?? false,
    };
    const format = error.format ?? fallback.format;
    if (format !== undefined) diagnostic.format = format;
    return diagnostic;
  }
  return fromMessage(String(error), fallback);
}

/** Диагностика по коду воркера: стадия известна даже без разметки Rust. */
export function workerDiagnostic(msg: WorkerErrorMsg, format?: DocFormat): Diagnostic {
  if (msg.diagnostic !== undefined) return msg.diagnostic;
  const mapped = fromWorkerCode(msg.code);
  const fallback: DiagnosticFallback = {
    code: mapped.code,
    layer: mapped.layer,
    retryable: mapped.retryable,
  };
  if (format !== undefined) fallback.format = format;
  return fromMessage(msg.message, fallback);
}

/** Человекочитаемая строка для CLI и логов. */
export function formatDiagnostic(diagnostic: Diagnostic): string {
  const format = diagnostic.format === undefined ? '' : ` [${diagnostic.format}]`;
  return `${diagnostic.layer}/${diagnostic.code}:${format} ${diagnostic.message}`;
}

/** Обернуть диагностику в `Error` для промисов и `throw`. */
export function diagnosticError(diagnostic: Diagnostic): ViewerError {
  return new ViewerError(diagnostic);
}

function fromMessage(message: string, fallback: DiagnosticFallback): Diagnostic {
  const prefix = Object.keys(CODE_PREFIX).find((candidate) => message.startsWith(candidate));
  const mapped = prefix === undefined ? undefined : CODE_PREFIX[prefix];
  const code = mapped ?? fallback.code ?? 'internal';
  const text = prefix === undefined ? message : message.slice(prefix.length).trim();
  const diagnostic: Diagnostic = {
    code,
    layer: fallback.layer ?? layerOf(code),
    message: text.length === 0 ? message : text,
    retryable: fallback.retryable ?? (code === 'cancelled' || code === 'transport-unavailable'),
  };
  if (fallback.format !== undefined) diagnostic.format = fallback.format;
  return diagnostic;
}

/** Слой по коду: нужен, когда ошибка пришла строкой без разметки. */
function layerOf(code: DiagnosticCode): DiagnosticLayer {
  switch (code) {
    case 'malformed-package':
    case 'encrypted':
    case 'limit-exceeded':
      return 'parse';
    case 'layout-failed':
      return 'layout';
    case 'render-failed':
      return 'render';
    case 'transport-unavailable':
      return 'transport';
    case 'export-failed':
    case 'io-failed':
      return 'export';
    case 'unsupported-format':
    case 'cancelled':
      return 'orchestration';
    case 'internal':
      return 'internal';
  }
}

function fromWorkerCode(code: WorkerErrorCode | undefined): {
  code: DiagnosticCode;
  layer: DiagnosticLayer;
  retryable: boolean;
} {
  switch (code) {
    case 'open':
      return { code: 'malformed-package', layer: 'parse', retryable: false };
    case 'export':
      return { code: 'export-failed', layer: 'export', retryable: false };
    case 'cancelled':
      return { code: 'cancelled', layer: 'orchestration', retryable: true };
    case 'init':
      return { code: 'internal', layer: 'orchestration', retryable: false };
    case 'internal':
    case undefined:
      return { code: 'internal', layer: 'internal', retryable: false };
  }
}

function isDiagnosticLike(
  value: unknown,
): value is {
  code: DiagnosticCode;
  layer: DiagnosticLayer;
  message: string;
  retryable?: boolean;
  format?: DocFormat;
} {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.code === 'string' &&
    typeof candidate.layer === 'string' &&
    typeof candidate.message === 'string'
  );
}
