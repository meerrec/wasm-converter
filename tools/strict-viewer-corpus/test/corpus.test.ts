// Регрессии инструмента сверки корпуса: правило slug, разбор матрицы и
// вычисление кода выхода. Проверяются наблюдаемые результаты, а не текст
// исходников: расхождение здесь ломает соответствие `corpus.json` ↔ матрица.

import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { problems, slug, type FixtureInput } from '../add.js';
import { formatReport, parseMatrix, summarize, type CorpusFixture } from '../report.js';

// Корень репозитория для чтения настоящих файлов. Витест запускается из
// корня пакета, поэтому `..` внутри `path.resolve` не используется: корень
// получается срезом суффикса пакета, остаётся абсолютным и не зависит от
// способа запуска.
const ROOT = process.cwd().replace(/\/tools\/strict-viewer-corpus$/, '');
const CORPUS = path.join(ROOT, 'test-fixtures', 'strict-viewer', 'corpus.json');

const tempDirs: string[] = [];

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

/** Временный каталог, изображающий корень репозитория. */
async function tempRoot(): Promise<string> {
  const dir = await mkdtemp(path.join(tmpdir(), 'corpus-root-'));
  tempDirs.push(dir);
  return dir;
}

function fixture(overrides: Partial<CorpusFixture> = {}): CorpusFixture {
  return {
    id: 'docx-tables-long-table',
    format: 'docx',
    source: { path: 'docx/tables/long_table.docx' },
    modes: ['print-layout'],
    ...overrides,
  };
}

function input(overrides: Partial<FixtureInput> = {}): FixtureInput {
  return {
    path: 'docx/tables/long_table.docx',
    title: 'Long table',
    tags: ['tables'],
    matrixRows: ['docx.tables-pagination'],
    expected: 'Шапка повторяется на каждой странице продолжения.',
    modes: ['print-layout'],
    ...overrides,
  };
}

describe('slug', () => {
  it('превращает путь фикстуры в id реестра', () => {
    expect(slug('docx/tables/long_table.docx')).toBe('docx-tables-long-table');
    expect(slug('xlsx/layout-shrink-to-fit.xlsx')).toBe('xlsx-layout-shrink-to-fit');
  });

  it('опускает регистр и схлопывает серии разделителей', () => {
    expect(slug('xlsx/Layout__Column Widths.XLSX')).toBe('xlsx-layout-column-widths');
  });

  it('срезает разделители по краям', () => {
    expect(slug('/docx/_basic_/multiple-runs.docx')).toBe('docx-basic-multiple-runs');
    expect(slug('__edge__')).toBe('edge');
  });
});

describe('parseMatrix', () => {
  it('берёт только строки DOCX/XLSX и читает минимальную фикстуру', () => {
    const rows = parseMatrix(
      [
        '| `docx.tables-pagination` | long tables | PL, PDF | `supported` | `docx/tables/long_table.docx` |',
        '| `xlsx.charts` | charts | N, PP | `supported` | `xlsx/charts-five-kinds.xlsx`; `требуется новая: combined chart` |',
        '| `docx.math` | math | PL, PDF | `unsupported-visible` | `требуется новая` |',
        '| не строка матрицы |',
      ].join('\n'),
    );
    expect(rows).toEqual([
      { rowId: 'docx.tables-pagination', fixture: 'docx/tables/long_table.docx' },
      { rowId: 'xlsx.charts', fixture: 'xlsx/charts-five-kinds.xlsx' },
      { rowId: 'docx.math', fixture: null },
    ]);
  });

  it('считает реальную матрицу сертификации', async () => {
    const text = await readFile(path.join(ROOT, 'docs/strict-viewer/certification-matrix.md'), 'utf8');
    const corpus = JSON.parse(await readFile(CORPUS, 'utf8')) as { fixtures: CorpusFixture[] };

    const rows = parseMatrix(text);
    const report = summarize(rows, corpus.fixtures, () => true);
    // Матрица не должна нести незакрытых пробелов: каждая из 42 строк DOCX/XLSX
    // называет минимальную фикстуру (`требуется новая` в минимальной колонке
    // запрещена). Иначе счётчики ниже поедут молча.
    expect(rows.filter((row) => row.fixture === null)).toEqual([]);
    expect(new Set(rows.map((row) => row.rowId)).size).toBe(rows.length);
    // Покрытие матрицы складывается из реестра без остатка.
    expect(report.uniqueMinimalFiles).toBeLessThanOrEqual(report.total);
    expect(report).toMatchObject({
      matrixRows: 42,
      rowsWithFixture: 42,
      uniqueMinimalFiles: 41,
      docx: 41,
      xlsx: 35,
      total: 76,
      exitCode: 0,
    });
    expect(report.docx + report.xlsx).toBe(report.total);
    expect(report.missingFromCorpus).toEqual([]);
    expect(report.duplicateIds).toEqual([]);
    expect(report.duplicatePaths).toEqual([]);
  });
});

