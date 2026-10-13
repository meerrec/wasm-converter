/**
 * Публичное API пакета `@doc-converter/x2t`: загрузка рантайма x2t и конвертация через него.
 */
export { X2tError, convert, loadX2t } from './x2t.js';
export type { ConvertOptions, ConvertResult, X2tLoadOptions } from './x2t.js';
export type { X2tAnalyzePathResult, X2tFileStat, X2tFS, X2tModule } from './x2t.js';
export type { X2tModuleConfig } from './glue.js';
