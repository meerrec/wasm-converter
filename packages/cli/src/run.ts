/**
 * Исполнение разобранной команды: `convert` — в текущем процессе,
 * `batch` — в пуле воркеров.
 */

import { mkdir, stat } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { basename, join } from 'node:path';

import { parseArgs, USAGE, type BatchCommand, type ConvertCommand } from './args.js';
import { convertFile, type ConvertJob } from './convert.js';
import { CliError } from './errors.js';
import { expandInputs } from './expand.js';
import { assertSupported, formatFromPath, inputKind, withExtension } from './formats.js';
import { formatBytes, ProgressReporter } from './progress.js';
import { loadRuntime, type CliRuntime } from './runtime.js';

/** Код возврата: 0 — всё сошлось, 1 — файл(ы) не сконвертировались, 2 — ошибка вызова. */
export async function run(argv: readonly string[], streams: { stdout: NodeJS.WriteStream; stderr: NodeJS.WriteStream } = {
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
        return await runConvert(command, streams.stderr);
      case 'batch':
        return await runBatch(command, streams.stderr, runtime);
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    streams.stderr.write(`error: ${message}\n`);
    if (error instanceof CliError && error.exitCode === 2) {
      streams.stderr.write("run 'doc-converter help' for usage\n");
    }
    return error instanceof CliError ? error.exitCode : 1;
  }
}

async function runConvert(command: ConvertCommand, stderr: NodeJS.WriteStream): Promise<number> {
  const kind = inputKind(command.input);
  if (kind === null) {
    throw new CliError(`unsupported input '${command.input}': expected .docx or .xlsx`);
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
  stderr.write(`ok ${result.output} (${formatBytes(result.bytes)}, ${result.ms} ms)\n`);
  return 0;
}

async function runBatch(command: BatchCommand, stderr: NodeJS.WriteStream, runtime: CliRuntime): Promise<number> {
  const inputs = await expandInputs(command.inputs);
  const jobs = await planJobs(inputs, command);

  await mkdir(command.outDir, { recursive: true });

  const progress = new ProgressReporter(jobs.length, { stream: stderr, quiet: command.quiet });
  const pool = runtime.createPool(new URL('./worker.js', import.meta.url).href, command.workers);
  const started = Date.now();

  try {
    await Promise.all(
      jobs.map(async (job) => {
        try {
          const result = await pool.submit(job);
          progress.report({ ok: true, name: basename(job.input), detail: `${formatBytes(result.bytes)}, ${result.ms} ms` });
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          progress.report({ ok: false, name: basename(job.input), detail: message });
        }
      }),
    );
  } finally {
    await pool.close();
  }
  progress.finish();

  const seconds = ((Date.now() - started) / 1000).toFixed(2);
  const ok = jobs.length - progress.failed;
  stderr.write(`done: ${ok}/${jobs.length} ok, ${progress.failed} failed in ${seconds}s (${command.workers} workers)\n`);
  return progress.failed === 0 ? 0 : 1;
}

/** Проверить весь список до запуска воркеров: пары форматов и коллизии имён. */
async function planJobs(inputs: readonly string[], command: BatchCommand): Promise<ConvertJob[]> {
  const byOutput = new Map<string, string>();
  const jobs: ConvertJob[] = [];

  for (const input of inputs) {
    const kind = inputKind(input);
    if (kind === null) throw new CliError(`unsupported input '${input}': expected .docx or .xlsx`);
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
