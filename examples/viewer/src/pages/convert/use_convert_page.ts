import { useCallback, useEffect, useRef, useState, type ChangeEvent } from 'react';
import {
  extractDocxMediaUrls,
  preprocessDocxRuby,
  preprocessPptx,
  preprocessXlsxLineBreaks,
  saveFileToDisk,
} from '@meerrec/browser-convert';
import type { ConvertWorkerResponse } from '@meerrec/browser-convert/worker';
import { exportFileName } from '../../lib/download.js';
import { convertWorkerUrl, x2tScriptUrl, x2tWasmUrl } from '../../lib/viewer_urls.js';

/**
 * Логика страницы «Конвертер»: конвертация в воркере пакета
 * `@meerrec/browser-convert` плюс витрина его локальных возможностей —
 * медиа-экстракции и препроцессоров.
 *
 * Состояние обычное: страница ведёт одну операцию за раз, результат приходит по
 * событию. Байты между операциями не хранятся — их читает каждая операция заново.
 * Так мало того, что файл мог измениться на диске: буфер конвертации уходит в
 * воркер переносом и после этого на странице обнулён, а `File` позволяет
 * перечитать источник в любой момент.
 */

/** Форматы Word-семейства: конвертация внутри него x2t отдаёт кодом 0. */
const WORD_FORMATS = ['docx', 'odt', 'rtf', 'txt'] as const;

/** Форматы Excel-семейства: то же, но для таблиц. */
const EXCEL_FORMATS = ['xlsx', 'ods', 'csv'] as const;

/** Семейство выбранного файла: от него строится список целей конвертации. */
export type Family = 'word' | 'excel';

/** Форматы семейства — источник целей. */
const FAMILY_FORMATS: Readonly<Record<Family, readonly string[]>> = {
  word: WORD_FORMATS,
  excel: EXCEL_FORMATS,
};

/** Подписи семейств для строки о выбранном файле. */
const FAMILY_LABELS: Readonly<Record<Family, string>> = {
  word: 'Word',
  excel: 'Excel',
};

/** Тип контейнера, для которого у пакета есть препроцессор. */
export type PreprocessorId = 'docx' | 'xlsx' | 'pptx';

/** Подписи кнопок препроцессоров — по одной на тип файла. */
const PREPROCESS_LABELS: Readonly<Record<PreprocessorId, string>> = {
  docx: 'Развернуть ruby',
  pptx: 'Фиксы PPTX (notes/docProps)',
  xlsx: 'Снять двойное экранирование переносов',
};

/** Препроцессор по типу файла; набор и порядок — из пакета. */
const PREPROCESSORS: Readonly<
  Record<PreprocessorId, (bytes: Uint8Array) => Promise<Uint8Array>>
> = {
  docx: preprocessDocxRuby,
  pptx: preprocessPptx,
  xlsx: preprocessXlsxLineBreaks,
};

/** Контейнеры OOXML, из которых пакет умеет извлекать медиа. */
const MEDIA_FORMATS = new Set(['docx', 'xlsx', 'pptx']);

/** MIME медиа по расширению имени внутри контейнера (`media/image1.png`). */
const MEDIA_MIME: Readonly<Record<string, string>> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  bmp: 'image/bmp',
  tiff: 'image/tiff',
  tif: 'image/tiff',
  svg: 'image/svg+xml',
  webp: 'image/webp',
  emf: 'image/x-emf',
  wmf: 'image/x-wmf',
  mp4: 'video/mp4',
  webm: 'video/webm',
  mp3: 'audio/mpeg',
  wav: 'audio/wav',
};

/** Расширение имени без точки и регистра; без расширения — пустая строка. */
function extensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return dot < 0 ? '' : fileName.slice(dot + 1).toLowerCase();
}

/** Семейство по расширению; `null` — x2t-конвертация формата не поддержана. */
function familyOf(extension: string): Family | null {
  if ((WORD_FORMATS as readonly string[]).includes(extension)) return 'word';
  if ((EXCEL_FORMATS as readonly string[]).includes(extension)) return 'excel';
  return null;
}

