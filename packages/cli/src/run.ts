/**
 * Исполнение разобранной команды: `convert` — в текущем процессе,
 * `batch` — в пуле воркеров.
 *
 * Человекочитаемый режим пишет ошибки в stderr как `error: <layer>/<code>: …`;
 * `--json` уводит машинный результат в stdout (у `convert` — один объект,
 * у `batch` — JSON Lines), оставляя stderr пустым.
 */

import { mkdir, stat } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { basename, join } from 'node:path';

import { formatDiagnostic, toDiagnostic, type Diagnostic } from '@meerrec/core';

import { parseArgs, USAGE, type BatchCommand, type ConvertCommand } from './args.js';
import { convertFile, type ConvertJob } from './convert.js';
import { CliError } from './errors.js';
import { expandInputs } from './expand.js';
import { assertSupported, formatFromPath, INPUT_EXTENSIONS, inputKind, withExtension } from './formats.js';
import { formatBytes, ProgressReporter, type OutputStream } from './progress.js';
import { loadRuntime, type CliRuntime } from './runtime.js';

/** Потоки, которыми пользуется `run`; тесты подменяют их без Node-типов. */
export interface CliStreams {
  stdout: OutputStream;
  stderr: OutputStream;
}

/** Код возврата: 0 — всё сошлось, 1 — файл(ы) не сконвертировались, 2 — ошибка вызова. */
export async function run(argv: readonly string[], streams: CliStreams = {
  stdout: process.stdout,
  stderr: process.stderr,
}): Promise<number> {
  try {
    const runtime = await loadRuntime();
    const command = parseArgs(argv, runtime);
    switch (command.kind) {
      case 'help':
        streams.stdout.write(USAGE);
        return 0;
      case 'version':
        streams.stdout.write(`${version()}\n`);
        return 0;
      case 'convert':
        return await runConvert(command, streams);
      case 'batch':
        return await runBatch(command, streams, runtime);
    }
  } catch (error) {
    const diagnostic = diagnosticOf(error);
    streams.stderr.write(`error: ${formatDiagnostic(diagnostic)}\n`);
    if (error instanceof CliError && error.exitCode === 2) {
      streams.stderr.write("run 'doc-converter help' for usage\n");
    }
    return error instanceof CliError ? error.exitCode : 1;
  }
}

/**
 * Машинная диагностика любого исключения.
 *
 * `CliError` могла прийти с готовой диагностикой (форматы, конвертация); если
 * её нет — достраиваем по коду возврата: 2 — ошибка выбора, 1 — отказ файла.
 */
function diagnosticOf(error: unknown): Diagnostic {
  if (error instanceof CliError) {
    if (error.diagnostic !== undefined) return error.diagnostic;
    return toDiagnostic(error.message, {
      code: error.exitCode === 2 ? 'unsupported-format' : 'internal',
      layer: 'cli',
    });
  }
  return toDiagnostic(error, { code: 'internal', layer: 'cli' });
}

/** Одна строка JSON без хвостового мусора — формат `--json`. */
function writeJsonLine(stream: OutputStream, value: Record<string, unknown>): void {
  stream.write(`${JSON.stringify(value)}\n`);
}

async function runConvert(command: ConvertCommand, streams: CliStreams): Promise<number> {
  try {
    const kind = inputKind(command.input);
    if (kind === null) {
      const message = `unsupported input '${command.input}': expected ${INPUT_EXTENSIONS}`;
      throw new CliError(message, 2, toDiagnostic(message, { code: 'unsupported-format', layer: 'orchestration' }));
    }

    const outIsDirectory = (await isDirectory(command.out)) || command.out.endsWith('/');
    const format = command.format ?? (outIsDirectory ? null : formatFromPath(command.out));
    if (format === null) {
      throw new CliError(`cannot tell the output format from '${command.out}': pass --format <${['pdf', 'md', 'html', 'txt'].join('|')}>`);
    }
    assertSupported(kind, format);

    const output = outIsDirectory ? join(command.out, withExtension(basename(command.input), format)) : command.out;
    await assertWritable(output, command.force);

    const result = await convertFile({ input: command.input, output, format, pdf: command.pdf });
    if (command.json) {
      writeJsonLine(streams.stdout, {
        ok: true,
        input: result.input,
        output: result.output,
        bytes: result.bytes,
        ms: result.ms,
        stats: result.stats,
      });
    } else {
      streams.stderr.write(`ok ${result.output} (${formatBytes(result.bytes)}, ${result.ms} ms)\n`);
    }
    return 0;
  } catch (error) {
    // В человекочитаемом режиме ошибку обрабатывает общий catch `run`:
    // там же подсказка про help для кода 2.
    if (!command.json) throw error;
    writeJsonLine(streams.stdout, { ok: false, input: command.input, diagnostic: diagnosticOf(error) });
    return error instanceof CliError ? error.exitCode : 1;
  }
}

