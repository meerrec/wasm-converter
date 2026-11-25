import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import {
  CorpusError,
  filterFixtures,
  fixtureById,
  loadCorpus,
  slug,
  validateCorpus,
  type Corpus,
  type CorpusFixture,
} from '../src/corpus.js';

const tempDirs: string[] = [];

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'visual-oracle-corpus-'));
  tempDirs.push(dir);
  return dir;
}

/** Минимальная валидная фикстура: остальные тесты меняют одно поле. */
function fixture(overrides: Partial<CorpusFixture> = {}): CorpusFixture {
  return {
    id: 'docx-tables-long-table',
    format: 'docx',
    source: { path: 'docx/tables/long_table.docx' },
    title: 'Long table',
    tags: ['table', 'pagination'],
    matrixRows: ['docx.tables-pagination'],
    expected: 'Header repeats on every continuation page.',
    modes: ['print-layout'],
    ...overrides,
  };
}

/** Минимальный валидный corpus index. */
function corpusValue(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schemaVersion: '1.0.0',
    corpus: {
      id: 'strict-viewer-1.0.0',
      version: '1.0.0',
      matrixVersion: '1.0.0',
      baselineId: null,
      storage: { kind: 'git-lfs', rootUri: 'test-fixtures/strict-viewer' },
    },
    fixtures: [fixture()],
    gaps: [{ matrixRow: 'docx.sections-columns', missing: 'columns', reason: 'no fixture yet' }],
    ...overrides,
  };
}

describe('slug', () => {
  it('строит id из пути источника', () => {
    expect(slug('docx/tables/long_table.docx')).toBe('docx-tables-long-table');
    expect(slug('xlsx/charts-five-kinds.xlsx')).toBe('xlsx-charts-five-kinds');
  });

  it('детерминирован и не зависит от регистра и разделителей', () => {
    expect(slug('DOCX/Tables/Long__Table.DOCX')).toBe(slug('docx/tables/long_table.docx'));
    expect(slug('a//b///c.docx')).toBe('a-b-c');
  });

  it('обрезает ведущие и хвостовые дефисы', () => {
    expect(slug('_leading.docx')).toBe('leading');
    expect(slug('trailing_.docx')).toBe('trailing');
  });
});

describe('filterFixtures', () => {
  const fixtures: CorpusFixture[] = [
    fixture(),
    fixture({
      id: 'docx-basic-sections',
      source: { path: 'docx/basic/sections.docx' },
      tags: ['sections'],
      matrixRows: ['docx.sections-columns'],
    }),
    fixture({
      id: 'xlsx-charts-five-kinds',
      format: 'xlsx',
      source: { path: 'xlsx/charts-five-kinds.xlsx' },
      tags: ['charts'],
      matrixRows: ['xlsx.charts'],
      modes: ['normal', 'print-preview'],
    }),
  ];

  it('без фильтров возвращает всё', () => {
    expect(filterFixtures(fixtures, [])).toHaveLength(3);
  });

  it('фильтрует по тегу', () => {
    expect(filterFixtures(fixtures, ['tag=table']).map((item) => item.id)).toEqual([
      'docx-tables-long-table',
    ]);
  });

  it('фильтрует по glob id', () => {
    expect(filterFixtures(fixtures, ['id=docx-*']).map((item) => item.id)).toEqual([
      'docx-tables-long-table',
      'docx-basic-sections',
    ]);
    expect(filterFixtures(fixtures, ['id=*-five-kinds']).map((item) => item.id)).toEqual([
      'xlsx-charts-five-kinds',
    ]);
  });

  it('объединяет фильтры по «И»', () => {
    expect(filterFixtures(fixtures, ['tag=table', 'id=xlsx-*'])).toEqual([]);
  });

  it('отвергает неизвестный вид фильтра', () => {
    expect(() => filterFixtures(fixtures, ['mode=pdf'])).toThrow(/unknown kind/);
    expect(() => filterFixtures(fixtures, ['tag'])).toThrow(/invalid filter/);
  });
});

describe('validateCorpus', () => {
  it('принимает валидный corpus index', () => {
    const result = validateCorpus(corpusValue());
    expect(result.ok).toBe(true);
  });

  it('отвергает дубли id', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture(), fixture({ title: 'Second copy' })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.map((error) => error.message).join('\n')).toMatch(/duplicate fixture id/);
  });

  it('требует id, совпадающий со slug(source.path)', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture({ id: 'docx-wrong-id' })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => error.path === 'fixtures[0].id')).toBe(true);
  });

  it('требует префикс формата в пути источника', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture({ source: { path: 'xlsx/tables/long_table.docx' } })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /must start with "docx\/"/.test(error.message))).toBe(true);
  });

  it('отвергает режим, недопустимый для формата', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture({ modes: ['normal'] })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /not legal for docx/.test(error.message))).toBe(true);
  });

  it('требует непустые modes, tags и matrixRows', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture({ modes: [], tags: [], matrixRows: [] })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    const paths = result.errors.map((error) => error.path);
    expect(paths).toContain('fixtures[0].modes');
    expect(paths).toContain('fixtures[0].tags');
    expect(paths).toContain('fixtures[0].matrixRows');
  });

  it('отвергает дубль режима в одной фикстуре', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture({ modes: ['print-layout', 'print-layout'] })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /duplicate mode/.test(error.message))).toBe(true);
  });

  it('отвергает непереносимый путь источника', () => {
    const result = validateCorpus(
      corpusValue({ fixtures: [fixture({ source: { path: 'docx/../secret.docx' } })] }),
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.errors.some((error) => /portable relative path/.test(error.message))).toBe(true);
  });
});

describe('loadCorpus', () => {
  it('читает corpus index с диска', async () => {
    const dir = await tempDir();
    const path = join(dir, 'corpus.json');
    await writeFile(path, JSON.stringify(corpusValue()), 'utf8');
    const corpus: Corpus = await loadCorpus(path);
    expect(corpus.corpus.id).toBe('strict-viewer-1.0.0');
    expect(fixtureById(corpus, 'docx-tables-long-table')?.tags).toEqual(['table', 'pagination']);
    expect(fixtureById(corpus, 'missing')).toBeUndefined();
  });

  it('бросает CorpusError со всеми нарушениями', async () => {
    const dir = await tempDir();
    const path = join(dir, 'corpus.json');
    await writeFile(path, JSON.stringify(corpusValue({ fixtures: [fixture({ tags: [] })] })), 'utf8');
    await expect(loadCorpus(path)).rejects.toBeInstanceOf(CorpusError);
  });

  it('бросает CorpusError на отсутствующий файл', async () => {
    const dir = await tempDir();
    await expect(loadCorpus(join(dir, 'nope.json'))).rejects.toBeInstanceOf(CorpusError);
  });
});
