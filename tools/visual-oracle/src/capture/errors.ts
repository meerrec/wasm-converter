// Типизированные ошибки захвата: код — единственное, на что опирается
// вызывающий, текст — для человека. Коды фиксированы: по ним CLI раскладывает
// сбой в `capture-error` или `harness-error`, и добавление кода без правки
// таблицы отображения сломало бы отчёт.

/** Код сбоя захвата. */
export type CaptureErrorCode =
  | 'open-failed'
  | 'unstable-frame'
  | 'export-failed'
  | 'rasterize-failed'
  | 'missing-tool'
  | 'fixture-not-listed'
  | 'browser-failed';

/** Тип результата, в который CLI превращает код сбоя. */
export type CaptureFailureResult = 'capture-error' | 'harness-error';

/**
 * Таблица отображения кода в тип результата.
 *
 * Граница проходит по вопросу «виноват ли движок»: не открылся документ,
 * не стабилизировался кадр или не отдался PNG — это неудачный захват
 * (`capture-error`, код выхода 2). Отсутствующий `pdftoppm`, фикстура вне
 * списка страницы и упавший браузер — сломанный контур (`harness-error`,
 * код выхода 3): такие результаты не сертифицируют движок ни в какую сторону.
 */
const RESULT_BY_CODE: Readonly<Record<CaptureErrorCode, CaptureFailureResult>> = {
  'open-failed': 'capture-error',
  'unstable-frame': 'capture-error',
  'export-failed': 'capture-error',
  'rasterize-failed': 'harness-error',
  'missing-tool': 'harness-error',
  'fixture-not-listed': 'harness-error',
  'browser-failed': 'harness-error',
};

/** Тип результата для кода сбоя. */
export function resultTypeForCode(code: CaptureErrorCode): CaptureFailureResult {
  return RESULT_BY_CODE[code];
}

/** Все известные коды — для проверок и диагностики. */
export const CAPTURE_ERROR_CODES: readonly CaptureErrorCode[] = Object.keys(
  RESULT_BY_CODE,
) as CaptureErrorCode[];

/** Сбой захвата с машинным кодом. Сообщение — по-английски: это публичный вывод. */
export class CaptureError extends Error {
  readonly code: CaptureErrorCode;

  constructor(code: CaptureErrorCode, message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = 'CaptureError';
    this.code = code;
  }
}

/** Сузить неизвестное исключение до `CaptureError`. */
export function isCaptureError(error: unknown): error is CaptureError {
  return error instanceof CaptureError;
}

/**
 * Привести любое исключение к `CaptureError`: чужой сбой внутри шага получает
 * код этого шага, а не «неизвестная ошибка» — иначе отчёт потерял бы причину.
 */
export function asCaptureError(error: unknown, code: CaptureErrorCode, context: string): CaptureError {
  if (isCaptureError(error)) return error;
  const detail = error instanceof Error ? error.message : String(error);
  return new CaptureError(code, `${context}: ${detail}`, { cause: error });
}
