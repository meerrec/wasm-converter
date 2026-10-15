/**
 * Загрузка wasm-модулей в Node.
 *
 * Модули собраны `wasm-pack --target web`: клей — обычный ESM, а `init()`
 * по умолчанию идёт за `.wasm` через `fetch(new URL(..., import.meta.url))`.
 * В Node `fetch` не умеет `file:`, поэтому путь не отдаём вовсе — читаем
 * файл сами и передаём байты: `__wbg_load` для не-`Response` зовёт
 * `WebAssembly.instantiate` напрямую. Сборка под `--target nodejs` не нужна,
 * и общий `scripts/build-wasm.sh` не трогается.
 *
 * Промис кешируется на процесс (или на воркер): модуль грузится один раз на
 * много файлов, а не на каждый.
 */

import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';

type PdfWasm = typeof import('@meerrec/wasm-pdf');
type DocxWasm = typeof import('@meerrec/wasm-docx');

let pdfModule: Promise<PdfWasm> | undefined;
let docxModule: Promise<DocxWasm> | undefined;

/** Модуль экспорта в PDF (и XLSX, и DOCX). */
export function loadPdfWasm(): Promise<PdfWasm> {
  pdfModule ??= initPdf();
  return pdfModule;
}

/** Модуль экспорта DOCX в Markdown/HTML/текст. */
export function loadDocxWasm(): Promise<DocxWasm> {
  docxModule ??= initDocx();
  return docxModule;
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

/**
 * Байты `.wasm`, лежащего рядом со своим клеем.
 *
 * `require.resolve` берёт путь у Node, а не у нас: в pnpm-раскладке пакет
 * лежит в стор, и склеенная из `import.meta.url` дорожка туда не ведёт.
 * Подпуть `./pkg/*` объявлен в `exports` обоих wasm-пакетов.
 */
async function wasmBytes(jsSpecifier: string): Promise<Uint8Array> {
  const require = createRequire(import.meta.url);
  const jsPath = require.resolve(jsSpecifier);
  return readFile(jsPath.replace(/\.js$/, '_bg.wasm'));
}
