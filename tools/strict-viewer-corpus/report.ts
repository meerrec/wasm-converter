// Сверка приоритетного состава strict-viewer корпуса с матрицей и реестром.
//
// Зачем отдельный скрипт, а не абзац в README: числа в README устаревают
// молча, и расхождение «в документе 20, в файлах 36» обнаружилось только
// вручную. Скрипт отвечает на три вопроса одной командой:
//   1. какие файлы матрица называет минимальными (`Минимальная фикстура`);
//   2. какие из них есть в реестре `corpus.json` и на диске;
//   3. сколько фикстур каждого формата и нет ли дублей.
//
// Запуск:
//   npx tsx tools/strict-viewer-corpus/report.ts
//
// Коды выхода: 0 — расхождений нет; 1 — есть отсутствующие или дубли;
// 2 — не найден корпус или матрица.

import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/**
 * Корень репозитория: два уровня вверх от файла, независимо от cwd.
 *
 * От cwd он не зависит намеренно: скрипт запускают и из корня, и из
 * `tools/strict-viewer-corpus` (`pnpm -F … report`), и оба запуска обязаны
 * видеть одну и ту же матрицу, а не падать с `ENOENT` на чужом каталоге.
 */
const ROOT = path.resolve(import.meta.dirname, '..', '..');

/** Минимальная фикстура одной строки матрицы. */
export interface MatrixRow {
  /** ID строки, например `docx.tables-pagination`. */
  rowId: string;
  /** Путь от `test-fixtures/` либо `null`, если матрица пишет «требуется новая». */
  fixture: string | null;
}

/** Запись реестра корпуса. */
export interface CorpusFixture {
  id: string;
  format: 'docx' | 'xlsx';
  source: { path: string };
  modes: string[];
}

/** Итог сверки: числа, расхождения и код выхода. */
export interface CorpusReport {
  matrixRows: number;
  rowsWithFixture: number;
  uniqueMinimalFiles: number;
  /** Уникальные минимальные фикстуры, которых нет в реестре. */
  missingFromCorpus: string[];
  /** Из `missingFromCorpus` те, которых нет и на диске. */
  missingOnDisk: string[];
  duplicateIds: string[];
  duplicatePaths: string[];
  docx: number;
  xlsx: number;
  total: number;
  /** 0 — расхождений нет; 1 — есть отсутствующие или дубли. */
  exitCode: number;
}

/** Разбить строку таблицы матрицы на ячейки без внешних пустых краёв. */
function cells(line: string): string[] {
  return line
    .split('|')
    .slice(1, -1)
    .map((cell) => cell.trim());
}

/**
 * Прочитать строки матрицы DOCX/XLSX.
 *
 * Разбор построчный и намеренно простой: таблицы в матрице плоские, а пятая
 * колонка (`Минимальная фикстура`) начинается с обратных кавычек вокруг пути.
 * Полноценный markdown-парсер здесь был бы дороже, чем польза.
 */
