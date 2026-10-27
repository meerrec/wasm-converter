import { SpanStatusCode, trace } from '@opentelemetry/api';
import type { Attributes, Span } from '@opentelemetry/api';

import pkg from '../package.json';

/**
 * Пока потребитель не зарегистрировал провайдера, API отдаёт no-op-трейсер:
 * спаны не записываются и ничего никуда не уходит — политика «файлы
 * пользователя не покидают устройство» сохранена (ADR-0037). Версия
 * трейсера — версия пакета из `package.json`, чтобы она не расходилась с
 * версией релиза.
 */

/**
 * Атрибуты спана: только размеры, формат и идентификаторы. Байты документа,
 * их фрагменты, имена файлов и текст сюда не попадают никогда — спаны уходят
 * во внешний APM, и содержимое документа — не телеметрия (ADR-0037).
 */
export interface SpanAttributes extends Attributes {
  /** Формат документа: `'xlsx' | 'docx' | 'pptx' | 'xlsb' | 'ods' | 'odp' | 'odt'`. */
  format?: 'xlsx' | 'docx' | 'pptx' | 'xlsb' | 'ods' | 'odp' | 'odt';
  /** Что выгружали: `'pdf' | 'png'`. */
  kind?: 'pdf' | 'png';
  /** Длина входа (при открытии) или результата (при экспорте). */
  bytes?: number;
  /** Сколько листов открыла книга. */
  sheetCount?: number;
  /** Сколько страниц открыл документ. */
  pageCount?: number;
  /** Сколько слайдов открыла презентация. */
  slideCount?: number;
}

/**
 * Начать спан. Без зарегистрированного провайдера это no-op-спан:
 * `isRecording() === false`, атрибуты и завершение ничего не стоят.
 *
 * Трейсер берётся лениво, на каждый вызов, а не один раз при импорте:
 * потребитель может зарегистрировать провайдера после загрузки модуля, и
 * захваченный при импорте прокси-трейсер такого провайдера не увидел бы.
 * Глобальный реестр OpenTelemetry общий (`Symbol.for`), поэтому ленивый
 * вызов видит провайдера, зарегистрированного любой копией API.
 */
export function startSpan(name: string, attributes?: SpanAttributes): Span {
  return trace
    .getTracer('@meerrec/core', pkg.version)
    .startSpan(name, attributes === undefined ? undefined : { attributes });
}

/**
 * Завершить спан. При ошибке и только если спан записывается —
 * `recordException` и статус `ERROR`; `end()` вызывается всегда. Сама ошибка
 * наружу пробрасывается вызывающим кодом без подмены.
 */
export function endSpan(span: Span, error?: unknown): void {
  if (span.isRecording()) {
    if (error !== undefined) {
      const e = error instanceof Error ? error : new Error(String(error));
      span.recordException(e);
      span.setStatus({ code: SpanStatusCode.ERROR, message: e.message });
    }
  }
  span.end();
}

/**
 * Записать атрибут, только если спан записывается: без провайдера — no-op,
 * как и всё остальное здесь.
 */
export function setSpanAttribute(
  span: Span,
  key: string,
  value: string | number | boolean,
): void {
  if (span.isRecording()) span.setAttribute(key, value);
}
