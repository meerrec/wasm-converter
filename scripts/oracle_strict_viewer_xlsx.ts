// Досыпка XLSX-фикстур strict-viewer в `test-fixtures/xlsx/oracle.json`.
//
// Зачем: `oracle.json` строит `scripts/gen-fixtures.ts` только по своему
// массиву `FIXTURES`, а книги, которые он не создаёт (генератор
// `scripts/generate_strict_viewer_xlsx.ts`), в эталон не попадают. Тест
// `every_fixture_parses_like_the_oracle` требует точного совпадения набора, и
// молча разойтись они не могут.
//
// Запуск:
//   npx tsx scripts/oracle_strict_viewer_xlsx.ts
//
// Скрипт идемпотентен: повторный запуск даёт тот же `oracle.json`.

import { readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

import ExcelJS from 'exceljs';

const ROOT = path.resolve(import.meta.dirname, '..');
const XLSX_DIR = path.join(ROOT, 'test-fixtures', 'xlsx');
const ORACLE = path.join(XLSX_DIR, 'oracle.json');

/** Клетка эталона: `[row, col, kind, value?]` — формат `describe` из gen-fixtures. */
type CellRecord = [number, number, string] | [number, number, string, unknown];

/** Лист эталона. */
interface SheetOracle {
  name: string;
  state: string;
  cells: CellRecord[];
}

/** Описание значения ячейки — та же форма, что в `gen-fixtures.ts`. */
function describe(cell: ExcelJS.Cell, row: number, col: number): CellRecord {
  const at = [row - 1, col - 1] as const;
  const value: unknown = cell.value;
  if (value === null || value === undefined) return [...at, '-'];
  if (typeof value === 'number') return [...at, 'n', value];
  if (typeof value === 'string') return [...at, 's', value];
  if (typeof value === 'boolean') return [...at, 'b', value];
  if (value instanceof Date) return [...at, 'd', value.toISOString()];
  if (typeof value === 'object' && 'richText' in value && Array.isArray(value.richText)) {
    const text = value.richText
      .filter((part): part is { text: string } => typeof part === 'object' && part !== null && 'text' in part && typeof part.text === 'string')
      .map((part) => part.text)
      .join('');
    return [...at, 's', text];
  }
  if (typeof value === 'object' && 'formula' in value) {
    const result = 'result' in value ? value.result : undefined;
    if (typeof result === 'number') return [...at, 'n', result];
    if (typeof result === 'string') return [...at, 's', result];
    return [...at, 'f'];
  }
  if (typeof value === 'object' && 'error' in value && typeof value.error === 'string') {
    return [...at, 'e', value.error];
  }
  return [...at, '?'];
}

/** Эталон одной книги. */
async function oracleFor(bytes: Buffer): Promise<{ sheets: SheetOracle[] }> {
  const wb = new ExcelJS.Workbook();
  await wb.xlsx.load(bytes);
  const sheets: SheetOracle[] = [];
  wb.eachSheet((ws) => {
    const cells: CellRecord[] = [];
    ws.eachRow({ includeEmpty: false }, (row, rowNumber) => {
      row.eachCell({ includeEmpty: false }, (cell, colNumber) => {
        if (cell.value === null || cell.value === undefined) return;
        if (cell.isMerged && cell.master.address !== cell.address) return;
        cells.push(describe(cell, rowNumber, colNumber));
      });
    });
    sheets.push({ name: ws.name, state: ws.state ?? 'visible', cells });
  });
  return { sheets };
}

/** Книги этого набора: ровно те, что создаёт `generate_strict_viewer_xlsx.ts`. */
const OWN_BOOKS = new Set([
  'print-area-titles.xlsx',
  'print-fit-to-page.xlsx',
  'print-headers-footers.xlsx',
  'formats-date-1904.xlsx',
  'layout-shrink-to-fit.xlsx',
  'external-workbook-link.xlsx',
  'pivot-basic.xlsx',
  'slicer-basic.xlsx',
]);

/** Точка входа. */
async function main(): Promise<void> {
  const raw = JSON.parse(await readFile(ORACLE, 'utf8')) as {
    version: number;
    files: Record<string, unknown>;
  };
  const present = await readdir(XLSX_DIR);
  const books = present.filter((name) => name.endsWith('.xlsx') && OWN_BOOKS.has(name)).sort();

  for (const name of books) {
    const bytes = await readFile(path.join(XLSX_DIR, name));
    raw.files[name] = await oracleFor(bytes);
  }

  // Каталог и эталон обязаны совпадать: иначе тест падает с непонятной
  // разницей, а виноват будет именно этот шаг.
  const allBooks = present.filter((name) => name.endsWith('.xlsx')).sort();
  const missing = allBooks.filter((name) => raw.files[name] === undefined);
  if (missing.length > 0) {
    process.stderr.write(
      `в эталоне нет книг: ${missing.join(', ')}\n` +
        'это не книги strict-viewer — добавьте их в FIXTURES scripts/gen-fixtures.ts\n',
    );
    process.exitCode = 1;
    return;
  }

  await writeFile(ORACLE, `${JSON.stringify(raw)}\n`, 'utf8');
  process.stdout.write(`oracle: ${Object.keys(raw.files).length} файл(ов), досыпано ${books.length}\n`);
}

await main();