/** Первая цель семейства, кроме исходного формата; `null` — целей нет. */
function firstTarget(extension: string): string | null {
  const family = familyOf(extension);
  if (family === null) return null;
  return FAMILY_FORMATS[family].find((format) => format !== extension) ?? null;
}

/** MIME медиафайла по имени; неизвестное расширение — octet-stream. */
function mediaMimeType(name: string): string {
  return MEDIA_MIME[extensionOf(name)] ?? 'application/octet-stream';
}

/** Текст ошибки: у исключений он есть, у остальных значений — их запись строкой. */
function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Результат успешной конвертации — всё, что страница показывает и сохраняет. */
export interface ConvertResultView {
  /** Байты выхода: при коде 0 их отдаёт кнопка «Скачать». */
  bytes: Uint8Array;
  /** Код возврата x2t без фильтрации: 0 — успех, 80/88 — отказ (см. README пакета). */
  code: number;
  /** stderr рантайма за этот вызов; пустая строка — рантайм молчал. */
  stderr: string;
  /** Время конвертации в воркере, мс; первый прогон включает подъём модуля. */
  convertMs: number;
  /** Имя, которым результат предлагается к сохранению. */
  fileName: string;
}

/** Медиафайл контейнера для галереи. */
export interface MediaView {
  /** Имя с каталогом внутри контейнера (`media/image1.png`) — уникальный ключ. */
  name: string;
  /** Object URL: живёт до повторного извлечения, смены файла или ухода со страницы. */
  url: string;
  /** MIME по расширению — по нему выбирается картинка или строка списка. */
  mime: string;
}

/** Итог препроцессинга: правленые байты и что с ними стало. */
export interface PreprocessedView {
  bytes: Uint8Array;
  /** `false` — препроцессор вернул исходные байты, архив не пересобирался. */
  changed: boolean;
  /** Размер до и после: в байтах, показываются рядом. */
  before: number;
  after: number;
  /** Имя, которым результат предлагается к сохранению. */
  fileName: string;
}

/** Кнопка препроцессора для текущего типа файла. */
export interface PreprocessorView {
  id: PreprocessorId;
  label: string;
}

/** Что страница берёт у хука: состояние секций и обработчики. */
export interface ConvertPageApi {
  /** Имя выбранного файла; `null` — файл не выбран. */
  fileName: string | null;
  /** Семейство выбранного формата; `null` — конвертация не поддержана. */
  family: Family | null;
  /** Подпись семейства для строки о файле; `null` — конвертация не поддержана. */
  familyLabel: string | null;
  /** Цели своего семейства без исходного формата. */
  targets: readonly string[];
  target: string;
  onTargetChange: (value: string) => void;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void;
  /** Конвертация идёт — кнопка заблокирована. */
  busy: boolean;
  canConvert: boolean;
  onConvert: () => void;
  result: ConvertResultView | null;
  /** Отказ воркера или чтения файла — человекочитаемая строка. */
  error: string | null;
  onDownloadResult: () => void;
  /** Медиа-секция применима к этому файлу. */
  canExtractMedia: boolean;
  mediaBusy: boolean;
  /** `null` — извлечение ещё не запускали; пустой массив — медиа нет. */
  media: readonly MediaView[] | null;
  mediaError: string | null;
  onShowMedia: () => void;
  /** Препроцессор текущего типа; `null` — для формата препроцессора нет. */
  preprocessor: PreprocessorView | null;
  preprocessBusy: boolean;
  preprocessed: PreprocessedView | null;
  preprocessError: string | null;
  onPreprocess: () => void;
  onDownloadPreprocessed: () => void;
}

