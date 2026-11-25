// Corpus index строгого просмотра: список фикстур, их режимов и пробелов
// матрицы. Файл читается один раз на прогон и служит единственным источником
// fixture id — slug от `source.path` детерминирован, поэтому id не может
// разъехаться между corpus index и каталогом эталонов.

import { readFile } from 'node:fs/promises';

import { isRecord } from './json.js';
import type { DocFormat, ViewMode } from './types.js';

/** Версия схемы corpus index, которую понимает этот загрузчик. */
export const CORPUS_SCHEMA_VERSION = '1.0.0';

/** Допустимые режимы по формату: матрица не знает других комбинаций. */
export const MODES_BY_FORMAT: Record<DocFormat, readonly ViewMode[]> = {
  docx: ['print-layout', 'pdf'],
  xlsx: ['normal', 'print-preview'],
};

/** Одно нарушение corpus index. */
export interface CorpusErrorEntry {
  /** Путь до поля в документе, например `fixtures[2].id`. */
  path: string;
  /** Английское сообщение: его читает оператор и CI. */
  message: string;
}

/** Хранилище артефактов корпуса. */
export interface CorpusStorage {
  kind: 'git-lfs' | 'release-asset' | 'object-store';
  rootUri: string;
  maxCorpusBytes?: number;
}

/** Паспорт корпуса: версии и ссылка на принятый baseline. */
export interface CorpusMeta {
  id: string;
  version: string;
  matrixVersion: string;
  baselineId: string | null;
  storage: CorpusStorage;
}

/** Одна фикстура корпуса. */
export interface CorpusFixture {
  id: string;
  format: DocFormat;
  source: { path: string };
  title: string;
  tags: string[];
  matrixRows: string[];
  expected: string;
  modes: ViewMode[];
  notes?: string;
}

/** Пробел матрицы: строка, для которой нет фикстуры. */
export interface CorpusGap {
  matrixRow: string;
  missing: string;
  reason: string;
}

/** Разобранный corpus index. */
export interface Corpus {
  schemaVersion: '1.0.0';
  corpus: CorpusMeta;
  fixtures: CorpusFixture[];
  gaps: CorpusGap[];
}

/** Результат проверки corpus index без исключений. */
export type CorpusValidation =
  | { ok: true; corpus: Corpus }
  | { ok: false; errors: CorpusErrorEntry[] };

/** Ошибка загрузки: несёт все нарушения сразу, а не первое из них. */
export class CorpusError extends Error {
  readonly errors: CorpusErrorEntry[];

  constructor(errors: CorpusErrorEntry[]) {
    super(`corpus index is invalid: ${errors.length} error(s)`);
    this.name = 'CorpusError';
    this.errors = errors;
  }
}

/**
 * Детерминированный fixture id из пути источника.
 *
 * Снимается расширение, строка приводится к нижнему регистру, все
 * не-алфанумерические символы схлопываются в `-`, края обрезаются:
 * `docx/tables/long_table.docx` → `docx-tables-long-table`.
 */
