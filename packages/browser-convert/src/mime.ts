/**
 * Карта расширений документов в MIME-типы для браузерной конвертации OOXML: результат
 * нужно отдавать потребителю с правильным типом и использовать его в диалоге сохранения.
 */

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  doc: 'application/msword',
  odt: 'application/vnd.oasis.opendocument.text',
  rtf: 'application/rtf',
  txt: 'text/plain',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  xls: 'application/vnd.ms-excel',
  ods: 'application/vnd.oasis.opendocument.spreadsheet',
  csv: 'text/csv',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  ppt: 'application/vnd.ms-powerpoint',
  odp: 'application/vnd.oasis.opendocument.presentation',
  pdf: 'application/pdf',
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  html: 'text/html',
};

const DEFAULT_MIME_TYPE = 'application/octet-stream';

/**
 * MIME-тип документа по расширению без точки; регистр не важен, ведущая точка и пробелы
 * по краям снимаются. Неизвестное расширение — `'application/octet-stream'`.
 */
export function documentMimeType(extension: string): string {
  const normalized = extension.trim().replace(/^\./, '').toLowerCase();
  return MIME_BY_EXTENSION[normalized] ?? DEFAULT_MIME_TYPE;
}
