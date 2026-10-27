/**
 * Загрузка wasm-модулей.
 *
 * Модули собраны `wasm-pack --target web`: клей — обычный ESM, а `init()`
 * по умолчанию идёт за `.wasm` через `fetch(new URL(..., import.meta.url))`.
 * В Node `fetch` не умеет `file:`, поэтому путь не отдаём вовсе — читаем
 * файл сами и передаём байты: `__wbg_load` для не-`Response` зовёт
 * `WebAssembly.instantiate` напрямую. Как читать байты, решает рантайм
 * (`runtime.ts`): Node/Bun — `createRequire`, Deno — `import.meta.resolve`.
 * Сборка под `--target nodejs` не нужна, и общий `scripts/build-wasm.sh` не
 * трогается.
 *
 * Промис кешируется на процесс (или на воркер): модуль грузится один раз на
 * много файлов, а не на каждый.
 */

import { loadRuntime } from './runtime.js';

type PdfWasm = typeof import('@meerrec/wasm-pdf');
type DocxWasm = typeof import('@meerrec/wasm-docx');
type PptxWasm = typeof import('@meerrec/wasm-pptx');
type PptxPdfWasm = typeof import('@meerrec/wasm-pptx-pdf');
type OdtWasm = typeof import('@meerrec/wasm-odt');
type OdpWasm = typeof import('@meerrec/wasm-odp');

let pdfModule: Promise<PdfWasm> | undefined;
let docxModule: Promise<DocxWasm> | undefined;
let pptxModule: Promise<PptxWasm> | undefined;
let pptxPdfModule: Promise<PptxPdfWasm> | undefined;
let odtModule: Promise<OdtWasm> | undefined;
let odpModule: Promise<OdpWasm> | undefined;

/** Модуль экспорта в PDF (XLSX/ODS/XLSB, DOCX/ODT). */
export function loadPdfWasm(): Promise<PdfWasm> {
  pdfModule ??= initPdf();
  return pdfModule;
}

/** Модуль экспорта DOCX в Markdown/HTML/текст. */
export function loadDocxWasm(): Promise<DocxWasm> {
  docxModule ??= initDocx();
  return docxModule;
}

/** Модуль просмотра PPTX (Markdown/HTML/текст). */
export function loadPptxWasm(): Promise<PptxWasm> {
  pptxModule ??= initPptx();
  return pptxModule;
}

/** Модуль экспорта PPTX/ODP в PDF. */
export function loadPptxPdfWasm(): Promise<PptxPdfWasm> {
  pptxPdfModule ??= initPptxPdf();
  return pptxPdfModule;
}

/** Модуль просмотра ODT (Markdown/HTML/текст). */
export function loadOdtWasm(): Promise<OdtWasm> {
  odtModule ??= initOdt();
  return odtModule;
}

/** Модуль просмотра ODP (Markdown/HTML/текст). */
export function loadOdpWasm(): Promise<OdpWasm> {
  odpModule ??= initOdp();
  return odpModule;
}

async function initPdf(): Promise<PdfWasm> {
  const module = await import('@meerrec/wasm-pdf');
  await module.default({ module_or_path: await wasmBytes('@meerrec/wasm-pdf/pkg/doc_converter_pdf_wasm.js') });
  return module;
}

async function initDocx(): Promise<DocxWasm> {
  const module = await import('@meerrec/wasm-docx');
  await module.default({ module_or_path: await wasmBytes('@meerrec/wasm-docx/pkg/doc_converter_docx_wasm.js') });
  return module;
}

async function initPptx(): Promise<PptxWasm> {
  const module = await import('@meerrec/wasm-pptx');
  await module.default({ module_or_path: await wasmBytes('@meerrec/wasm-pptx/pkg/doc_converter_pptx_wasm.js') });
  return module;
}

async function initPptxPdf(): Promise<PptxPdfWasm> {
  const module = await import('@meerrec/wasm-pptx-pdf');
  await module.default({ module_or_path: await wasmBytes('@meerrec/wasm-pptx-pdf/pkg/doc_converter_pptx_pdf_wasm.js') });
  return module;
}

async function initOdt(): Promise<OdtWasm> {
  const module = await import('@meerrec/wasm-odt');
  await module.default({ module_or_path: await wasmBytes('@meerrec/wasm-odt/pkg/doc_converter_odt_wasm.js') });
  return module;
}

async function initOdp(): Promise<OdpWasm> {
  const module = await import('@meerrec/wasm-odp');
  await module.default({ module_or_path: await wasmBytes('@meerrec/wasm-odp/pkg/doc_converter_odp_wasm.js') });
  return module;
}

/**
 * Байты `.wasm`, лежащего рядом со своим клеем.
 *
 * Путь ищет рантайм: в pnpm-раскладке пакет лежит в сторе, и склеенная из
 * `import.meta.url` дорожка туда не ведёт. Подпуть `./pkg/*` объявлен в
 * `exports` обоих wasm-пакетов.
 */
async function wasmBytes(jsSpecifier: string): Promise<Uint8Array> {
  const runtime = await loadRuntime();
  return runtime.loadWasmBytes(jsSpecifier);
}
