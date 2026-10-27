/**
 * `run` целиком: разбор аргументов, настоящая конвертация и вывод.
 *
 * Потоки подменяются объектом с одним `write`, поэтому тест не трогает
 * `process.stdout`/`process.stderr` и работает в любом раннере.
 */

import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import type { ConversionStats } from '@meerrec/core';

import type { OutputStream } from '../src/progress.js';
import { run, type CliStreams } from '../src/run.js';

const repoRoot = fileURLToPath(new URL('../../..', import.meta.url));
const DOCX = join(repoRoot, 'test-fixtures/docx/simple/one_paragraph.docx');
const XLSX = join(repoRoot, 'test-fixtures/xlsx/content-single-cell.xlsx');

let outDir: string;

beforeAll(async () => {
  outDir = await mkdtemp(join(tmpdir(), 'doc-converter-run-'));
});

afterAll(async () => {
  await rm(outDir, { recursive: true, force: true });
});

interface Capture {
  streams: CliStreams;
  stdout(): string;
  stderr(): string;
}

function capture(): Capture {
  const out: string[] = [];
  const err: string[] = [];
  const sink = (chunks: string[]): OutputStream => ({
    write(chunk: string) {
      chunks.push(chunk);
      return true;
    },
  });
  return {
    streams: { stdout: sink(out), stderr: sink(err) },
    stdout: () => out.join(''),
    stderr: () => err.join(''),
  };
}

interface JsonSuccess {
  ok: true;
  input: string;
  output: string;
  bytes: number;
  ms: number;
  stats: ConversionStats;
}

interface JsonFailure {
  ok: false;
  input: string;
  diagnostic: { code: string; layer: string; message: string };
}

describe('run', () => {
  it('convert --json: один JSON-объект в stdout и тишина в stderr', async () => {
    const output = join(outDir, 'run-json.pdf');
    const captured = capture();

    const code = await run(['convert', DOCX, '-o', output, '--json'], captured.streams);

    expect(code).toBe(0);
    expect(captured.stderr()).toBe('');
    const parsed = JSON.parse(captured.stdout()) as JsonSuccess;
    expect(parsed).toMatchObject({ ok: true, input: DOCX, output });
    expect(parsed.bytes).toBe((await stat(output)).size);
    expect(parsed.stats).toEqual({
      format: 'docx',
      bytesIn: (await stat(DOCX)).size,
      bytesOut: parsed.bytes,
      ms: parsed.ms,
    });
    expect((await readFile(output)).subarray(0, 5).toString('latin1')).toBe('%PDF-');
  });

  it('convert --json при ошибке пишет diagnostic и возвращает 2', async () => {
    const captured = capture();

    const code = await run(['convert', XLSX, '-o', join(outDir, 'nope.md'), '--json'], captured.streams);

    expect(code).toBe(2);
    expect(captured.stderr()).toBe('');
    const parsed = JSON.parse(captured.stdout()) as JsonFailure;
    expect(parsed.ok).toBe(false);
    expect(parsed.input).toBe(XLSX);
    expect(parsed.diagnostic).toMatchObject({ code: 'unsupported-format', layer: 'orchestration' });
  });

  it('без --json печатает ok в stderr, а stdout оставляет пустым', async () => {
    const output = join(outDir, 'run-human.pdf');
    const captured = capture();

    const code = await run(['convert', DOCX, '-o', output], captured.streams);

    expect(code).toBe(0);
    expect(captured.stdout()).toBe('');
    expect(captured.stderr()).toContain(`ok ${output}`);
  });

  it('без --json печатает диагностику и подсказку про help', async () => {
    const captured = capture();

    const code = await run(['convert', XLSX, '-o', join(outDir, 'nope.md')], captured.streams);

    expect(code).toBe(2);
    expect(captured.stdout()).toBe('');
    expect(captured.stderr()).toContain('error: orchestration/unsupported-format: xlsx → md is not supported');
    expect(captured.stderr()).toContain("run 'doc-converter help' for usage");
  });
});
