// Растеризация PDF в PNG через `pdftoppm`.
//
// PDF-режимы (`docx.pdf`, `xlsx.print-preview`) снимают не холст, а экспорт
// движка: страницы PDF — это то, что сертифицируется. Растеризатор обязан быть
// тем же, что записан в manifest (`environment.rasterizer`: `pdftoppm`, 144 dpi),
// иначе сравнение шло бы с другим сэмплингом.

import { execFile } from 'node:child_process';
import { mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { deferred } from './deferred.js';
import { CaptureError } from './errors.js';

/** Разрешение растеризации: зафиксировано матрицей сертификации. */
export const RASTER_DPI = 144;

/** Аргументы `pdftoppm` для одного PDF. Чистая функция — её и проверяет тест. */
export function pdftoppmArgs(pdfPath: string, outPrefix: string, dpi: number = RASTER_DPI): string[] {
  return ['-png', '-r', String(dpi), pdfPath, outPrefix];
}

/** Запуск внешней команды с захватом вывода. */
function run(command: string, args: readonly string[]): Promise<{ stdout: string; stderr: string }> {
  const { promise, resolve, reject } = deferred<{ stdout: string; stderr: string }>();
  execFile(command, [...args], { maxBuffer: 16 * 1024 * 1024 }, (error, stdout, stderr) => {
    if (error) {
      reject(Object.assign(error, { stdout, stderr }));
      return;
    }
    resolve({ stdout, stderr });
  });
  return promise;
}

/** Есть ли `pdftoppm` в PATH. Проверяется один раз за сессию. */
let availability: Promise<void> | null = null;

/**
 * Проверить наличие `pdftoppm`.
 *
 * @throws {CaptureError} код `missing-tool` — без растеризатора PDF-режимы
 * молча выдали бы пустоту, поэтому отсутствие инструмента — отказ, а не скип.
 */
export function ensurePdftoppm(mode: string): Promise<void> {
  availability ??= (async () => {
    try {
      await run('pdftoppm', ['-v']);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw new CaptureError(
        'missing-tool',
        `pdftoppm is required for view mode "${mode}" but is not available: ${detail}`,
      );
    }
  })();
  return availability;
}

/** Одна растеризованная страница: путь к PNG и его байты. */
export interface RasterPage {
  /** Номер страницы PDF, начиная с 1. */
  pageNumber: number;
  bytes: Uint8Array;
}

/** Порядковый номер страницы из имени `pdftoppm` (`prefix-12.png`). */
function pageNumberFromName(fileName: string, prefix: string): number | null {
  if (!fileName.startsWith(`${prefix}-`) || !fileName.endsWith('.png')) return null;
  const digits = fileName.slice(prefix.length + 1, -'.png'.length);
  const value = Number(digits);
  return Number.isInteger(value) && value > 0 ? value : null;
}

/**
 * Растеризовать PDF в PNG-страницы.
 *
 * `pdftoppm` сам выбирает ширину нумерации (`page-1.png` до десяти страниц,
 * `page-01.png` после), поэтому имена не угадываются: каталог читается, а
 * страницы сортируются по номеру из имени.
 *
 * @throws {CaptureError} `rasterize-failed` — команда упала или не дала страниц.
 */
export async function rasterizePdf(pdf: Uint8Array, mode: string): Promise<RasterPage[]> {
  await ensurePdftoppm(mode);
  const dir = await mkdtemp(path.join(tmpdir(), 'visual-oracle-pdf-'));
  try {
    const pdfPath = path.join(dir, 'document.pdf');
    await writeFile(pdfPath, pdf);
    const prefix = 'page';
    const outPrefix = path.join(dir, prefix);
    try {
      await run('pdftoppm', pdftoppmArgs(pdfPath, outPrefix));
    } catch (error) {
      // `execFile` кладёт stderr в объект ошибки; поле читается через `in`,
      // чтобы не выдумывать форму чужого объекта.
      const stderr =
        typeof error === 'object' && error !== null && 'stderr' in error
          ? String(error.stderr)
          : '';
      const detail = error instanceof Error ? error.message : String(error);
      throw new CaptureError(
        'rasterize-failed',
        `pdftoppm failed for view mode "${mode}": ${detail}${stderr ? `\n${stderr}` : ''}`,
        { cause: error },
      );
    }

    const names = (await readdir(dir)).filter((name) => name.endsWith('.png'));
    const pages: RasterPage[] = [];
    for (const name of names) {
      const pageNumber = pageNumberFromName(name, prefix);
      if (pageNumber === null) continue;
      pages.push({ pageNumber, bytes: new Uint8Array(await readFile(path.join(dir, name))) });
    }
    pages.sort((a, b) => a.pageNumber - b.pageNumber);
    if (pages.length === 0) {
      throw new CaptureError(
        'rasterize-failed',
        `pdftoppm produced no pages for view mode "${mode}"`,
      );
    }
    return pages;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
