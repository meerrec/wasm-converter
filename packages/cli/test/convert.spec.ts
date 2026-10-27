import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { convertFile } from '../src/convert.js';
import { CliError } from '../src/errors.js';

/** Корень репозитория: фикстуры лежат в `test-fixtures/`, не в пакете. */
const repoRoot = fileURLToPath(new URL('../../..', import.meta.url));
const DOCX = join(repoRoot, 'test-fixtures/docx/simple/one_paragraph.docx');
const XLSX = join(repoRoot, 'test-fixtures/xlsx/content-single-cell.xlsx');

let outDir: string;

beforeAll(async () => {
  outDir = await mkdtemp(join(tmpdir(), 'doc-converter-cli-'));
});

afterAll(async () => {
  await rm(outDir, { recursive: true, force: true });
});

describe('convertFile', () => {
  it('DOCX → PDF: настоящий PDF в выходном файле', async () => {
    const output = join(outDir, 'one_paragraph.pdf');
    const result = await convertFile({ input: DOCX, output, format: 'pdf', pdf: {} });

    const bytes = await readFile(output);
    const inputBytes = (await stat(DOCX)).size;
    expect(bytes.subarray(0, 5).toString('latin1')).toBe('%PDF-');
    expect(bytes.byteLength).toBeGreaterThan(1000);
    expect(result.bytes).toBe(bytes.byteLength);
    expect(result.ms).toBeGreaterThanOrEqual(0);
    // stats — общая форма браузера и CLI: формат входа и замеры в байтах.
    expect(result.stats).toEqual({
      format: 'docx',
      bytesIn: inputBytes,
      bytesOut: bytes.byteLength,
      ms: result.ms,
    });
  });

  it('DOCX → Markdown/HTML/текст: в тексте есть абзац документа', async () => {
    for (const [format, extension] of [
      ['md', '.md'],
      ['html', '.html'],
      ['txt', '.txt'],
    ] as const) {
      const output = join(outDir, `one_paragraph${extension}`);
      await convertFile({ input: DOCX, output, format, pdf: {} });
      const text = await readFile(output, 'utf8');
      expect(text).toContain('Hello, World!');
    }
  });

  it('XLSX → PDF: лист и книга целиком', async () => {
    const sheet = join(outDir, 'single-cell-sheet.pdf');
    await convertFile({ input: XLSX, output: sheet, format: 'pdf', pdf: { sheetIndex: 0 } });
    expect((await readFile(sheet)).subarray(0, 5).toString('latin1')).toBe('%PDF-');

    const book = join(outDir, 'single-cell-book.pdf');
    await convertFile({ input: XLSX, output: book, format: 'pdf', pdf: {} });
    expect((await readFile(book)).subarray(0, 5).toString('latin1')).toBe('%PDF-');

    const fitted = join(outDir, 'single-cell-fit.pdf');
    await convertFile({
      input: XLSX,
      output: fitted,
      format: 'pdf',
      pdf: { fitToPage: true, fitToHeight: 0 },
    });
    expect((await readFile(fitted)).subarray(0, 5).toString('latin1')).toBe('%PDF-');
  });

  it('XLSX в текст — явная ошибка с диагностикой, модуль даже не грузится', async () => {
    const attempt = convertFile({ input: XLSX, output: join(outDir, 'x.md'), format: 'md', pdf: {} });
    await expect(attempt).rejects.toThrow(/xlsx → md is not supported/);
    await expect(attempt).rejects.toBeInstanceOf(CliError);
    await expect(attempt).rejects.toMatchObject({
      exitCode: 2,
      diagnostic: { code: 'unsupported-format', layer: 'orchestration' },
    });
  });

  it('неподдерживаемый вход — CliError с кодом unsupported-format', async () => {
    const attempt = convertFile({ input: 'book.odt', output: join(outDir, 'book.pdf'), format: 'pdf', pdf: {} });
    await expect(attempt).rejects.toBeInstanceOf(CliError);
    await expect(attempt).rejects.toMatchObject({
      exitCode: 2,
      diagnostic: { code: 'unsupported-format', layer: 'orchestration' },
    });
  });

  it('не-OOXML вход — ошибка обёрнута в CliError с диагностикой internal/cli', async () => {
    const notADocx = join(outDir, 'broken.docx');
    await writeFile(notADocx, 'not a zip');
    const attempt = convertFile({ input: notADocx, output: join(outDir, 'broken.pdf'), format: 'pdf', pdf: {} });
    await expect(attempt).rejects.toThrow();
    await expect(attempt).rejects.toBeInstanceOf(CliError);
    await expect(attempt).rejects.toMatchObject({
      exitCode: 1,
      diagnostic: { code: 'internal', layer: 'cli' },
    });
  });
});
