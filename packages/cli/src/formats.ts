/**
 * Форматы ввода и вывода: единственное место, где сказано, что во что
 * конвертируется. Оба фронта — `convert` и `batch` — спрашивают здесь,
 * поэтому неподдерживаемая пара расширений ловится до запуска воркеров.
 */

import { CliError } from './errors.js';

/** Формат, в который CLI умеет писать. */
export type OutputFormat = 'pdf' | 'md' | 'html' | 'txt';

/** Формат ввода: разбирается по расширению. */
export type InputKind = 'docx' | 'xlsx';

/** Расширение выходного файла, с точкой. */
export const OUTPUT_EXTENSION: Record<OutputFormat, string> = {
  pdf: '.pdf',
  md: '.md',
  html: '.html',
  txt: '.txt',
};

/** Что во что конвертируется. У XLSX путь только один — PDF. */
export const SUPPORTED: Record<InputKind, readonly OutputFormat[]> = {
  docx: ['pdf', 'md', 'html', 'txt'],
  xlsx: ['pdf'],
};

const INPUT_KIND_BY_EXTENSION: Record<string, InputKind> = {
  '.docx': 'docx',
  '.xlsx': 'xlsx',
};

const FORMAT_BY_EXTENSION: Record<string, OutputFormat> = {
  '.pdf': 'pdf',
  '.md': 'md',
  '.markdown': 'md',
  '.html': 'html',
  '.htm': 'html',
  '.txt': 'txt',
  '.text': 'txt',
};

const FORMATS: readonly OutputFormat[] = ['pdf', 'md', 'html', 'txt'];

/** Расширение пути в нижнем регистре, вместе с точкой; `''`, если его нет. */
export function extensionOf(path: string): string {
  const base = path.slice(path.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  return dot <= 0 ? '' : base.slice(dot).toLowerCase();
}

/** Формат ввода по расширению или `null`, если расширение чужое. */
export function inputKind(path: string): InputKind | null {
  return INPUT_KIND_BY_EXTENSION[extensionOf(path)] ?? null;
}

/** Формат вывода по расширению или `null`. */
export function formatFromPath(path: string): OutputFormat | null {
  return FORMAT_BY_EXTENSION[extensionOf(path)] ?? null;
}

/** Разобрать значение `--format`; регистр не важен. */
export function parseFormat(value: string): OutputFormat {
  const normalized = value.replace(/^\./, '').toLowerCase();
  const format = FORMATS.find((candidate) => candidate === normalized);
  if (format === undefined) {
    throw new CliError(`unknown format '${value}': expected one of ${FORMATS.join(', ')}`);
  }
  return format;
}

/** Проверить, что пара «вход → выход» поддерживается. */
export function assertSupported(kind: InputKind, format: OutputFormat): void {
  if (!SUPPORTED[kind].includes(format)) {
    throw new CliError(
      `${kind} → ${format} is not supported: .${kind} can only be written to ${SUPPORTED[kind]
        .map((f) => OUTPUT_EXTENSION[f])
        .join(', ')}`,
    );
  }
}

/** Заменить расширение пути на расширение формата. */
export function withExtension(path: string, format: OutputFormat): string {
  const extension = OUTPUT_EXTENSION[format];
  const slash = path.lastIndexOf('/');
  const dot = path.lastIndexOf('.');
  return dot > slash ? `${path.slice(0, dot)}${extension}` : `${path}${extension}`;
}