describe('summarize', () => {
  it('не жалуется, когда все минимальные фикстуры есть в реестре и на диске', () => {
    const rows = [{ rowId: 'docx.tables-pagination', fixture: 'docx/tables/long_table.docx' }];
    const report = summarize(rows, [fixture()], () => true);
    expect(report.exitCode).toBe(0);
    expect(formatReport(report)).toContain('реестр: 1 DOCX + 0 XLSX = 1');
  });

  it('считает отсутствие в реестре проблемой и валит код выхода', () => {
    const rows = [{ rowId: 'docx.tables-pagination', fixture: 'docx/tables/long_table.docx' }];
    const report = summarize(rows, [], () => false);
    expect(report.missingFromCorpus).toEqual(['docx/tables/long_table.docx']);
    expect(report.missingOnDisk).toEqual(['docx/tables/long_table.docx']);
    expect(report.exitCode).not.toBe(0);
    expect(formatReport(report)).toContain('минимальных нет в реестре: docx/tables/long_table.docx');
  });

  it('валит код выхода на дублях id и путей', () => {
    const rows = [{ rowId: 'docx.tables-pagination', fixture: null }];
    const report = summarize(rows, [fixture(), fixture()], () => true);
    expect(report.duplicateIds).toEqual(['docx-tables-long-table']);
    expect(report.duplicatePaths).toEqual(['docx/tables/long_table.docx']);
    expect(report.exitCode).not.toBe(0);
  });
});

describe('problems', () => {
  it('пропускает запись, у которой файл есть на диске', async () => {
    await expect(problems(input(), new Set(), new Set(), ROOT)).resolves.toEqual([]);
  });

  it('сообщает об отсутствующем файле и делает код выхода ненулевым', async () => {
    const missing = input({ path: 'docx/tables/missing_from_disk.docx' });
    const issues = await problems(missing, new Set(), new Set(), ROOT);
    expect(issues).toContain('docx/tables/missing_from_disk.docx: файла нет на диске');

    // Тот же путь, названный в матрице, но не попавший в реестр: summarize
    // обязан увидеть проблему и вернуть ненулевой код — то, по чему add.ts
    // отказывается дописывать реестр.
    const report = summarize([{ rowId: 'docx.tables-pagination', fixture: missing.path }], [], () => false);
    expect(report.missingOnDisk).toContain(missing.path);
    expect(report.exitCode).not.toBe(0);
  });

  it('ищет фикстуру относительно переданного корня', async () => {
    const root = await tempRoot();
    await mkdir(path.join(root, 'test-fixtures', 'docx', 'tables'), { recursive: true });
    await writeFile(path.join(root, 'test-fixtures', 'docx', 'tables', 'long_table.docx'), '');

    await expect(problems(input(), new Set(), new Set(), root)).resolves.toEqual([]);

    const absent = input({ path: 'docx/tables/absent.docx' });
    await expect(problems(absent, new Set(), new Set(), root)).resolves.toContain(
      'docx/tables/absent.docx: файла нет на диске',
    );
  });

  it('проверяет словарь режимов, префикс, формат и дубли', async () => {
    const bad = input({
      path: 'No_Prefix.pdf',
      tags: [],
      matrixRows: [],
      modes: ['nonsense'],
    });
    const issues = await problems(bad, new Set(['No_Prefix.pdf']), new Set(['No_Prefix.pdf']), ROOT);
    expect([...issues].sort()).toEqual(
      [
        'No_Prefix.pdf: расширение .pdf, ожидается .docx или .xlsx',
        'No_Prefix.pdf: нет префикса формата (ожидается docx/… или xlsx/…)',
        'No_Prefix.pdf: путь уже в реестре',
        'No_Prefix.pdf: путь повторён в спецификации',
        'No_Prefix.pdf: неизвестный режим "nonsense"',
        'No_Prefix.pdf: пустой matrixRows',
        'No_Prefix.pdf: пустой tags',
        'No_Prefix.pdf: файла нет на диске',
      ].sort(),
    );
  });
});
