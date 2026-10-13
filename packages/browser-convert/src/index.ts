/**
 * Публичное API пакета `@doc-converter/browser-convert`.
 *
 * Браузерная конвертация OOXML поверх `@doc-converter/x2t` и вспомогательная механика:
 * zip для чтения и пересборки OOXML-контейнеров, класс-обёртка x2t, карта MIME-типов
 * и сохранение результата на диск.
 */
export type { ZipEntry, ZipRewriteOptions } from './zip.js';
export { listZipEntries, readZipEntry, rewriteZip, zipHasEntry } from './zip.js';
export { X2TConverter } from './converter.js';
export type { ConvertOptions, ConvertResult, X2TConverterOptions } from './converter.js';
export { documentMimeType } from './mime.js';
export { saveFileToDisk } from './save.js';
export type { SaveFileOptions } from './save.js';
