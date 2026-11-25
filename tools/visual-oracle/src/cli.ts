#!/usr/bin/env node
// CLI визуального оракула: `validate`, `reference import`, `run` и `capture`.
// Разбор аргументов ручной — зависимость ради четырёх подкоманд не нужна, а
// формат ошибок должен совпадать с остальными командами репозитория.
//
// Коды выхода: 0 — чисто, 1 — измеренное расхождение, 2 — ошибка аргументов
// или неполный захват, 3 — schema/semantic error (baseline-policy §6:
// сломанный контур не сертифицирует движок). У `capture` код 3 шире: там он
// значит ещё и сбой захвата, потому что отчёта, который несёт `capture-error`,
// эта подкоманда не пишет.

import { readdir, stat } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import {
  CorpusError,
  loadCorpus,
  type Corpus,
  type CorpusErrorEntry,
} from './corpus.js';
import {
  DEFAULT_FIXTURES_ROOT,
  DEFAULT_SCHEMA_PATH,
  validateManifestFile,
  type ManifestErrorEntry,
} from './manifest.js';
import {
  CaptureOnlyUsageError,
  runCaptureOnly,
  type CaptureOnlyOptions,
} from './capture_only.js';
import { resolveFromRepo } from './paths.js';
import type { Cause, RunReport } from './types.js';
import { ImportError, importReference } from './reference/import.js';
import { EmptySelectionError, parseOverrides, runOracle, type RunSummary } from './run.js';

/** Код выхода: успех. */
const EXIT_OK = 0;
/** Код выхода: ошибка аргументов. */
const EXIT_USAGE = 2;
/** Код выхода: schema/semantic error. */
const EXIT_INVALID = 3;

/** Справка верхнего уровня. */
const USAGE = `visual-oracle — offline visual oracle for the strict viewer certification

Usage:
  visual-oracle validate --corpus <path> [--references <dir>]
  visual-oracle reference import --from <dir> --corpus <path> [--force]
  visual-oracle run --corpus <path> --out <dir> [--filter <expr>]... [--override <expr>]...
  visual-oracle capture --doc <path.docx|path.xlsx> --out <dir> [--mode <mode>] [--port <n>] [--headed]
  visual-oracle --help

Commands:
  validate            Validate the corpus index and every manifest under the references root.
  reference import    Turn an operator capture drop into a schema-valid manifest.
  run                 Capture the viewer, compare with references, write report.json/report.html.
  capture             Capture one document with the local viewer; no reference, no manifest.

Exit codes:
  0  every capture matched
  1  visual-diff or page-mismatch
  2  invalid arguments, missing-reference or capture-error
  3  harness-error, schema or semantic error
`;

/** Справка `validate`. */
const VALIDATE_USAGE = `Usage: visual-oracle validate --corpus <path> [--references <dir>]

Validates the corpus index and every manifest.json found under the references
root (default: test-fixtures/strict-viewer), then cross-checks the two: every
manifest must belong to a declared fixture/mode, and an accepted corpus
(baselineId set) must have a manifest for each of them. A draft corpus
(baselineId null) may still be missing manifests; they are counted, not failed.

Options:
  --corpus <path>      Corpus index JSON (required).
  --references <dir>   Root of reference manifests (default: test-fixtures/strict-viewer).
  --fixtures <dir>     Root of source documents (default: test-fixtures).
  --schema <path>      Manifest JSON Schema (default: docs/strict-viewer/manifest.schema.json).
  --help               Show this help.
`;

/** Справка `reference import`. */
const REFERENCE_USAGE = `Usage: visual-oracle reference import --from <dir> --corpus <path> [--force]

Imports one operator capture drop laid out as <format>/<fixture-id>/<mode>/ with
capture.json and reference artifacts, and writes the manifest next to it.

Options:
  --from <dir>         Capture drop directory (required).
  --corpus <path>      Corpus index JSON (required).
  --references <dir>   Root of reference manifests (default: test-fixtures/strict-viewer).
  --fixtures <dir>     Root of source documents (default: test-fixtures).
  --schema <path>      Manifest JSON Schema (default: docs/strict-viewer/manifest.schema.json).
  --force              Overwrite an existing manifest.json.
  --help               Show this help.
`;

