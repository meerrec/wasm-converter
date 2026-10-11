// URL-ы воркеров и ленивых wasm-модулей. Здесь они собраны в одном месте,
// потому что нужны обеим страницам, а собираться должны один раз: чанки
// воркеров и wasm-модулей общие для DOCX и XLSX.

// Воркеры подключаются суффиксом `?worker&url`: только так Vite собирает их
// отдельными чанками. Голый `new URL('./docx_worker.ts', import.meta.url)` он
// считает обычным ассетом и инлайнит исходник в data-URL, где импорт
// `@doc-converter/core/worker` уже не разрешается, — воркер молча не стартует.
import docxWorkerFileUrl from '../workers/docx_worker.ts?worker&url';
import xlsxWorkerFileUrl from '../workers/xlsx_worker.ts?worker&url';

/** URL собранного воркера DOCX — уходит в `createDocxViewer({ workerUrl })`. */
export const docxWorkerUrl = docxWorkerFileUrl;

/** URL собранного воркера XLSX — уходит в `createXlsxViewer({ workerUrl })`. */
export const xlsxWorkerUrl = xlsxWorkerFileUrl;

// Ленивые wasm-модули отдаются URL-ами, а не импортом: `new URL(...)` Vite
// разворачивает в ассет, тогда как импорт модуля втянул бы его в основной
// бандл, и страница платила бы за wasm ещё до открытия документа. JS при этом
// копируется как есть, поэтому сборщик не кладёт рядом его wasm-бинарь: второй
// URL передаётся явно.

/** URL DOCX-модуля: воркер грузит его при открытии документа. */
export const docxWasmModuleUrl = new URL(
  '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm.js',
  import.meta.url,
).href;

/** URL wasm-бинарника DOCX-модуля: рядом с JS он не оказывается. */
export const docxWasmBinaryUrl = new URL(
  '@doc-converter/wasm-docx/pkg/doc_converter_docx_wasm_bg.wasm',
  import.meta.url,
).href;

/** URL PDF-модуля: воркер грузит его при первом экспорте в PDF. */
export const pdfWasmModuleUrl = new URL(
  '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm.js',
  import.meta.url,
).href;

/** URL wasm-бинарника PDF-модуля: рядом с JS он не оказывается. */
export const pdfWasmBinaryUrl = new URL(
  '@doc-converter/wasm-pdf/pkg/doc_converter_pdf_wasm_bg.wasm',
  import.meta.url,
).href;
