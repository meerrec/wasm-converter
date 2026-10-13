/**
 * Сообщения главного потока и воркера харнесса. Файл только с типами и константой: и
 * страница, и воркер импортируют его, поэтому ни один из них не тянет за собой другой.
 */

/** Версия протокола: воркер из старого бандла иначе молча разойдётся со страницей. */
export const PROTOCOL = 'x2t-harness/1';

export interface ConvertRequest {
  readonly protocol: typeof PROTOCOL;
  readonly kind: 'convert';
  /** Порядковый номер запроса — ответы приходят не в том порядке, в каком их ждали. */
  readonly id: number;
  readonly fileName: string;
  /** Целевое расширение без точки (`odt`, `txt`, `pdf`, …). */
  readonly to: string;
  /** Байты входного файла; передаются переносом, а не копией. */
  readonly bytes: ArrayBuffer;
}

export type MainToWorker = ConvertRequest;

/** Модуль поднялся: wasm загружен, MEMFS готов. */
export interface ReadyMessage {
  readonly protocol: typeof PROTOCOL;
  readonly kind: 'ready';
  readonly bootMs: number;
}

/** Строка из print/printErr модуля. */
export interface LogMessage {
  readonly protocol: typeof PROTOCOL;
  readonly kind: 'log';
  readonly line: string;
}

/** Отказ до подъёма модуля или поломка воркера; конвертации не было. */
export interface ErrorMessage {
  readonly protocol: typeof PROTOCOL;
  readonly kind: 'error';
  readonly message: string;
}

/** Итог конвертации: код возврата и stderr показываются как есть, без причёсывания. */
export interface ResultMessage {
  readonly protocol: typeof PROTOCOL;
  readonly kind: 'result';
  readonly id: number;
  readonly outputName: string;
  readonly code: number;
  readonly stderr: string;
  readonly convertMs: number;
  readonly bytes: ArrayBuffer;
}

export type WorkerToMain = ReadyMessage | LogMessage | ErrorMessage | ResultMessage;
