/**
 * Разбор аргументов командной строки. Чистая функция без обращения к диску
 * и сети: тесты дергают её напрямую, а `run` получает уже готовую команду.
 *
 * Своя реализация вместо `commander`/`yargs`: аргументов немного, а CLI
 * должен запускаться на голом `node` без установки зависимостей, кроме
 * самих wasm-модулей.
 */

import { availableParallelism } from 'node:os';

import type { PdfOptions } from '@meerrec/core';

import { CliError } from './errors.js';
import { parseFormat, type OutputFormat } from './formats.js';
import type { CliRuntime } from './runtime.js';

export interface ConvertCommand {
  kind: 'convert';
  input: string;
  out: string;
  /** `null` — формат берётся из расширения `--out`. */
  format: OutputFormat | null;
  force: boolean;
  pdf: PdfOptions;
}

export interface BatchCommand {
  kind: 'batch';
  inputs: readonly string[];
  outDir: string;
  format: OutputFormat;
  workers: number;
  force: boolean;
  quiet: boolean;
  pdf: PdfOptions;
}

export type Command = ConvertCommand | BatchCommand | { kind: 'help' } | { kind: 'version' };

export const USAGE = `doc-converter — OOXML → PDF/Markdown/HTML/text

Usage:
  doc-converter convert <input> -o <output> [options]
  doc-converter batch <input...> --out <dir> [options]
  doc-converter help | --version

Formats:
  .docx → .pdf .md .html .txt
  .xlsx → .pdf

Convert options:
  -o, --out <path>        output file, or directory to name the file from the input
  -f, --format <format>   output format when --out has no known extension
      --force             overwrite existing output files

Batch options:
      --out <dir>         output directory (required)
  -f, --format <format>   output format (default: pdf)
  -w, --workers <n>       worker threads (default: cpu count, at most 8)
  -q, --quiet             do not print progress
      --force             overwrite existing output files

PDF options (both commands):
      --page-size <A4|A3|Letter|Legal>   XLSX only; DOCX keeps its section size
      --orientation <portrait|landscape> XLSX only
      --scale <factor>                   XLSX only, e.g. 0.8
      --fit-to-page                      XLSX only: fit the sheet on one page
      --fit-to-width <n>                 XLSX only: pages wide, 0 = unlimited
      --fit-to-height <n>                XLSX only: pages tall, 0 = unlimited
      --margins <top,right,bottom,left>  XLSX only, millimetres
      --sheet <n>                        XLSX only, 0-based; default is the whole book

Batch inputs may be files, directories (searched for .docx/.xlsx recursively)
or glob patterns like './in/**/*.xlsx'. Output files are written flat into
--out as <name>.<format>; duplicate names are an error.

Examples:
  doc-converter convert report.docx -o report.pdf
  doc-converter convert report.docx -o report.md
  doc-converter batch ./in/*.xlsx --out ./pdf --workers 4
  doc-converter batch ./in --out ./out --format pdf --workers 8 --scale 0.9
`;

interface ScanResult {
  positionals: string[];
  out: string | null;
  format: string | null;
  workers: string | null;
  force: boolean;
  quiet: boolean;
  pdf: PdfOptions;
  help: boolean;
}

/** Рантайм нужен только для умолчания `--workers`; остальное — чистый разбор. */
export type ParallelismSource = Pick<CliRuntime, 'availableParallelism'>;

export function parseArgs(argv: readonly string[], runtime?: ParallelismSource): Command {
  if (argv.length === 0) return { kind: 'help' };

  const command = argv[0];
  if (command === 'help' || command === '--help' || command === '-h') return { kind: 'help' };
  if (command === 'version' || command === '--version' || command === '-V') return { kind: 'version' };

  const rest = argv.slice(1);
  switch (command) {
    case 'convert':
      return toConvert(scan(rest));
    case 'batch':
      return toBatch(scan(rest), runtime);
    default:
      throw new CliError(`unknown command '${command}': expected 'convert', 'batch', 'help' or 'version'`);
  }
}