/** Справка `run`. */
const RUN_USAGE = `Usage: visual-oracle run --corpus <path> --out <dir> [options]

Captures every selected fixture/mode with the local viewer, compares each
capture against its reference, and writes report.json plus report.html into
--out. A fixture without a reference manifest is reported as missing-reference
and does not stop the run; the capture session (dev server + Chromium) is only
started when at least one reference exists.

Options:
  --corpus <path>      Corpus index JSON (required).
  --out <dir>          Output directory for the report and captures (required).
  --references <dir>   Root of reference manifests (default: corpus storage rootUri).
  --filter <expr>      Keep fixtures matching tag=<tag> or id=<glob>; repeatable, ANDed.
  --override <expr>    Force a cause: <fixtureId|fixtureId/captureId>=<cause>; repeatable.
  --port <number>      Dev server port (default: 5178).
  --headed             Show the browser window (debugging only).
  --help               Show this help.

Causes: parse, model, layout, font, render, harness, reference.
`;

/** Справка `capture`. */
const CAPTURE_USAGE = `Usage: visual-oracle capture --doc <path.docx|path.xlsx> --out <dir> [options]

Captures one document with the local viewer and writes PNGs into
<out>/<fixture-id>/<mode>/, next to runtime.json. This is a look-at-what-the-
engine-draws tool: no reference, no comparison and no manifest are involved, and
nothing is ever written under test-fixtures/strict-viewer.

The document must live under test-fixtures/<format>/ because the demo viewer
serves only that tree.

Options:
  --doc <path>         Document to capture, .docx or .xlsx (required).
  --out <dir>          Output directory for PNGs and runtime.json (required).
  --mode <mode>        View mode: print-layout (docx) or normal (xlsx).
                       Default: print-layout for docx, normal for xlsx.
                       pdf and print-preview need export and rasterization and
                       are rejected here.
  --port <number>      Dev server port (default: 5178).
  --headed             Show the browser window (debugging only).
  --help               Show this help.

Exit codes:
  0  PNGs written
  2  invalid arguments, missing document, unsupported mode
  3  capture failed (browser, timeout or viewer error)
`;

/** Разобранная команда. */
type Command =
  | { kind: 'help'; text: string }
  | { kind: 'validate'; options: ValidateOptions }
  | { kind: 'reference-import'; options: ReferenceImportOptions }
  | { kind: 'run'; options: RunCommandOptions }
  | { kind: 'capture'; options: CaptureOnlyOptions };

interface ValidateOptions {
  corpus: string;
  references: string;
  fixtures: string;
  schema: string;
}

interface ReferenceImportOptions {
  from: string;
  corpus: string;
  references: string;
  fixtures: string;
  schema: string;
  force: boolean;
}

interface RunCommandOptions {
  corpus: string;
  out: string;
  references: string | null;
  filters: string[];
  overrides: string[];
  port: number | null;
  headed: boolean;
}

/** Ошибка аргументов: печатается со справкой и кодом 2. */
class UsageError extends Error {
  readonly usage: string;

  constructor(message: string, usage: string) {
    super(message);
    this.name = 'UsageError';
    this.usage = usage;
  }
}

/** Точка входа CLI. */
export async function main(argv: readonly string[]): Promise<number> {
  let command: Command;
  try {
    command = parseArgs(argv);
  } catch (error) {
    if (error instanceof UsageError) {
      process.stderr.write(`error: ${error.message}\n\n${error.usage}`);
      return EXIT_USAGE;
    }
    throw error;
  }

  if (command.kind === 'help') {
    process.stdout.write(command.text);
    return EXIT_OK;
  }
  if (command.kind === 'validate') return runValidate(command.options);
  if (command.kind === 'run') return runRun(command.options);
  if (command.kind === 'capture') return runCapture(command.options);
  return runReferenceImport(command.options);
}