export function parseMatrix(text: string): MatrixRow[] {
  const rows: MatrixRow[] = [];
  for (const line of text.split('\n')) {
    if (!line.startsWith('| `docx.') && !line.startsWith('| `xlsx.')) continue;
    const parts = cells(line);
    const rowId = (parts[0] ?? '').replace(/`/g, '');
    const match = /^`([^`]+\.(?:docx|xlsx))`/.exec(parts[4] ?? '');
    rows.push({ rowId, fixture: match?.[1] ?? null });
  }
  return rows;
}

/**
 * Сверить строки матрицы с реестром и диском.
 *
 * `existsOnDisk` получает путь от `test-fixtures/`, поэтому проверка файлов
 * подменяется в тестах без чтения настоящего репозитория.
 */
export function summarize(
  rows: readonly MatrixRow[],
  fixtures: readonly CorpusFixture[],
  existsOnDisk: (relativePath: string) => boolean,
): CorpusReport {
  const inCorpus = new Set(fixtures.map((fixture) => fixture.source.path));
  const minimal = [...new Set(rows.map((row) => row.fixture).filter((value): value is string => value !== null))];

  const missingFromCorpus = minimal.filter((path_) => !inCorpus.has(path_));
  const missingOnDisk = missingFromCorpus.filter((path_) => !existsOnDisk(path_));

  const ids = new Map<string, number>();
  const paths = new Map<string, number>();
  for (const fixture of fixtures) {
    ids.set(fixture.id, (ids.get(fixture.id) ?? 0) + 1);
    paths.set(fixture.source.path, (paths.get(fixture.source.path) ?? 0) + 1);
  }
  const duplicateIds = [...ids].filter(([, count]) => count > 1).map(([id]) => id);
  const duplicatePaths = [...paths].filter(([, count]) => count > 1).map(([path_]) => path_);

  const docx = fixtures.filter((fixture) => fixture.format === 'docx').length;
  const xlsx = fixtures.filter((fixture) => fixture.format === 'xlsx').length;
  const hasProblems = missingFromCorpus.length > 0 || duplicateIds.length > 0 || duplicatePaths.length > 0;

  return {
    matrixRows: rows.length,
    rowsWithFixture: rows.filter((row) => row.fixture !== null).length,
    uniqueMinimalFiles: minimal.length,
    missingFromCorpus,
    missingOnDisk,
    duplicateIds,
    duplicatePaths,
    docx,
    xlsx,
    total: fixtures.length,
    exitCode: hasProblems ? 1 : 0,
  };
}

/** Текст отчёта для человека. */
export function formatReport(report: CorpusReport): string {
  const lines = [
    `строк матрицы: ${report.matrixRows}`,
    `строк с минимальной фикстурой: ${report.rowsWithFixture}`,
    `уникальных минимальных файлов: ${report.uniqueMinimalFiles}`,
    `реестр: ${report.docx} DOCX + ${report.xlsx} XLSX = ${report.total}`,
  ];
  if (report.missingFromCorpus.length > 0) {
    lines.push(`минимальных нет в реестре: ${report.missingFromCorpus.join(', ')}`);
  }
  if (report.missingOnDisk.length > 0) {
    lines.push(`минимальных нет и на диске: ${report.missingOnDisk.join(', ')}`);
  }
  if (report.duplicateIds.length > 0) lines.push(`дубли id: ${report.duplicateIds.join(', ')}`);
  if (report.duplicatePaths.length > 0) lines.push(`дубли путей: ${report.duplicatePaths.join(', ')}`);
  return `${lines.join('\n')}\n`;
}

/** Прочитать реестр корпуса. */
async function readCorpus(corpusPath: string): Promise<CorpusFixture[]> {
  const text = await readFile(corpusPath, 'utf8');
  const parsed = JSON.parse(text) as { fixtures?: CorpusFixture[] };
  return parsed.fixtures ?? [];
}

/** Какие из минимальных фикстур, отсутствующих в реестре, есть на диске. */
async function existsOnDisk(rows: readonly MatrixRow[], fixtures: readonly CorpusFixture[]): Promise<Set<string>> {
  const inCorpus = new Set(fixtures.map((fixture) => fixture.source.path));
  const missing = new Set(rows.map((row) => row.fixture).filter((path_): path_ is string => path_ !== null && !inCorpus.has(path_)));
  const present = new Set<string>();
  for (const relativePath of missing) {
    try {
      await stat(path.join(ROOT, 'test-fixtures', relativePath));
      present.add(relativePath);
    } catch {
      // Нет на диске: в отчёте она попадает в `missingOnDisk`.
    }
  }
  return present;
}

/** Точка входа. */
async function main(): Promise<number> {
  const matrixPath = path.join(ROOT, 'docs', 'strict-viewer', 'certification-matrix.md');
  const corpusPath = path.join(ROOT, 'test-fixtures', 'strict-viewer', 'corpus.json');
  let report: CorpusReport;
  try {
    const rows = parseMatrix(await readFile(matrixPath, 'utf8'));
    const fixtures = await readCorpus(corpusPath);
    const there = await existsOnDisk(rows, fixtures);
    report = summarize(rows, fixtures, (relativePath) => there.has(relativePath));
  } catch (error) {
    process.stderr.write(`error: ${error instanceof Error ? error.message : String(error)}\n`);
    return 2;
  }
  process.stdout.write(formatReport(report));
  return report.exitCode;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main();
}
