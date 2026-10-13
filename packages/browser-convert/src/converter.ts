/**
 * Класс-обёртка над x2t для браузерной конвертации OOXML: поднимает рантайм WASM один раз
 * и гоняет через него файлы. Рантайм и ограничения сборки — в
 * [`packages/x2t/README.md`](../../x2t/README.md); почему pdf/html/png/jpg-цели на текущих
 * сборках возвращают код 80 — [`docs/adr/0024-x2t-rendering-backend.md`](../../../docs/adr/0024-x2t-rendering-backend.md).
 */
import { convert as convertWithX2t, loadX2t } from '@doc-converter/x2t';
import type {
  ConvertOptions as X2tConvertOptions,
  X2tLoadOptions,
  X2tModule,
} from '@doc-converter/x2t';

import { documentMimeType } from './mime.js';

export interface X2TConverterOptions {
  /** URL glue-скрипта x2t.js. */
  readonly scriptUrl: string;
  /** URL x2t.wasm; по умолчанию — путь рядом с glue-скриптом. */
  readonly wasmUrl?: string;
  /** Вывод рантайма x2t (print). */
  readonly onPrint?: (line: string) => void;
}

export interface ConvertOptions {
  /**
   * Имя файла (без каталога) для MEMFS и имени выхода; по умолчанию — имя File/Blob
   * или `'document.<расширение>'`. Санитизируется под MEMFS.
   */
  readonly fileName?: string;
}

export interface ConvertResult {
  /** Байты выходного файла; при ненулевом коде может быть пустым. */
  readonly bytes: Uint8Array;
  /**
   * Код возврата main1: 0 — успех; 80 — рендер-цель (pdf/html/png/jpg), движка раскладки
   * в wasm-сборке нет (ADR-0024); 88 — кросс-семейная конвертация. Пакет код НЕ фильтрует.
   */
  readonly code: number;
  /** Всё, что рантайм напечатал в stderr за этот вызов. */
  readonly stderr: string;
  /** Имя выходного файла: санитизированная база входа + `.` + расширение цели. */
  readonly fileName: string;
  /** MIME-тип результата по расширению цели. */
  readonly mimeType: string;
}

/** Символы, недопустимые в имени MEMFS: разделители путей, разметка XML, управляющие. */
const ILLEGAL_IN_NAME = /[/\\?<>:*|"\u0000-\u001f\u0080-\u009f]/g;

/** Максимальная длина имени файла — с запасом под расширение и каталог `/working/`. */
const MAX_FILE_NAME_LENGTH = 100;

/**
 * Приводит имя файла к безопасному для MEMFS виду: недопустимые символы — в `_`, обрезка
 * до {@link MAX_FILE_NAME_LENGTH}, пустому имени — `'document'`. Кириллица и пробелы
 * остаются: MEMFS и XML-параметры их переживают, опасны только разделители путей и разметка.
 */
function sanitizeFileName(name: string): string {
  const sanitized = name.replace(ILLEGAL_IN_NAME, '_').trim().slice(0, MAX_FILE_NAME_LENGTH).trim();
  return sanitized.length > 0 ? sanitized : 'document';
}

/**
 * Расширение файла из имени: часть после последней точки, без точки, в нижнем регистре.
 * Бросает, если точки нет или она стоит на месте, где расширения не остаётся.
 */
function extensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  if (dot <= 0 || dot === fileName.length - 1) {
    throw new Error(`fileName has no usable extension: "${fileName}"`);
  }
  return fileName.slice(dot + 1).toLowerCase();
}

export class X2TConverter {
  /** Модуль x2t после {@link X2TConverter.init}; undefined до первого вызова init. */
  private module: X2tModule | undefined;

  /**
   * Поднимает модуль x2t (`loadX2t` из `@doc-converter/x2t`). Повторный вызов бросает:
   * glue-скрипт исполняется глобально и кладёт единственный `Module` на область видимости,
   * поэтому второй init — и init другого экземпляра в той же области — невозможен.
   *
   * @param options — URL glue-скрипта и wasm, приёмник строк рантайма.
   * @throws {Error} — если init уже был вызван у этого экземпляра; `X2tError` — если
   *   модуль уже загружен в этой области или рантайм не поднялся.
   */
  async init(options: X2TConverterOptions): Promise<void> {
    if (this.module !== undefined) {
      throw new Error('X2TConverter.init() was already called: one x2t module per scope');
    }
    const loadOptions: X2tLoadOptions = {
      scriptUrl: options.scriptUrl,
      ...(options.wasmUrl !== undefined ? { wasmUrl: options.wasmUrl } : {}),
      ...(options.onPrint !== undefined ? { onPrint: options.onPrint } : {}),
    };
    this.module = await loadX2t(loadOptions);
  }

  /**
   * Конвертирует файл в `targetExt` (расширение без точки, регистр не важен).
   *
   * Блокирующая операция: x2t без asyncify, `main1` крутится в потоке до конца конвертации —
   * вызывать из воркера, на главном потоке страница зависнет. Код возврата не фильтруется:
   * pdf/html/png/jpg-цели на текущих сборках возвращают 80 (см. ADR-0024), кросс-семейная
   * конвертация — 88.
   *
   * @param file — вход: File, Blob или готовые байты.
   * @param targetExt — целевой формат без точки.
   * @param options — имя файла для MEMFS и результата.
   * @returns Байты выхода, код `main1`, stderr, итоговое имя и MIME-тип.
   * @throws {Error} — если init не вызван или из имени входа нельзя вывести расширение.
   */
  async convert(
    file: File | Blob | Uint8Array,
    targetExt: string,
    options?: ConvertOptions,
  ): Promise<ConvertResult> {
    const module = this.module;
    if (module === undefined) {
      throw new Error('convert() requires init(): call X2TConverter.init() first');
    }

    const to = targetExt.trim().replace(/^\./, '').toLowerCase();
    const rawName = options?.fileName ?? (file instanceof File ? file.name : `document.${to}`);
    const from = extensionOf(rawName);
    const base = sanitizeFileName(rawName.slice(0, rawName.length - from.length - 1));

    const bytes = file instanceof Uint8Array ? file : new Uint8Array(await file.arrayBuffer());

    const x2tOptions: X2tConvertOptions = { from, to, fileName: base };
    const result = await convertWithX2t(module, bytes, x2tOptions);

    return {
      bytes: result.bytes,
      code: result.code,
      stderr: result.stderr,
      fileName: `${base}.${to}`,
      mimeType: documentMimeType(to),
    };
  }
}