function scan(args: readonly string[]): ScanResult {
  const result: ScanResult = {
    positionals: [],
    out: null,
    format: null,
    workers: null,
    force: false,
    quiet: false,
    pdf: {},
    help: false,
  };

  let onlyPositionals = false;
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i]!;
    if (onlyPositionals) {
      result.positionals.push(arg);
      continue;
    }
    if (arg === '--') {
      onlyPositionals = true;
      continue;
    }

    // `--option=value` и `--option value` равнозначны.
    const eq = arg.startsWith('--') ? arg.indexOf('=') : -1;
    const name = eq >= 0 ? arg.slice(0, eq) : arg;
    const inline = eq >= 0 ? arg.slice(eq + 1) : null;
    const take = (): string => {
      if (inline !== null) return inline;
      i += 1;
      const value = args[i];
      if (value === undefined) throw new CliError(`option '${name}' needs a value`);
      return value;
    };

    switch (name) {
      case '--help':
      case '-h':
        result.help = true;
        break;
      case '-o':
      case '--out':
        result.out = take();
        break;
      case '-f':
      case '--format':
        result.format = take();
        break;
      case '-w':
      case '--workers':
        result.workers = take();
        break;
      case '--force':
        result.force = true;
        break;
      case '-q':
      case '--quiet':
        result.quiet = true;
        break;
      case '--page-size':
        result.pdf.pageSize = parsePageSize(take());
        break;
      case '--orientation':
        result.pdf.orientation = parseOrientation(take());
        break;
      case '--scale':
        result.pdf.scale = parsePositiveFloat(take(), '--scale');
        break;
      case '--fit-to-page':
        result.pdf.fitToPage = true;
        break;
      case '--fit-to-width':
        result.pdf.fitToWidth = parseIndex(take(), '--fit-to-width');
        break;
      case '--fit-to-height':
        result.pdf.fitToHeight = parseIndex(take(), '--fit-to-height');
        break;
      case '--margins':
        result.pdf.margins = parseMargins(take());
        break;
      case '--sheet':
        result.pdf.sheetIndex = parseIndex(take(), '--sheet');
        break;
      default:
        if (name.startsWith('-') && name !== '-') throw new CliError(`unknown option '${name}'`);
        result.positionals.push(arg);
    }
  }

  return result;
}

function toConvert(scanned: ScanResult): Command {
  if (scanned.help) return { kind: 'help' };
  const input = scanned.positionals[0];
  if (scanned.positionals.length === 0 || input === undefined) {
    throw new CliError("'convert' needs an input file");
  }
  if (scanned.positionals.length > 1) {
    throw new CliError("'convert' takes one input file; use 'batch' for several");
  }
  if (scanned.out === null) throw new CliError("'convert' needs --out <file>");
  return {
    kind: 'convert',
    input,
    out: scanned.out,
    format: scanned.format === null ? null : parseFormat(scanned.format),
    force: scanned.force,
    pdf: scanned.pdf,
  };
}

function toBatch(scanned: ScanResult, runtime?: ParallelismSource): Command {
  if (scanned.help) return { kind: 'help' };
  if (scanned.positionals.length === 0) throw new CliError("'batch' needs at least one input file, directory or glob");
  if (scanned.out === null) throw new CliError("'batch' needs --out <dir>");
  const workers = scanned.workers === null ? defaultWorkers(runtime) : parseIndex(scanned.workers, '--workers');
  if (workers < 1) throw new CliError('--workers must be at least 1');
  return {
    kind: 'batch',
    inputs: scanned.positionals,
    outDir: scanned.out,
    format: scanned.format === null ? 'pdf' : parseFormat(scanned.format),
    workers,
    force: scanned.force,
    quiet: scanned.quiet,
    pdf: scanned.pdf,
  };
}

/**
 * Умолчание воркеров: по числу ядер, но не больше восьми — дальше выигрыша нет.
 *
 * CLI передаёт рантайм (Node/Bun — `os.availableParallelism`, Deno —
 * `navigator.hardwareConcurrency`). Без рантайма (программный вызов, тесты)
 * берётся `node:os`: его реализуют все три рантайма, включая Deno.
 */
export function defaultWorkers(runtime?: ParallelismSource): number {
  const cores = runtime?.availableParallelism() ?? availableParallelism();
  return Math.max(1, Math.min(8, cores));
}

function parsePageSize(value: string): NonNullable<PdfOptions['pageSize']> {
  const sizes = ['A4', 'A3', 'Letter', 'Legal'] as const;
  const found = sizes.find((size) => size.toLowerCase() === value.toLowerCase());
  if (found === undefined) throw new CliError(`unknown page size '${value}': expected ${sizes.join(', ')}`);
  return found;
}

function parseOrientation(value: string): NonNullable<PdfOptions['orientation']> {
  const lower = value.toLowerCase();
  if (lower !== 'portrait' && lower !== 'landscape') {
    throw new CliError(`unknown orientation '${value}': expected portrait or landscape`);
  }
  return lower;
}

function parsePositiveFloat(value: string, option: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) throw new CliError(`${option} must be a positive number, got '${value}'`);
  return parsed;
}

function parseIndex(value: string, option: string): number {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new CliError(`${option} must be a non-negative integer, got '${value}'`);
  }
  return parsed;
}

/** `--margins top,right,bottom,left` в миллиметрах. */
function parseMargins(value: string): NonNullable<PdfOptions['margins']> {
  const parts = value.split(',').map((part) => part.trim());
  if (parts.length !== 4) {
    throw new CliError(`--margins needs four comma-separated millimetres (top,right,bottom,left), got '${value}'`);
  }
  const [top, right, bottom, left] = parts.map((part) => Number(part));
  if ([top, right, bottom, left].some((mm) => mm === undefined || !Number.isFinite(mm) || mm < 0)) {
    throw new CliError(`--margins takes non-negative numbers, got '${value}'`);
  }
  return { top: top as number, right: right as number, bottom: bottom as number, left: left as number };
}
