/**
 * Публичное API пакета `@doc-converter/browser-convert`.
 *
 * Сейчас здесь только zip-механика — фундамент для браузерной конвертации OOXML:
 * OOXML-контейнеры это zip, и прежде чем отдать документ x2t, его нужно читать и
 * пересобирать без серверного рантайма.
 */
export type { ZipEntry, ZipRewriteOptions } from './zip.js';
export { listZipEntries, readZipEntry, rewriteZip, zipHasEntry } from './zip.js';
