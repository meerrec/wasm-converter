/**
 * Вход конвертации в воркере: принимает `ConvertWorkerRequest`, отвечает
 * `ConvertWorkerResponse`. Конвертация живёт только здесь: сборка x2t без asyncify,
 * `main1` крутится в потоке до конца вызова и на главном потоке заморозит страницу.
 *
 * Вся логика — в фабрике `createConvertWorkerHandler`: `self` в неё не проникает,
 * поэтому обработчик гоняется в тестах без настоящего воркера. Слушатель реального
 * воркера висит внизу файла и только перекладывает сообщения из `self` в фабрику.
 */
import { convert as convertWithX2t, loadX2t } from '@meerrec/x2t';
import type { X2tLoadOptions, X2tModule } from '@meerrec/x2t';

/** Запрос конвертации воркеру: байты приезжают через postMessage с transfer. */
export interface ConvertWorkerRequest {
  readonly kind: 'convert';
  readonly id: number;
  /** URL glue-скрипта x2t.js. Запоминается с первого сообщения. */
  readonly scriptUrl: string;
  /** URL x2t.wasm; по умолчанию — рядом с glue. Тоже с первого сообщения. */
  readonly wasmUrl?: string;
  readonly bytes: ArrayBuffer;
  /** Формат входа: расширение без точки, нижний регистр. */
  readonly from: string;
  /** Формат выхода: расширение без точки, нижний регистр. */
  readonly to: string;
  /** Имя файла (для MEMFS и имени выхода). */
  readonly fileName: string;
}

export type ConvertWorkerResponse =
  | {
      readonly kind: 'result';
      readonly id: number;
      /** Байты выхода; уходят через transfer, а не копией. */
      readonly bytes: ArrayBuffer;
      /** Код возврата main1; пакет его НЕ фильтрует (80/88 — см. README). */
      readonly code: number;
      /** Всё, что рантайм напечатал в stderr за этот вызов. */
      readonly stderr: string;
      /** Время от прихода запроса до ответа, мс; на первом запросе включает подъём модуля. */
      readonly convertMs: number;
    }
  | {
      readonly kind: 'error';
      readonly id: number;
      /** Текст ошибки; stderr ошибки, если он есть, дописан отдельной строкой. */
      readonly message: string;
      /** Код из ошибки, если она его несёт (поле `code` у `X2tError`). */
      readonly code?: number;
    };

/** Из ошибки — текст, код и stderr: поля берутся, только если они на ней есть. */
function describeError(error: unknown): { message: string; code?: number; stderr: string } {
  const message = error instanceof Error ? error.message : String(error);
  let code: number | undefined;
  let stderr = '';
  if (typeof error === 'object' && error !== null) {
    const record = error as Record<string, unknown>;
    if (typeof record['code'] === 'number') {
      code = record['code'];
    }
    if (typeof record['stderr'] === 'string') {
      stderr = record['stderr'];
    }
  }
  return { message, ...(code !== undefined ? { code } : {}), stderr };
}

/**
 * Буфер результата для переноса: `convert` отдаёт свежий `Uint8Array` на собственном
 * буфере, но если это окажется view поверх чужого — через transfer уехал бы буфер
 * целиком, поэтому тогда берётся копия только нужного диапазона.
 */
function toArrayBuffer(bytes: Uint8Array<ArrayBuffer>): ArrayBuffer {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
    ? bytes.buffer
    : bytes.slice().buffer;
}

/**
 * Собирает обработчик сообщений конвертации: модуль x2t поднимается лениво на первом
 * запросе, дальнейшие запросы используют уже поднятый модуль.
 *
 * `scriptUrl`/`wasmUrl` берутся из **первого** сообщения и запоминаются; URL из
 * последующих сообщений игнорируются (в том числе при повторной попытке после отказа
 * подъёма). Неудача подъёма не кэшируется: следующий запрос пробует `loadX2t` снова —
 * один сетевой сбой не должен навсегда ломать конвертацию.
 *
 * Код возврата `main1` не фильтруется: `result` уходит при любом коде, решает
 * вызывающий. `error` отвечает только отказу подъёма модуля и падению рантайма.
 *
 * @param post — отправка ответа воркером; `transfer` — список переносимых буферов.
 * @returns Обработчик сообщения; сообщения не-`convert` игнорируются.
 */
export function createConvertWorkerHandler(
  post: (message: ConvertWorkerResponse, transfer?: Transferable[]) => void,
): (event: { readonly data: ConvertWorkerRequest }) => void {
  let module: X2tModule | null = null;
  let bootPromise: Promise<X2tModule> | null = null;
  let scriptUrl: string | null = null;
  let wasmUrl: string | undefined;

  /** Подъём модуля; URL'ы уже запомнены с первого сообщения. */
  async function boot(): Promise<X2tModule> {
    if (scriptUrl === null) {
      throw new Error('x2t boot attempted before the first convert request');
    }
    const options: X2tLoadOptions = {
      scriptUrl,
      ...(wasmUrl !== undefined ? { wasmUrl } : {}),
    };
    const x2tModule = await loadX2t(options);
    module = x2tModule;
    return x2tModule;
  }

  /** Поднятый модуль; отказ подъёма не кэшируется — следующая попытка стартует заново. */
  function ensureBoot(): Promise<X2tModule> {
    if (module !== null) {
      return Promise.resolve(module);
    }
    bootPromise ??= boot().then(
      (x2tModule) => x2tModule,
      (error: unknown) => {
        bootPromise = null;
        throw error;
      },
    );
    return bootPromise;
  }

  async function onConvert(request: ConvertWorkerRequest): Promise<void> {
    if (scriptUrl === null) {
      scriptUrl = request.scriptUrl;
      wasmUrl = request.wasmUrl;
    }
    const startedAt = performance.now();
    try {
      const x2tModule = await ensureBoot();
      const result = await convertWithX2t(x2tModule, new Uint8Array(request.bytes), {
        from: request.from,
        to: request.to,
        fileName: request.fileName,
      });
      const buffer = toArrayBuffer(result.bytes);
      post(
        {
          kind: 'result',
          id: request.id,
          bytes: buffer,
          code: result.code,
          stderr: result.stderr,
          convertMs: performance.now() - startedAt,
        },
        [buffer],
      );
    } catch (error) {
      const details = describeError(error);
      post({
        kind: 'error',
        id: request.id,
        message: details.stderr === '' ? details.message : `${details.message}\n${details.stderr}`,
        ...(details.code !== undefined ? { code: details.code } : {}),
      });
    }
  }

  return (event) => {
    if (event.data.kind !== 'convert') {
      return;
    }
    void onConvert(event.data);
  };
}

// Слушатель реального воркера: модуль исполняется и там, где `self` есть (браузерный
// воркер), и в node-окружении тестов, где его нет. `Worker` в касте — интерфейс с
// сигнатурой postMessage(message, transfer): у `self` типы — пересечение Window и
// WorkerGlobalScope (обе lib'ы в tsconfig), и нужной сигнатуры там нет.
if (typeof self !== 'undefined' && typeof self.addEventListener === 'function') {
  self.addEventListener('message', (event) =>
    createConvertWorkerHandler((message, transfer) =>
      (self as unknown as Worker).postMessage(message, transfer ?? []),
    )(event as MessageEvent<ConvertWorkerRequest>),
  );
}