/** Разобрать аргументы без зависимостей. */
export function parseArgs(argv: readonly string[]): Command {
  const [first, ...rest] = argv;
  if (first === undefined || first === '--help' || first === '-h' || first === 'help') {
    return { kind: 'help', text: USAGE };
  }
  if (first === 'validate') {
    if (rest.includes('--help') || rest.includes('-h')) return { kind: 'help', text: VALIDATE_USAGE };
    const flags = parseFlags(rest, ['--corpus', '--references', '--fixtures', '--schema'], [], VALIDATE_USAGE);
    return {
      kind: 'validate',
      options: {
        corpus: requireFlag(flags, '--corpus', VALIDATE_USAGE),
        references: flags.get('--references') ?? 'test-fixtures/strict-viewer',
        fixtures: flags.get('--fixtures') ?? DEFAULT_FIXTURES_ROOT,
        schema: flags.get('--schema') ?? DEFAULT_SCHEMA_PATH,
      },
    };
  }
  if (first === 'run') {
    if (rest.includes('--help') || rest.includes('-h')) return { kind: 'help', text: RUN_USAGE };
    const flags = parseFlags(
      rest,
      ['--corpus', '--out', '--references', '--port'],
      ['--headed'],
      RUN_USAGE,
      ['--filter', '--override'],
    );
    const port = flags.get('--port');
    if (port !== undefined && !/^[0-9]+$/.test(port)) {
      throw new UsageError(`flag --port expects a number, got "${port}"`, RUN_USAGE);
    }
    return {
      kind: 'run',
      options: {
        corpus: requireFlag(flags, '--corpus', RUN_USAGE),
        out: requireFlag(flags, '--out', RUN_USAGE),
        references: flags.get('--references') ?? null,
        filters: flags.all('--filter'),
        overrides: flags.all('--override'),
        port: port === undefined ? null : Number(port),
        headed: flags.has('--headed'),
      },
    };
  }
  if (first === 'capture') {
    if (rest.includes('--help') || rest.includes('-h')) return { kind: 'help', text: CAPTURE_USAGE };
    const flags = parseFlags(rest, ['--doc', '--out', '--mode', '--port'], ['--headed'], CAPTURE_USAGE);
    const port = flags.get('--port');
    if (port !== undefined && !/^[0-9]+$/.test(port)) {
      throw new UsageError(`flag --port expects a number, got "${port}"`, CAPTURE_USAGE);
    }
    return {
      kind: 'capture',
      options: {
        doc: requireFlag(flags, '--doc', CAPTURE_USAGE),
        out: requireFlag(flags, '--out', CAPTURE_USAGE),
        mode: flags.get('--mode') ?? null,
        port: port === undefined ? null : Number(port),
        headed: flags.has('--headed'),
      },
    };
  }
  if (first === 'reference') {
    const [sub, ...tail] = rest;
    if (sub === undefined || sub === '--help' || sub === '-h') {
      return { kind: 'help', text: REFERENCE_USAGE };
    }
    if (sub !== 'import') {
      throw new UsageError(`unknown reference subcommand "${sub}"`, REFERENCE_USAGE);
    }
    if (tail.includes('--help') || tail.includes('-h')) {
      return { kind: 'help', text: REFERENCE_USAGE };
    }
    const flags = parseFlags(
      tail,
      ['--from', '--corpus', '--references', '--fixtures', '--schema'],
      ['--force'],
      REFERENCE_USAGE,
    );
    return {
      kind: 'reference-import',
      options: {
        from: requireFlag(flags, '--from', REFERENCE_USAGE),
        corpus: requireFlag(flags, '--corpus', REFERENCE_USAGE),
        references: flags.get('--references') ?? 'test-fixtures/strict-viewer',
        fixtures: flags.get('--fixtures') ?? DEFAULT_FIXTURES_ROOT,
        schema: flags.get('--schema') ?? DEFAULT_SCHEMA_PATH,
        force: flags.has('--force'),
      },
    };
  }
  throw new UsageError(`unknown command "${first}"`, USAGE);
}

