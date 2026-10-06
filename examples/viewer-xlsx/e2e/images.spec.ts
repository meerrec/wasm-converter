// Картинки: три фикстуры с картинками открываются в браузере, рисуются на
// холсте и доезжают до PDF XObject'ами.
//
// Ожидания — из нативного `crates/pdf/tests/image.rs`: одна media-часть даёт
// один XObject, сколько бы якорей её ни рисовало, а каждый якорь — оператор
// `Do` в содержимом страницы.

import { expect, test, type Page } from '@playwright/test';
import { boot, colorShare, frameStats } from './canvas.js';
import { downloadPdf } from './download.js';
import { countOps, imageXObjects, probePdf } from './pdf-probe.js';

/** Заливка PNG-фикстуры (`IMAGE_PNG` в `scripts/gen-fixtures.ts`). */
const PNG_BLUE = [0x2f, 0x6b, 0x9a] as const;

/** Открыть фикстуру и дождаться первого кадра. */
async function openFixture(page: Page, name: string): Promise<string[]> {
  const errors = await boot(page);
  await page.selectOption('#fixture', name);
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });
  await expect(page.locator('#cmds')).not.toHaveText('0');
  return errors;
}

/** Скачать PDF текущего листа и посчитать картинки и их выводы. */
async function exportAndCount(
  page: Page,
  fixture: string,
): Promise<{ xobjects: number; draws: number }> {
  const { bytes, name } = await downloadPdf(page, '#export-pdf');
  expect(name).toBe(fixture.replace(/\.xlsx$/, '.pdf'));
  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });

  const probe = probePdf(bytes);
  const draws = probe.pages.reduce((sum, ops) => sum + countOps(ops, 'Do'), 0);
  return { xobjects: imageXObjects(probe.text), draws };
}

test('PNG: картинка видна на холсте и в PDF', async ({ page }) => {
  const errors = await openFixture(page, 'images-png.xlsx');

  // Две картинки фикстуры: якорь на ячейку B2 и якорь-диапазон E3:H8.
  const blue = await colorShare(page, PNG_BLUE);
  expect(blue, 'на холсте нет заливки картинки').toBeGreaterThan(0.005);

  const counts = await exportAndCount(page, 'images-png.xlsx');
  expect(counts).toEqual({ xobjects: 1, draws: 2 });

  expect(errors).toEqual([]);
});

test('JPEG: картинка доезжает до PDF', async ({ page }) => {
  const errors = await openFixture(page, 'images-jpeg.xlsx');

  // JPEG фикстуры — серые полосы; цветность не проверяем, но кадр не пуст.
  const stats = await frameStats(page);
  expect(stats.inkRatio).toBeGreaterThan(0.02);

  const counts = await exportAndCount(page, 'images-jpeg.xlsx');
  expect(counts).toEqual({ xobjects: 1, draws: 2 });

  expect(errors).toEqual([]);
});

test('картинки поверх данных: два XObject, три вывода', async ({ page }) => {
  const errors = await openFixture(page, 'images-over-data.xlsx');

  const stats = await frameStats(page);
  expect(stats.inkRatio).toBeGreaterThan(0.02);

  // PNG и JPEG — разные media-части; повтор PNG третьего XObject'а не заводит.
  const counts = await exportAndCount(page, 'images-over-data.xlsx');
  expect(counts).toEqual({ xobjects: 2, draws: 3 });

  expect(errors).toEqual([]);
});
