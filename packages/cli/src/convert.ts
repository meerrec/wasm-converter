/**
 * Конвертация одного файла: общая для `convert` в текущем процессе и для
 * задач, которые исполняет воркер пула.
 */

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';

import type { PdfCliOptions } from './args.js';
import { CliError } from './errors.js';
import { assertSupported, inputKind, type InputKind, type OutputFormat } from './formats.js';
import { loadDocxWasm, loadPdfWasm } from './wasm.js';

export interface ConvertJob {
  input: string;
  output: string;
  format: OutputFormat;
  pdf: PdfCliOptions;
}

export interface ConvertResult {
  input: string;
  output: string;
  bytes: number;
  ms: number;
}

/**
 * Прочитать `input`, записать `output` в нужном формате.
 *
 * # Errors
 * Если расширение входа или пара «вход → выход» не поддерживается, файл не
 * читается, wasm-модуль не разобрал документ или выход не записался.
 */
export async function convertFile(job: ConvertJob): Promise<ConvertResult> {
  const started = Date.now();
  const kind = inputKind(job.input);
  if (kind === null) {
    throw new CliError(`unsupported input '${job.input}': expected .docx or .xlsx`);
  }
  assertSupported(kind, job.format);

  const input = await readFile(job.input);
  const data: Uint8Array = job.format === 'pdf' ? await toPdf(kind, input, job.pdf) : await toText(kind, input, job.format);

  await mkdir(dirname(job.output), { recursive: true });
  await writeFile(job.output, data);

  return { input: job.input, output: job.output, bytes: data.byteLength, ms: Date.now() - started };
}

async function toPdf(kind: InputKind, bytes: Uint8Array, pdf: PdfCliOptions): Promise<Uint8Array> {
  const wasm = await loadPdfWasm();
  const options = pdfOptionsForWasm(pdf);
  if (kind === 'docx') return wasm.export_docx_pdf(bytes, options);
  return pdf.sheetIndex === undefined
    ? wasm.export_pdf_book(bytes, options)
    : wasm.export_pdf(bytes, pdf.sheetIndex, options);
}

async function toText(kind: InputKind, bytes: Uint8Array, format: OutputFormat): Promise<Uint8Array> {
  // XLSX сюда не доходит: `assertSupported` выше разрешает ему только PDF.
  if (kind !== 'docx') throw new CliError(`${kind} → ${format} is not supported`);
  const wasm = await loadDocxWasm();
  const text =
    format === 'md'
      ? wasm.docx_export_markdown(bytes)
      : format === 'html'
        ? wasm.docx_export_html(bytes)
        : wasm.docx_export_text(bytes);
  return Buffer.from(text, 'utf8');
}

/**
 * Опции для `PdfOptionsWire` wasm-модуля: только заданные ключи.
 * Имена — camelCase, как в TS-интерфейсе просмотрщика.
 */
function pdfOptionsForWasm(pdf: PdfCliOptions): Record<string, unknown> {
  const options: Record<string, unknown> = {};
  if (pdf.pageSize !== undefined) options['pageSize'] = pdf.pageSize;
  if (pdf.orientation !== undefined) options['orientation'] = pdf.orientation;
  if (pdf.scale !== undefined) options['scale'] = pdf.scale;
  if (pdf.margins !== undefined) {
    options['margins'] = { top: pdf.margins.top, right: pdf.margins.right, bottom: pdf.margins.bottom, left: pdf.margins.left };
  }
  if (pdf.sheetIndex !== undefined) options['sheetIndex'] = pdf.sheetIndex;
  if (pdf.fitToPage !== undefined) options['fitToPage'] = pdf.fitToPage;
  if (pdf.fitToWidth !== undefined) options['fitToWidth'] = pdf.fitToWidth;
  if (pdf.fitToHeight !== undefined) options['fitToHeight'] = pdf.fitToHeight;
  return options;
}
