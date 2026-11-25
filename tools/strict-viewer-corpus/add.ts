// Досыпка записей в реестр strict-viewer корпуса.
//
// Зачем скрипт, а не правка руками: записей много (десятки), каждая обязана
// иметь точный slug (`id` = slug от `source.path`), уникальный путь и modes из
// принятого словаря. Ручная правка уже давала расхождения (потерянный `docx/`
// префикс, дубли путей), и такой скрипт — единственная защита от повтора.
//
// Запуск:
//   npx tsx tools/strict-viewer-corpus/add.ts --list specs.ts
//   npx tsx tools/strict-viewer-corpus/add.ts --list specs.ts --dry-run
//
// Файл спецификаций экспортирует `default` массив `FixtureInput` (см. ниже).
// Скрипт проверяет: файл существует, slug совпадает, путь не дублируется,
// `format` совпадает с расширением, `modes` из словаря. Затем дописывает в
// `corpus.json`, сохраняя порядок DOCX-записей перед XLSX.

import { readFile, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

// То же правило, что в `report.ts`: корень — два уровня вверх от файла, не cwd.
const ROOT = path.resolve(import.meta.dirname, '..', '..');
const CORPUS = path.join(ROOT, 'test-fixtures', 'strict-viewer', 'corpus.json');

/** Словарь режимов из README корпуса: статический список, поэтому Record. */
export const MODES: Record<string, true> = {
  'print-layout': true,
  pdf: true,
  normal: true,
  'print-preview': true,
};

/** Формат записи, которую принимает этот скрипт. */
export interface FixtureInput {
  /** Путь относительно `test-fixtures/`. */
  path: string;
  title: string;
  tags: string[];
  matrixRows: string[];
  expected: string;
  modes: string[];
  notes?: string;
}

/** Запись реестра. */
export interface CorpusFixture {
  id: string;
  format: 'docx' | 'xlsx';
  source: { path: string };
  title: string;
  tags: string[];
  matrixRows: string[];
  expected: string;
  modes: string[];
  notes?: string;
}

/** Slug по правилу README: без расширения, нижний регистр, не-буквы → `-`. */
export function slug(sourcePath: string): string {
  return sourcePath
    .replace(/\.[^.]+$/, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

/**
 * Проверить одну запись против реестра и диска.
 *
 * Путь к фикстуре разрешается от `root` (корень репозитория), поэтому тесты
 * подставляют временный каталог вместо настоящего `test-fixtures/`.
 *
 * @returns список проблем; пустой список — запись годна.
 */
export async function problems(
  input: FixtureInput,
  existing: ReadonlySet<string>,
  seen: ReadonlySet<string>,
  root: string = ROOT,
): Promise<string[]> {
  const issues: string[] = [];
  const extension = path.extname(input.path).slice(1);
  if (extension !== 'docx' && extension !== 'xlsx') {
    issues.push(`${input.path}: расширение .${extension}, ожидается .docx или .xlsx`);
  }
  if (!/^[a-z0-9\-_]+\//.test(input.path)) {
    issues.push(`${input.path}: нет префикса формата (ожидается docx/… или xlsx/…)`);
  }
  if (existing.has(input.path)) issues.push(`${input.path}: путь уже в реестре`);
  if (seen.has(input.path)) issues.push(`${input.path}: путь повторён в спецификации`);
  for (const mode of input.modes) {
    if (MODES[mode] !== true) issues.push(`${input.path}: неизвестный режим "${mode}"`);
  }
  if (input.matrixRows.length === 0) issues.push(`${input.path}: пустой matrixRows`);
  if (input.tags.length === 0) issues.push(`${input.path}: пустой tags`);
  try {
    await stat(path.join(root, 'test-fixtures', input.path));
  } catch {
    issues.push(`${input.path}: файла нет на диске`);
  }
  return issues;
}

/** Точка входа. */
async function main(argv: readonly string[]): Promise<number> {
  const listIndex = argv.indexOf('--list');
  const specPath = listIndex === -1 ? undefined : argv[listIndex + 1];
  const dryRun = argv.includes('--dry-run');
  if (specPath === undefined) {
    process.stderr.write('usage: add.ts --list <specs.ts> [--dry-run]\n');
    return 2;
  }

  const corpus = JSON.parse(await readFile(CORPUS, 'utf8')) as {
    fixtures: CorpusFixture[];
    gaps?: unknown[];
  };
  const existing = new Set(corpus.fixtures.map((fixture) => fixture.source.path));
  const existingIds = new Set(corpus.fixtures.map((fixture) => fixture.id));

  const module = (await import(pathToFileURL(path.resolve(specPath)).href)) as {
    default: FixtureInput[];
  };
  const inputs = module.default;

  const seen = new Set<string>();
  const issues: string[] = [];
  const accepted: CorpusFixture[] = [];
  for (const input of inputs) {
    const found = await problems(input, existing, seen);
    issues.push(...found);
    if (found.length > 0) continue;
    seen.add(input.path);
    const id = slug(input.path);
    if (existingIds.has(id)) {
      issues.push(`${input.path}: id "${id}" уже занят другой записью`);
      continue;
    }
    existingIds.add(id);
    accepted.push({
      id,
      format: input.path.startsWith('docx/') ? 'docx' : 'xlsx',
      source: { path: input.path },
      title: input.title,
      tags: input.tags,
      matrixRows: input.matrixRows,
      expected: input.expected,
      modes: input.modes,
      ...(input.notes === undefined ? {} : { notes: input.notes }),
    });
  }

  if (issues.length > 0) {
    process.stderr.write(`${issues.join('\n')}\n\nvisual-oracle: ${issues.length} problem(s)\n`);
    return 1;
  }

  const sorted = [...corpus.fixtures, ...accepted].sort((a, b) => {
    if (a.format !== b.format) return a.format === 'docx' ? -1 : 1;
    return a.id.localeCompare(b.id);
  });
  if (!dryRun) {
    await writeFile(CORPUS, `${JSON.stringify({ ...corpus, fixtures: sorted }, null, 2)}\n`, 'utf8');
  }
  process.stdout.write(
    `${dryRun ? 'would add' : 'added'} ${accepted.length} fixture(s); total ${sorted.length}\n`,
  );
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main(process.argv.slice(2));
}