async function runBatch(command: BatchCommand, streams: CliStreams, runtime: CliRuntime): Promise<number> {
  const inputs = await expandInputs(command.inputs);
  const jobs = await planJobs(inputs, command);

  await mkdir(command.outDir, { recursive: true });

  // В `--json` прогресс в stderr не нужен, но счётчик неудач остаётся:
  // его печатает итоговая строка и по нему считается код возврата.
  const progress = new ProgressReporter(jobs.length, { stream: streams.stderr, quiet: command.quiet || command.json });
  const pool = runtime.createPool(new URL('./worker.js', import.meta.url).href, command.workers);
  const started = Date.now();

  try {
    await Promise.all(
      jobs.map(async (job) => {
        const name = basename(job.input);
        try {
          const result = await pool.submit(job);
          progress.report({ ok: true, name, detail: `${formatBytes(result.bytes)}, ${result.ms} ms` });
          if (command.json) {
            writeJsonLine(streams.stdout, {
              ok: true,
              name,
              input: job.input,
              output: result.output,
              bytes: result.bytes,
              ms: result.ms,
              stats: result.stats,
            });
          }
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          progress.report({ ok: false, name, detail: message });
          if (command.json) {
            writeJsonLine(streams.stdout, { ok: false, name, input: job.input, diagnostic: diagnosticOf(error) });
          }
        }
      }),
    );
  } finally {
    await pool.close();
  }
  progress.finish();

  const ms = Date.now() - started;
  const seconds = (ms / 1000).toFixed(2);
  const ok = jobs.length - progress.failed;
  if (command.json) {
    writeJsonLine(streams.stdout, {
      ok: progress.failed === 0,
      total: jobs.length,
      failed: progress.failed,
      ms,
      workers: command.workers,
    });
  } else {
    streams.stderr.write(`done: ${ok}/${jobs.length} ok, ${progress.failed} failed in ${seconds}s (${command.workers} workers)\n`);
  }
  return progress.failed === 0 ? 0 : 1;
}

/** Проверить весь список до запуска воркеров: пары форматов и коллизии имён. */
async function planJobs(inputs: readonly string[], command: BatchCommand): Promise<ConvertJob[]> {
  const byOutput = new Map<string, string>();
  const jobs: ConvertJob[] = [];

  for (const input of inputs) {
    const kind = inputKind(input);
    if (kind === null) {
      const message = `unsupported input '${input}': expected ${INPUT_EXTENSIONS}`;
      throw new CliError(message, 2, toDiagnostic(message, { code: 'unsupported-format', layer: 'orchestration' }));
    }
    assertSupported(kind, command.format);

    const output = join(command.outDir, withExtension(basename(input), command.format));
    const previous = byOutput.get(output);
    if (previous !== undefined) {
      throw new CliError(`two inputs map to the same output '${output}': '${previous}' and '${input}'`);
    }
    byOutput.set(output, input);
    await assertWritable(output, command.force);
    jobs.push({ input, output, format: command.format, pdf: command.pdf });
  }

  return jobs;
}

async function assertWritable(output: string, force: boolean): Promise<void> {
  if (!force && (await exists(output))) {
    throw new CliError(`output exists: ${output} (use --force to overwrite)`);
  }
}

async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

async function isDirectory(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isDirectory();
  } catch {
    return false;
  }
}

/** Версия пакета: `package.json` лежит на уровень выше `dist/`. */
function version(): string {
  const require = createRequire(import.meta.url);
  const pkg = require('../package.json') as { version?: string };
  return pkg.version ?? '0.0.0';
}