/** Разобранные флаги: одиночные значения и повторяемые списки. */
interface Flags {
  /** Последнее значение флага либо `undefined`. */
  get(name: string): string | undefined;
  /** Все значения повторяемого флага в порядке появления. */
  all(name: string): string[];
  /** Был ли булев флаг. */
  has(name: string): boolean;
}

/**
 * Разобрать `--flag value`, булевы флаги и повторяемые флаги.
 *
 * Повторяемые флаги (`--filter`, `--override`) собираются в список: у них
 * смысл в накоплении, и «последний победил» молча выбросил бы фильтры.
 */
function parseFlags(
  argv: readonly string[],
  valueFlags: readonly string[],
  booleanFlags: readonly string[],
  usage: string,
  repeatableFlags: readonly string[] = [],
): Flags {
  const values = new Map<string, string>();
  const lists = new Map<string, string[]>();
  const booleans = new Set<string>();
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === undefined) continue;
    if (booleanFlags.includes(arg)) {
      booleans.add(arg);
      continue;
    }
    if (valueFlags.includes(arg) || repeatableFlags.includes(arg)) {
      const value = argv[index + 1];
      if (value === undefined || value.startsWith('--')) {
        throw new UsageError(`flag ${arg} requires a value`, usage);
      }
      if (repeatableFlags.includes(arg)) {
        const list = lists.get(arg) ?? [];
        list.push(value);
        lists.set(arg, list);
      } else {
        values.set(arg, value);
      }
      index += 1;
      continue;
    }
    throw new UsageError(`unknown argument "${arg}"`, usage);
  }
  return {
    get: (name) => values.get(name),
    all: (name) => lists.get(name) ?? [],
    has: (name) => booleans.has(name),
  };
}

function requireFlag(flags: Flags, name: string, usage: string): string {
  const value = flags.get(name);
  if (value === undefined) throw new UsageError(`missing required flag ${name}`, usage);
  return value;
}

/** `validate`: corpus index плюс все manifest под корнем эталонов. */
async function runValidate(options: ValidateOptions): Promise<number> {
  const rows: ErrorRow[] = [];
  let missing = 0;

  const corpusPath = resolveFromRepo(options.corpus);
  let corpus: Corpus | null = null;
  try {
    corpus = await loadCorpus(corpusPath);
  } catch (error) {
    if (error instanceof EmptySelectionError) {
      process.stderr.write(`error: ${errorText(error)}\n\n${RUN_USAGE}`);
      return EXIT_USAGE;
    }
    if (error instanceof CorpusError) {
      rows.push(...error.errors.map((entry) => corpusRow(corpusPath, entry)));
    } else {
      rows.push({ source: corpusPath, path: '', message: errorText(error) });
    }
  }

  const referencesRoot = resolveFromRepo(options.references);
  const manifests = await findManifests(referencesRoot);
  if (manifests === null) {
    rows.push({ source: referencesRoot, path: '', message: 'references root does not exist' });
  } else {
    for (const manifestPath of manifests) {
      const result = await validateManifestFile(manifestPath, {
        fixturesRoot: resolveFromRepo(options.fixtures),
        schemaPath: resolveFromRepo(options.schema),
        storageRoot: referencesRoot,
      });
      if (!result.ok) {
        rows.push(...result.errors.map((entry) => manifestRow(manifestPath, entry)));
      }
    }
    // Corpus index и каталог эталонов обязаны описывать одно и то же
    // множество: пропущенный manifest — незакрытая строка матрицы, лишний —
    // эталон, которого нет в корпусе (baseline-policy §4).
    if (corpus !== null) {
      const cross = crossCheckCorpus(corpus, referencesRoot, manifests);
      rows.push(...cross.errors);
      missing = cross.missing;
    }
  }

  if (rows.length === 0) {
    process.stdout.write(
      `visual-oracle: corpus and ${manifests?.length ?? 0} manifest(s) are valid\n`,
    );
    if (missing > 0) {
      // Черновой корпус (baselineId = null) ещё не обязан быть полным:
      // эталоны доснимаются по одной фикстуре. Принятый baseline — обязан.
      process.stdout.write(
        `visual-oracle: draft corpus, ${missing} fixture/mode pair(s) still have no manifest\n`,
      );
    }
    return EXIT_OK;
  }
  process.stderr.write(formatTable(rows));
  process.stderr.write(`\nvisual-oracle: ${rows.length} error(s)\n`);
  return EXIT_INVALID;
}

