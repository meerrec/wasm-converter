/**
 * Сохранение результата конвертации на диск: File System Access API там, где он есть,
 * иначе скачивание через `<a download>`. Серверный рантайм не поддержан — среда обязана
 * быть браузером.
 */
import { documentMimeType } from './mime.js';

export interface SaveFileOptions {
  /** MIME-тип; по умолчанию documentMimeType(fileName). */
  readonly mimeType?: string;
  /** Человекочитаемое описание типа файла для диалога сохранения. */
  readonly description?: string;
}

/**
 * Часть File System Access API, которой нет в lib.dom.d.ts установленного TypeScript:
 * там есть `FileSystemFileHandle`, но picker-функции (`showSaveFilePicker`) и их опции
 * отсутствуют. Описываем минимум, нужный диалогу сохранения.
 */
interface FilePickerAcceptType {
  readonly description?: string;
  readonly accept: Record<string, string | string[]>;
}

interface SaveFilePickerOptions {
  suggestedName?: string;
  types?: FilePickerAcceptType[];
}

/** `window` с опциональным `showSaveFilePicker` — как оно выглядит в реальных браузерах. */
type WindowWithSavePicker = Window & {
  readonly showSaveFilePicker?: (options: SaveFilePickerOptions) => Promise<FileSystemFileHandle>;
};

/** Расширение файла из имени — часть после последней точки, без точки. */
function fileExtensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return dot >= 0 ? fileName.slice(dot + 1) : '';
}

/**
 * Сохранить данные на диск: File System Access API, если доступен, иначе скачивание через
 * `<a download>` + object URL. Пользователь закрыл диалог → `false` без ошибки; прочие
 * ошибки → reject. Только браузер: вне браузера бросает понятную ошибку.
 *
 * @param data — байты или Blob для записи.
 * @param fileName — предлагаемое имя файла (без каталога).
 * @param options — MIME-тип и описание типа для диалога сохранения.
 */
export async function saveFileToDisk(
  data: Blob | Uint8Array,
  fileName: string,
  options?: SaveFileOptions,
): Promise<boolean> {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    throw new Error('saveFileToDisk works only in a browser');
  }

  const mimeType = options?.mimeType ?? documentMimeType(fileExtensionOf(fileName));
  // TS типизирует Uint8Array как <ArrayBufferLike>, а BlobPart требует <ArrayBuffer>;
  // Blob на самом деле принимает любые байты, поэтому каст здесь чисто типовый.
  const blob =
    data instanceof Blob ? data : new Blob([data as Uint8Array<ArrayBuffer>], { type: mimeType });

  const showSaveFilePicker = (window as WindowWithSavePicker).showSaveFilePicker;
  if (typeof showSaveFilePicker === 'function') {
    return saveWithPicker(showSaveFilePicker, blob, fileName, mimeType, options?.description);
  }
  return downloadWithAnchor(blob, fileName);
}

/** Путь через File System Access API: диалог выбора файла и запись в writable-поток. */
async function saveWithPicker(
  showSaveFilePicker: (pickerOptions: SaveFilePickerOptions) => Promise<FileSystemFileHandle>,
  blob: Blob,
  fileName: string,
  mimeType: string,
  description: string | undefined,
): Promise<boolean> {
  const pickerOptions: SaveFilePickerOptions = { suggestedName: fileName };
  if (description !== undefined) {
    pickerOptions.types = [
      { description, accept: { [mimeType]: [`.${fileExtensionOf(fileName)}`] } },
    ];
  }

  let handle: FileSystemFileHandle;
  try {
    handle = await showSaveFilePicker(pickerOptions);
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') {
      return false; // пользователь закрыл диалог — это не ошибка
    }
    throw error;
  }

  const writable = await handle.createWritable();
  try {
    await writable.write(blob);
  } finally {
    await writable.close();
  }
  return true;
}

/** Фолбэк без File System Access API: клик по невидимой ссылке с атрибутом download. */
function downloadWithAnchor(blob: Blob, fileName: string): boolean {
  const anchor = document.createElement('a');
  anchor.href = URL.createObjectURL(blob);
  anchor.download = fileName;
  anchor.click();
  URL.revokeObjectURL(anchor.href);
  return true;
}
