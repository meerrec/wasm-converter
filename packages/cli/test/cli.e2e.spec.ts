/**
 * Сквозной прогон собранного bin: `node dist/cli.js`.
 *
 * `dist` собирается перед тестами (`turbo run test` зависит от `build`), а
 * прямой `vitest` без сборки просто пропускает этот файл — проверять тут
 * нечего, пока bin не собран.
 */

import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const run = promisify(execFile);
const packageRoot = fileURLToPath(new URL('..', import.meta.url));
const CLI = join(packageRoot, 'dist/cli.js');
const repoRoot = fileURLToPath(new URL('../../..', import.meta.url));
const DOCX = join(repoRoot, 'test-fixtures/docx/simple/one_paragraph.docx');

let outDir: string;

beforeAll(async () => {
  outDir = await mkdtemp(join(tmpdir(), 'doc-converter-e2e-'));
});

afterAll(async () => {
  await rm(outDir, { recursive: true, force: true });
});

describe.skipIf(!existsSync(CLI))('dist/cli.js', () => {
  it('convert DOCX → PDF', async () => {
    const output = join(outDir, 'one_paragraph.pdf');
    const { stderr } = await run('node', [CLI, 'convert', DOCX, '-o', output]);
    expect(stderr).toContain('ok');
    const { stdout } = await run('head', ['-c', '5', output]);
    expect(stdout).toBe('%PDF-');
  });

  it('convert DOCX → Markdown', async () => {
    const output = join(outDir, 'one_paragraph.md');
    await run('node', [CLI, 'convert', DOCX, '-o', output]);
    const { stdout } = await run('cat', [output]);
    expect(stdout).toContain('Hello, World!');
  });

  it('batch каталога с --workers 4 печатает прогресс', async () => {
    const inputDir = join(repoRoot, 'test-fixtures/docx/simple');
    const pdfDir = join(outDir, 'pdf');
    const { stderr } = await run('node', [CLI, 'batch', inputDir, '--out', pdfDir, '--workers', '4']);

    // Прогресс идёт построчно, когда stderr не терминал.
    expect(stderr).toMatch(/ok\s+1\/\d+/);
    expect(stderr).toMatch(/done: \d+\/\d+ ok, 0 failed/);

    const { stdout } = await run('ls', [pdfDir]);
    const files = stdout.trim().split('\n');
    expect(files.length).toBeGreaterThan(1);
    expect(files.every((name) => name.endsWith('.pdf'))).toBe(true);
  });

  it('неподдерживаемая пара — код возврата 2 и текст ошибки', async () => {
    const xlsx = join(repoRoot, 'test-fixtures/xlsx/content-single-cell.xlsx');
    await expect(run('node', [CLI, 'convert', xlsx, '-o', join(outDir, 'nope.md')])).rejects.toMatchObject({
      code: 2,
      stderr: expect.stringContaining('xlsx → md is not supported') as unknown as string,
    });
  });
});