/** `run`: захват, сравнение и отчёт. */
async function runRun(options: RunCommandOptions): Promise<number> {
  let overrides: Record<string, Cause>;
  try {
    overrides = parseOverrides(options.overrides);
  } catch (error) {
    // Опечатка в причине — ошибка аргументов, а не прогона: отчёт с
    // выдуманной причиной хуже отсутствия отчёта.
    process.stderr.write(`error: ${errorText(error)}\n\n${RUN_USAGE}`);
    return EXIT_USAGE;
  }

  let summary: RunSummary;
  try {
    summary = await runOracle({
      corpusPath: options.corpus,
      outDir: options.out,
      ...(options.references === null ? {} : { referencesRoot: options.references }),
      filters: options.filters,
      overrides,
      ...(options.port === null ? {} : { port: options.port }),
      ...(options.headed ? { headless: false } : {}),
    });
  } catch (error) {
    if (error instanceof EmptySelectionError) {
      process.stderr.write(`error: ${errorText(error)}\n\n${RUN_USAGE}`);
      return EXIT_USAGE;
    }
    if (error instanceof CorpusError) {
      process.stderr.write(formatTable(error.errors.map((entry) => corpusRow(resolveFromRepo(options.corpus), entry))));
      process.stderr.write(`\nvisual-oracle: ${error.errors.length} error(s)\n`);
      return EXIT_INVALID;
    }
    process.stderr.write(`error: ${errorText(error)}\n`);
    return EXIT_INVALID;
  }

  process.stdout.write(formatResults(summary.report));
  process.stdout.write(`\nreport: ${summary.reportJsonPath}\n`);
  process.stdout.write(`html:   ${summary.reportHtmlPath}\n`);
  return summary.exitCode;
}

/**
 * `capture`: один документ без эталона; справка печатается только на ошибку
 * аргументов.
 *
 * Признак — имя ошибки, а не `instanceof`: модуль может оказаться загружен
 * дважды (src и dist в одном процессе тестов), и тогда `instanceof` молча
 * отправил бы ошибку аргументов в код 3.
 */
async function runCapture(options: CaptureOnlyOptions): Promise<number> {
  try {
    return await runCaptureOnly(options);
  } catch (error) {
    if (error instanceof CaptureOnlyUsageError || (error instanceof Error && error.name === 'CaptureOnlyUsageError')) {
      process.stderr.write(`error: ${errorText(error)}\n\n${CAPTURE_USAGE}`);
      return EXIT_USAGE;
    }
    process.stderr.write(`error: ${errorText(error)}\n`);
    return EXIT_INVALID;
  }
}

/** Таблица результатов: по строке на capture, затем итоги. */
function formatResults(report: RunReport): string {
  const rows = report.captures.map((capture) => ({
    source: `${capture.fixtureId}/${capture.captureId}`,
    path: capture.mode,
    message: capture.resultType === 'match' ? 'match' : `${capture.resultType} (${capture.cause})`,
  }));
  const lines = rows.length === 0 ? '' : formatTable(rows);
  const summary = report.summary;
  const totals = [
    `total ${summary.total}`,
    `match ${summary.match}`,
    `visual-diff ${summary['visual-diff']}`,
    `page-mismatch ${summary['page-mismatch']}`,
    `missing-reference ${summary['missing-reference']}`,
    `capture-error ${summary['capture-error']}`,
    `harness-error ${summary['harness-error']}`,
  ].join(', ');
  return `${lines}\nvisual-oracle: ${totals}\n`;
}