export function slug(sourcePath: string): string {
  return sourcePath
    .replace(/\.[^./\\]+$/, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

/**
 * Прочитать и проверить corpus index.
 *
 * @throws {CorpusError} если файл не читается, не парсится или нарушает схему.
 */
export async function loadCorpus(corpusPath: string): Promise<Corpus> {
  let text: string;
  try {
    text = await readFile(corpusPath, 'utf8');
  } catch (error) {
    throw new CorpusError([
      { path: '', message: `cannot read corpus index: ${errorMessage(error)}` },
    ]);
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new CorpusError([
      { path: '', message: `corpus index is not valid JSON: ${errorMessage(error)}` },
    ]);
  }
  const result = validateCorpus(value);
  if (!result.ok) throw new CorpusError(result.errors);
  return result.corpus;
}

/** Проверить уже разобранный JSON corpus index. */
export function validateCorpus(value: unknown): CorpusValidation {
  const errors: CorpusErrorEntry[] = [];
  if (!isRecord(value)) {
    return { ok: false, errors: [{ path: '', message: 'corpus index must be a JSON object' }] };
  }
  if (value['schemaVersion'] !== CORPUS_SCHEMA_VERSION) {
    errors.push({
      path: 'schemaVersion',
      message: `expected "${CORPUS_SCHEMA_VERSION}", got ${JSON.stringify(value['schemaVersion'])}`,
    });
  }
  checkCorpusMeta(value['corpus'], errors);
  checkFixtures(value['fixtures'], errors);
  checkGaps(value['gaps'], errors);
  if (errors.length > 0) return { ok: false, errors };
  return { ok: true, corpus: value as unknown as Corpus };
}

/**
 * Отфильтровать фикстуры по выражениям `tag=<tag>` и `id=<glob>`.
 *
 * Фильтры объединяются по «И»: фикстура проходит, только если выполнено
 * каждое выражение. Пустой список фильтров возвращает все фикстуры.
 */
export function filterFixtures(
  fixtures: readonly CorpusFixture[],
  filters: readonly string[],
): CorpusFixture[] {
  const predicates = filters.map(compileFilter);
  return fixtures.filter((fixture) => predicates.every((predicate) => predicate(fixture)));
}

/** Найти фикстуру по id. */
export function fixtureById(corpus: Corpus, id: string): CorpusFixture | undefined {
  return corpus.fixtures.find((fixture) => fixture.id === id);
}

/** Скомпилировать одно выражение фильтра в предикат. */
function compileFilter(filter: string): (fixture: CorpusFixture) => boolean {
  const separator = filter.indexOf('=');
  if (separator <= 0) {
    throw new Error(`invalid filter "${filter}": expected tag=<tag> or id=<glob>`);
  }
  const kind = filter.slice(0, separator);
  const value = filter.slice(separator + 1);
  if (value.length === 0) {
    throw new Error(`invalid filter "${filter}": empty value`);
  }
  if (kind === 'tag') {
    return (fixture) => fixture.tags.includes(value);
  }
  if (kind === 'id') {
    const pattern = globToRegExp(value);
    return (fixture) => pattern.test(fixture.id);
  }
  throw new Error(`invalid filter "${filter}": unknown kind "${kind}"`);
}

/** Glob с `*` и `?`; остальные символы экранируются. */
function globToRegExp(glob: string): RegExp {
  let source = '^';
  for (const char of glob) {
    if (char === '*') source += '.*';
    else if (char === '?') source += '.';
    else source += char.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }
  return new RegExp(`${source}$`);
}

function checkCorpusMeta(value: unknown, errors: CorpusErrorEntry[]): void {
  if (!isRecord(value)) {
    errors.push({ path: 'corpus', message: 'corpus must be an object' });
    return;
  }
  checkPattern(value['id'], /^[a-z0-9][a-z0-9.-]*$/, 'corpus.id', errors);
  checkNonEmptyString(value['version'], 'corpus.version', errors);
  checkNonEmptyString(value['matrixVersion'], 'corpus.matrixVersion', errors);
  const baselineId = value['baselineId'];
  if (baselineId !== null && typeof baselineId !== 'string') {
    errors.push({ path: 'corpus.baselineId', message: 'baselineId must be a string or null' });
  } else if (typeof baselineId === 'string' && baselineId.length === 0) {
    errors.push({ path: 'corpus.baselineId', message: 'baselineId must not be empty' });
  }
  const storage = value['storage'];
  if (!isRecord(storage)) {
    errors.push({ path: 'corpus.storage', message: 'storage must be an object' });
    return;
  }
  const kind = storage['kind'];
  if (kind !== 'git-lfs' && kind !== 'release-asset' && kind !== 'object-store') {
    errors.push({
      path: 'corpus.storage.kind',
      message: 'kind must be one of git-lfs, release-asset, object-store',
    });
  }
  checkNonEmptyString(storage['rootUri'], 'corpus.storage.rootUri', errors);
  if (kind === 'git-lfs' && typeof storage['rootUri'] === 'string') {
    if (!isPortableRelativePath(storage['rootUri'])) {
      errors.push({
        path: 'corpus.storage.rootUri',
        message: 'git-lfs rootUri must be a portable relative path',
      });
    }
  }
  const maxBytes = storage['maxCorpusBytes'];
  if (maxBytes !== undefined && !(typeof maxBytes === 'number' && Number.isInteger(maxBytes) && maxBytes > 0)) {
    errors.push({
      path: 'corpus.storage.maxCorpusBytes',
      message: 'maxCorpusBytes must be a positive integer',
    });
  }
}

function checkFixtures(value: unknown, errors: CorpusErrorEntry[]): void {
  if (!Array.isArray(value)) {
    errors.push({ path: 'fixtures', message: 'fixtures must be an array' });
    return;
  }
  const seen = new Set<string>();
  value.forEach((entry, index) => {
    const base = `fixtures[${index}]`;
    if (!isRecord(entry)) {
      errors.push({ path: base, message: 'fixture must be an object' });
      return;
    }
    const id = entry['id'];
    if (typeof id !== 'string' || id.length === 0) {
      errors.push({ path: `${base}.id`, message: 'id must be a non-empty string' });
    } else if (seen.has(id)) {
      errors.push({ path: `${base}.id`, message: `duplicate fixture id "${id}"` });
    } else {
      seen.add(id);
    }

    const format = entry['format'];
    if (format !== 'docx' && format !== 'xlsx') {
      errors.push({ path: `${base}.format`, message: 'format must be docx or xlsx' });
    }

    const source = entry['source'];
    let sourcePath: string | null = null;
    if (!isRecord(source)) {
      errors.push({ path: `${base}.source`, message: 'source must be an object' });
    } else if (typeof source['path'] !== 'string' || source['path'].length === 0) {
      errors.push({ path: `${base}.source.path`, message: 'path must be a non-empty string' });
    } else {
      sourcePath = source['path'];
    }

    if (sourcePath !== null) {
      if (!isPortableRelativePath(sourcePath)) {
        errors.push({
          path: `${base}.source.path`,
          message: 'path must be a portable relative path with "/" separators',
        });
      }
      if (typeof id === 'string' && id.length > 0 && id !== slug(sourcePath)) {
        errors.push({
          path: `${base}.id`,
          message: `id "${id}" does not match slug("${sourcePath}") = "${slug(sourcePath)}"`,
        });
      }
      if (format === 'docx' || format === 'xlsx') {
        if (!sourcePath.startsWith(`${format}/`)) {
          errors.push({
            path: `${base}.source.path`,
            message: `path must start with "${format}/" for format ${format}`,
          });
        }
      }
    }

    checkNonEmptyString(entry['title'], `${base}.title`, errors);
    checkNonEmptyString(entry['expected'], `${base}.expected`, errors);
    checkStringArray(entry['tags'], `${base}.tags`, errors);
    checkStringArray(entry['matrixRows'], `${base}.matrixRows`, errors);

    const modes = entry['modes'];
    if (!Array.isArray(modes) || modes.length === 0) {
      errors.push({ path: `${base}.modes`, message: 'modes must be a non-empty array' });
    } else if (format === 'docx' || format === 'xlsx') {
      const legal = MODES_BY_FORMAT[format];
      const seenModes = new Set<unknown>();
      modes.forEach((mode, modeIndex) => {
        if (!legal.includes(mode as ViewMode)) {
          errors.push({
            path: `${base}.modes[${modeIndex}]`,
            message: `mode ${JSON.stringify(mode)} is not legal for ${format}; expected one of ${legal.join(', ')}`,
          });
        }
        // Пара (source.path, view.mode) обязана быть уникальной
        // (baseline-policy §7): дубль режима дал бы два manifest на один
        // и тот же baseline.
        if (seenModes.has(mode)) {
          errors.push({
            path: `${base}.modes[${modeIndex}]`,
            message: `duplicate mode ${JSON.stringify(mode)}`,
          });
        }
        seenModes.add(mode);
      });
    }

    const notes = entry['notes'];
    if (notes !== undefined && typeof notes !== 'string') {
      errors.push({ path: `${base}.notes`, message: 'notes must be a string' });
    }
  });
}

function checkGaps(value: unknown, errors: CorpusErrorEntry[]): void {
  if (!Array.isArray(value)) {
    errors.push({ path: 'gaps', message: 'gaps must be an array' });
    return;
  }
  value.forEach((entry, index) => {
    const base = `gaps[${index}]`;
    if (!isRecord(entry)) {
      errors.push({ path: base, message: 'gap must be an object' });
      return;
    }
    checkNonEmptyString(entry['matrixRow'], `${base}.matrixRow`, errors);
    checkNonEmptyString(entry['missing'], `${base}.missing`, errors);
    checkNonEmptyString(entry['reason'], `${base}.reason`, errors);
  });
}

function checkNonEmptyString(value: unknown, path: string, errors: CorpusErrorEntry[]): void {
  if (typeof value !== 'string' || value.length === 0) {
    errors.push({ path, message: 'must be a non-empty string' });
  }
}

function checkStringArray(value: unknown, path: string, errors: CorpusErrorEntry[]): void {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push({ path, message: 'must be a non-empty array of strings' });
    return;
  }
  value.forEach((item, index) => {
    if (typeof item !== 'string' || item.length === 0) {
      errors.push({ path: `${path}[${index}]`, message: 'must be a non-empty string' });
    }
  });
}

function checkPattern(
  value: unknown,
  pattern: RegExp,
  path: string,
  errors: CorpusErrorEntry[],
): void {
  if (typeof value !== 'string' || !pattern.test(value)) {
    errors.push({ path, message: `must match ${pattern.source}` });
  }
}

/**
 * Переносимый относительный путь: `/` как разделитель, без `\`, `:`,
 * абсолютного начала и сегментов `.`/`..`.
 */
export function isPortableRelativePath(value: string): boolean {
  if (value.length === 0) return false;
  if (value.startsWith('/')) return false;
  if (value.includes('\\') || value.includes(':')) return false;
  return value.split('/').every((segment) => segment !== '' && segment !== '.' && segment !== '..');
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