/** Хук страницы «Конвертер»: воркер x2t, медиа и препроцессоры. */
export function useConvertPage(): ConvertPageApi {
  const [file, setFile] = useState<File | null>(null);
  const [target, setTarget] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ConvertResultView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [media, setMedia] = useState<readonly MediaView[] | null>(null);
  const [mediaBusy, setMediaBusy] = useState(false);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [preprocessed, setPreprocessed] = useState<PreprocessedView | null>(null);
  const [preprocessBusy, setPreprocessBusy] = useState(false);
  const [preprocessError, setPreprocessError] = useState<string | null>(null);

  /** Воркер создаётся при первой конвертации: 38 МБ внутри модуля и так ленивы. */
  const workerRef = useRef<Worker | null>(null);
  /** Идентификатор запроса: ответы одного прогона не должны разъезжаться с другими. */
  const nextIdRef = useRef(0);
  /** Object URL-ы текущей галереи — по этому набору они и отзываются. */
  const mediaUrlsRef = useRef<readonly string[]>([]);

  const extension = file === null ? '' : extensionOf(file.name);
  const family = familyOf(extension);
  const targets = family === null ? [] : FAMILY_FORMATS[family].filter((f) => f !== extension);
  const preprocessor: PreprocessorView | null =
    extension === 'docx' || extension === 'xlsx' || extension === 'pptx'
      ? { id: extension, label: PREPROCESS_LABELS[extension] }
      : null;

  /** Отпустить object URL-ы галереи: вызывается до нового извлечения и при уборке. */
  const revokeMedia = useCallback((): void => {
    for (const url of mediaUrlsRef.current) URL.revokeObjectURL(url);
    mediaUrlsRef.current = [];
  }, []);

  // Уход со страницы отпускает медиа и гасит воркер: оба живут дольше разметки
  // и без этого остались бы в памяти вкладки до её закрытия.
  useEffect(
    () => () => {
      revokeMedia();
      workerRef.current?.terminate();
      workerRef.current = null;
    },
    [revokeMedia],
  );

  /**
   * Отправить байты в воркер и дождаться ответа.
   *
   * Ответ ищется по `id` (запросов в полёте не больше одного, но протокол общий),
   * а сбой самого воркера — не ответ пакета, поэтому `error` тоже завершает
   * обещание: иначе кнопка осталась бы в «в работе» навсегда.
   */
  const convertInWorker = (
    bytes: ArrayBuffer,
    from: string,
    to: string,
    fileName: string,
  ): Promise<ConvertWorkerResponse> => {
    const worker = (workerRef.current ??= new Worker(convertWorkerUrl, {
      type: 'module',
      name: 'doc-converter-convert',
    }));
    const id = ++nextIdRef.current;
    return new Promise((resolve, reject) => {
      const settle = (): void => {
        worker.removeEventListener('message', onMessage);
        worker.removeEventListener('error', onError);
      };
      const onMessage = (event: MessageEvent<ConvertWorkerResponse>): void => {
        if (event.data.id !== id) return;
        settle();
        resolve(event.data);
      };
      const onError = (event: ErrorEvent): void => {
        settle();
        reject(new Error(event.message || 'воркер конвертера упал'));
      };
      worker.addEventListener('message', onMessage);
      worker.addEventListener('error', onError);
      // scriptUrl/wasmUrl воркер запоминает с первого сообщения; передаём их
      // каждый раз, чтобы странице не знать о внутреннем состоянии воркера.
      worker.postMessage(
        { kind: 'convert', id, scriptUrl: x2tScriptUrl, wasmUrl: x2tWasmUrl, bytes, from, to, fileName },
        [bytes],
      );
    });
  };

  /** Смена файла: прошлые результаты к новому файлу не относятся и не показываются. */
  const onFileChange = (event: ChangeEvent<HTMLInputElement>): void => {
    const next = event.target.files?.[0] ?? null;
    revokeMedia();
    setFile(next);
    setTarget(next === null ? '' : (firstTarget(extensionOf(next.name)) ?? ''));
    setResult(null);
    setError(null);
    setMedia(null);
    setMediaError(null);
    setPreprocessed(null);
    setPreprocessError(null);
  };

  const onConvert = (): void => {
    if (file === null || family === null || target === '' || busy) return;
    setBusy(true);
    setError(null);
    setResult(null);
    // Байты читаются под сам запрос: воркер забирает их переносом, а после этого
    // буфер на странице обнулён, — хранить его между операциями нельзя.
    void file
      .arrayBuffer()
      .then((bytes) => convertInWorker(bytes, extension, target, file.name))
      .then((response) => {
        if (response.kind === 'result') {
          setResult({
            bytes: new Uint8Array(response.bytes),
            code: response.code,
            stderr: response.stderr,
            convertMs: response.convertMs,
            fileName: exportFileName(file.name, target),
          });
        } else {
          setError(
            response.code === undefined
              ? response.message
              : `${response.message} (код ${response.code})`,
          );
        }
      })
      .catch((e: unknown) => {
        setError(messageOf(e));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  const onDownloadResult = (): void => {
    if (result === null) return;
    // Отмена диалога сохранения — не ошибка; reject (нет прав, диск) показываем.
    void saveFileToDisk(result.bytes, result.fileName).catch((e: unknown) => {
      setError(`сохранение не удалось: ${messageOf(e)}`);
    });
  };

  const onShowMedia = (): void => {
    if (file === null || !MEDIA_FORMATS.has(extension) || mediaBusy) return;
    setMediaBusy(true);
    setMediaError(null);
    void file
      .arrayBuffer()
      .then((buffer) => extractDocxMediaUrls(new Uint8Array(buffer)))
      .then((urls) => {
        // Прошлая галерея отпускается только после успешного извлечения: при
        // ошибке она остаётся живой и показанной.
        revokeMedia();
        const items = Object.entries(urls).map(([name, url]) => ({
          name,
          url,
          mime: mediaMimeType(name),
        }));
        mediaUrlsRef.current = items.map((item) => item.url);
        setMedia(items);
      })
      .catch((e: unknown) => {
        setMediaError(messageOf(e));
      })
      .finally(() => {
        setMediaBusy(false);
      });
  };

  const onPreprocess = (): void => {
    if (file === null || preprocessor === null || preprocessBusy) return;
    setPreprocessBusy(true);
    setPreprocessError(null);
    const id = preprocessor.id;
    void file
      .arrayBuffer()
      .then((buffer) => {
        const source = new Uint8Array(buffer);
        return PREPROCESSORS[id](source).then((fixed) => ({ source, fixed }));
      })
      .then(({ source, fixed }) => {
        setPreprocessed({
          bytes: fixed,
          // Без изменений препроцессор возвращает исходные байты — по этой
          // ссылке и отличаем «файл и так в порядке» от пересобранного архива.
          changed: fixed !== source,
          before: source.byteLength,
          after: fixed.byteLength,
          fileName: exportFileName(file.name, extension),
        });
      })
      .catch((e: unknown) => {
        setPreprocessError(messageOf(e));
      })
      .finally(() => {
        setPreprocessBusy(false);
      });
  };

  const onDownloadPreprocessed = (): void => {
    if (preprocessed === null) return;
    void saveFileToDisk(preprocessed.bytes, preprocessed.fileName).catch((e: unknown) => {
      setPreprocessError(`сохранение не удалось: ${messageOf(e)}`);
    });
  };

  return {
    fileName: file?.name ?? null,
    family,
    familyLabel: family === null ? null : FAMILY_LABELS[family],
    targets,
    target,
    onTargetChange: setTarget,
    onFileChange,
    busy,
    canConvert: file !== null && family !== null && target !== '' && !busy,
    onConvert,
    result,
    error,
    onDownloadResult,
    canExtractMedia: file !== null && MEDIA_FORMATS.has(extension) && !mediaBusy,
    mediaBusy,
    media,
    mediaError,
    onShowMedia,
    preprocessor,
    preprocessBusy,
    preprocessed,
    preprocessError,
    onPreprocess,
    onDownloadPreprocessed,
  };
}