/** `reference import`: один операторский захват → manifest. */
async function runReferenceImport(options: ReferenceImportOptions): Promise<number> {
  try {
    const result = await importReference({
      from: resolveFromRepo(options.from),
      corpusPath: resolveFromRepo(options.corpus),
      rootUri: resolveFromRepo(options.references),
      fixturesRoot: resolveFromRepo(options.fixtures),
      schemaPath: resolveFromRepo(options.schema),
      ...(options.force ? { force: true } : {}),
    });
    if (result.previousBaselineId !== null) {
      process.stdout.write(`previous baselineId: ${result.previousBaselineId}\n`);
    }
    process.stdout.write(`baselineId: ${result.baselineId}\n`);
    process.stdout.write(
      `wrote ${result.manifestPath} (${result.captureCount} capture(s))\n`,
    );
    return EXIT_OK;
  } catch (error) {
    if (error instanceof ImportError) {
      process.stderr.write(`error: ${error.message}\n`);
      return EXIT_INVALID;
    }
    throw error;
  }
}

/**
 * Сверить corpus index с найденными manifest.
 *
 * Лишний manifest — всегда ошибка: эталон, которого нет в корпусе, не
 * сертифицирует ничего. Пропущенный manifest — ошибка только у принятого
 * корпуса (`baselineId` задан): черновой корпус доснимается по фикстурам, и
 * запрещать промежуточное состояние значило бы требовать всё сразу.
 */
function crossCheckCorpus(
  corpus: Corpus,
  referencesRoot: string,
  manifests: readonly string[],
): { errors: ErrorRow[]; missing: number } {
  const errors: ErrorRow[] = [];
  const expected = new Set<string>();
  let missing = 0;
  for (const fixture of corpus.fixtures) {
    for (const mode of fixture.modes) {
      const manifestPath = join(referencesRoot, fixture.format, fixture.id, mode, 'manifest.json');
      expected.add(manifestPath);
      if (manifests.includes(manifestPath)) continue;
      missing += 1;
      if (corpus.corpus.baselineId !== null) {
        errors.push({
          source: manifestPath,
          path: '',
          message: `manifest is missing for corpus fixture "${fixture.id}" mode "${mode}"`,
        });
      }
    }
  }
  for (const manifestPath of manifests) {
    if (!expected.has(manifestPath)) {
      errors.push({
        source: manifestPath,
        path: '',
        message: 'manifest does not belong to any fixture/mode declared in the corpus index',
      });
    }
  }
  return { errors, missing };
}

/** Строка таблицы ошибок. */
interface ErrorRow {
  source: string;
  path: string;
  message: string;
}

function corpusRow(source: string, entry: CorpusErrorEntry): ErrorRow {
  return { source, path: entry.path, message: entry.message };
}

function manifestRow(source: string, entry: ManifestErrorEntry): ErrorRow {
  return { source, path: entry.path, message: entry.message };
}

/** Найти все `manifest.json` под корнем; `null`, если корня нет. */
async function findManifests(root: string): Promise<string[] | null> {
  try {
    const info = await stat(root);
    if (!info.isDirectory()) return null;
  } catch {
    return null;
  }
  const found: string[] = [];
  const walk = async (dir: string): Promise<void> => {
    const entries = await readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) await walk(path);
      else if (entry.isFile() && entry.name === 'manifest.json') found.push(path);
    }
  };
  await walk(root);
  return found.sort();
}

/** Выровнять колонки: путь и сообщение читаются в терминале и в CI-логе. */
function formatTable(rows: readonly ErrorRow[]): string {
  const widths = {
    source: Math.max(...rows.map((row) => row.source.length)),
    path: Math.max(...rows.map((row) => row.path.length)),
  };
  return rows
    .map(
      (row) =>
        `${row.source.padEnd(widths.source)}  ${row.path.padEnd(widths.path)}  ${row.message}\n`,
    )
    .join('');
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// Запуск как `node dist/cli.js`: модуль не должен исполняться при импорте
// из тестов, поэтому точку входа проверяем по argv[1].
const entry = process.argv[1];
if (entry !== undefined && import.meta.url === pathToFileURL(resolve(entry)).href) {
  main(process.argv.slice(2)).then(
    (code) => {
      process.exitCode = code;
    },
    (error: unknown) => {
      process.stderr.write(`error: ${errorText(error)}\n`);
      process.exitCode = EXIT_INVALID;
    },
  );
}
