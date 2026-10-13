/**
 * Медиа-экстракция из OOXML-контейнеров: медиафайлы DOCX/XLSX/PPTX как object URLs.
 */
import { listZipEntries, readZipEntry } from './zip.js';

/** Каталоги медиа в контейнерах DOCX/XLSX/PPTX. */
const MEDIA_PREFIXES = ['word/media/', 'xl/media/', 'ppt/media/'] as const;

/** MIME по расширению; отсутствующее расширение или неизвестное — application/octet-stream. */
const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
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
  mov: 'video/quicktime',
  avi: 'video/x-msvideo',
  wmv: 'video/x-ms-wmv',
  mkv: 'video/x-matroska',
  m4v: 'video/mp4',
  mp3: 'audio/mpeg',
  wav: 'audio/wav',
  ogg: 'audio/ogg',
  m4a: 'audio/mp4',
  aac: 'audio/aac',
  wma: 'audio/x-ms-wma',
  flac: 'audio/flac',
};

/** Имя медиафайла без префикса каталога; null — запись не медиа или путь некорректен. */
function mediaBaseName(name: string): string | null {
  for (const prefix of MEDIA_PREFIXES) {
    if (!name.startsWith(prefix)) {
      continue;
    }
    const baseName = name.slice(prefix.length);
    // Пустой остаток — сама запись каталога; слэш — вложенный путь, не медиафайл.
    if (baseName === '' || baseName.includes('/')) {
      return null;
    }
    return baseName;
  }
  return null;
}

/** MIME по расширению; неизвестное или отсутствующее — application/octet-stream. */
function mimeForName(baseName: string): string {
  const dot = baseName.lastIndexOf('.');
  if (dot === -1) {
    return 'application/octet-stream';
  }
  return MIME_BY_EXTENSION[baseName.slice(dot + 1).toLowerCase()] ?? 'application/octet-stream';
}

/**
 * Медиафайлы DOCX/XLSX/PPTX как object URLs: `{ "media/<имя>": blobUrl }`. Ключ — путь
 * без префикса `word/|xl/|ppt/media/`. Вызывающий обязан ревокать URLs.
 * Повреждённые записи медиа пропускаются и не валят функцию.
 */
export async function extractDocxMediaUrls(bytes: Uint8Array): Promise<Record<string, string>> {
  const urls: Record<string, string> = {};
  for (const entry of listZipEntries(bytes)) {
    if (entry.isDir) {
      continue;
    }
    const baseName = mediaBaseName(entry.name);
    if (baseName === null) {
      continue;
    }
    let data: Uint8Array | null;
    try {
      data = await readZipEntry(bytes, entry);
    } catch {
      // Читаемая структура архива, но битая запись — пропускаем, не роняя всю экстракцию.
      data = null;
    }
    if (data === null) {
      continue;
    }
    // Типизация BlobPart не принимает Uint8Array<ArrayBufferLike>, хотя данные приходят
    // из разжатия и никогда не лежат на SAB — сужение типа без копии.
    const blob = new Blob([data as BlobPart], { type: mimeForName(baseName) });
    urls[`media/${baseName}`] = URL.createObjectURL(blob);
  }
  return urls;
}
