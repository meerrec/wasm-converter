/**
 * Конвертация одного файла: общая для `convert` в текущем процессе и для
 * задач, которые исполняет воркер пула.
 */

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';

import { toDiagnostic, type ConversionStats, type PdfOptions } from '@meerrec/core';

import { CliError } from './errors.js';
import { assertSupported, INPUT_EXTENSIONS, inputKind, type InputKind, type OutputFormat } from './formats.js';
import { loadDocxWasm, loadOdpWasm, loadOdtWasm, loadPdfWasm, loadPptxPdfWasm, loadPptxWasm } from './wasm.js';

export interface ConvertJob {
  input: string;
  output: string;
  format: OutputFormat;
  pdf: PdfOptions;
}

export interface ConvertResult {
  input: string;
  output: string;
  bytes: number;
  ms: number;
  /** Те же метрики в общей форме браузера и CLI (для `--json`). */
  stats: ConversionStats;
}

/**
 * Прочитать `input`, записать `output` в нужном формате.
 *
 * # Errors
 * Если расширение входа или пара «вход → выход» не поддерживается, файл не
 * читается, wasm-модуль не разобрал документ или выход не записался.
 * Неподдерживаемая пара — `exitCode` 2 и код `unsupported-format`; отказ
 * чтения/конвертации/записи — `exitCode` 1 и диагностика `internal`/`cli`.
 */
export async function convertFile(job: ConvertJob): Promise<ConvertResult> {
  const started = Date.now();
  const kind = inputKind(job.input);
  if (kind === null) {
    const message = `unsupported input '${job.input}': expected ${INPUT_EXTENSIONS}`;
    throw new CliError(message, 2, toDiagnostic(message, { code: 'unsupported-format', layer: 'orchestration' }));
  }
  assertSupported(kind, job.format);

  try {
    const input = await readFile(job.input);
    const data: Uint8Array =
      job.format === 'pdf' ? await toPdf(kind, input, job.pdf) : await toText(kind, input, job.format);

    await mkdir(dirname(job.output), { recursive: true });
    await writeFile(job.output, data);

    const ms = Date.now() - started;
    const stats: ConversionStats = {
      format: kind,
      bytesIn: input.byteLength,
      bytesOut: data.byteLength,
      ms,
    };
    return { input: job.input, output: job.output, bytes: data.byteLength, ms, stats };
  } catch (error) {
    // Ошибку с готовой диагностикой не переоборачиваем: пул и `run` должны
    // получить исходный код, а не `internal`.
    if (error instanceof CliError) throw error;
    const message = error instanceof Error ? error.message : String(error);
    throw new CliError(message, 1, toDiagnostic(error, { code: 'internal', layer: 'cli' }));
  }
}

async function toPdf(kind: InputKind, bytes: Uint8Array, pdf: PdfOptions): Promise<Uint8Array> {
  // Опции для wasm-модуля: только заданные ключи (exactOptionalPropertyTypes).
  // Имена — camelCase, как в `PdfOptionsWire`; `allSheets` CLI не знает и не шлёт.
  const options: PdfOptions = {};
  if (pdf.pageSize !== undefined) options.pageSize = pdf.pageSize;
  if (pdf.orientation !== undefined) options.orientation = pdf.orientation;
  if (pdf.scale !== undefined) options.scale = pdf.scale;
  if (pdf.margins !== undefined) options.margins = pdf.margins;
  if (pdf.sheetIndex !== undefined) options.sheetIndex = pdf.sheetIndex;
  if (pdf.fitToPage !== undefined) options.fitToPage = pdf.fitToPage;
  if (pdf.fitToWidth !== undefined) options.fitToWidth = pdf.fitToWidth;
  if (pdf.fitToHeight !== undefined) options.fitToHeight = pdf.fitToHeight;

  // PPTX/ODP идут ленивым модулем pptx-pdf; `options` валидируются, но не
  // читаются — размер страницы задаёт сама презентация.
  if (kind === 'pptx') return (await loadPptxPdfWasm()).export_pptx_pdf(bytes, options);
  if (kind === 'odp') return (await loadPptxPdfWasm()).export_odp_pdf(bytes, options);

  // Остальные PDF — через wasm-pdf. У ODT/ODS/XLSB `sheetIndex` не
  // поддерживается: документ/книга экспортируются целиком, лист не выбирается.
  const wasm = await loadPdfWasm();
  if (kind === 'docx') return wasm.export_docx_pdf(bytes, options);
  if (kind === 'odt') return wasm.export_odt_pdf(bytes, options);
  if (kind === 'ods') return wasm.export_ods_pdf(bytes, options);
  if (kind === 'xlsb') return wasm.export_xlsb_pdf(bytes, options);
  // XLSX — единственный путь с выбором листа: `sheetIndex` либо вся книга.
  return pdf.sheetIndex === undefined
    ? wasm.export_pdf_book(bytes, options)
    : wasm.export_pdf(bytes, pdf.sheetIndex, options);
}

async function toText(kind: InputKind, bytes: Uint8Array, format: OutputFormat): Promise<Uint8Array> {
  return Buffer.from(await textOf(kind, bytes, format), 'utf8');
}

/** Текстовый экспорт по формату входа: документы и презентации, не таблицы. */
async function textOf(kind: InputKind, bytes: Uint8Array, format: OutputFormat): Promise<string> {
  switch (kind) {
    case 'docx': {
      const wasm = await loadDocxWasm();
      return exportText(bytes, format, wasm.docx_export_markdown, wasm.docx_export_html, wasm.docx_export_text);
    }
    case 'odt': {
      const wasm = await loadOdtWasm();
      return exportText(bytes, format, wasm.odt_export_markdown, wasm.odt_export_html, wasm.odt_export_text);
    }
    case 'pptx': {
      const wasm = await loadPptxWasm();
      return exportText(bytes, format, wasm.pptx_export_markdown, wasm.pptx_export_html, wasm.pptx_export_text);
    }
    case 'odp': {
      const wasm = await loadOdpWasm();
      return exportText(bytes, format, wasm.odp_export_markdown, wasm.odp_export_html, wasm.odp_export_text);
    }
    case 'xlsx':
    case 'ods':
    case 'xlsb': {
      // Таблицы сюда не доходят: `assertSupported` разрешает им только PDF.
      const message = `${kind} → ${format} is not supported`;
      throw new CliError(message, 2, toDiagnostic(message, { code: 'unsupported-format', layer: 'orchestration' }));
    }
  }
}

/** Выбрать экспортёр текста по цели: md, html, иначе txt. */
function exportText(
  bytes: Uint8Array,
  format: OutputFormat,
  markdown: (bytes: Uint8Array) => string,
  html: (bytes: Uint8Array) => string,
  text: (bytes: Uint8Array) => string,
): string {
  return format === 'md' ? markdown(bytes) : format === 'html' ? html(bytes) : text(bytes);
}
