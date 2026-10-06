// Диаграммы: пять видов из `charts-five-kinds.xlsx` доезжают до холста
// цветными заливками, а до PDF — векторными примитивами внутри клипов.
//
// Это e2e-двойник нативного `crates/pdf/tests/chart.rs::chart_is_vector_not_image`:
// там поток содержимого разбирает lopdf, здесь — мини-разбор из `pdf-probe.ts`,
// но путь другой: книгу открывает воркер в браузере, PDF собирает
// wasm-модуль примера.

import { expect, test } from '@playwright/test';
import { boot, colorShare } from './canvas.js';
import { downloadPdf } from './download.js';
import { buildsPath, clips, paintsPath, probePdf } from './pdf-probe.js';

/**
 * Цвет первой серии палитры диаграмм (`PALETTE[0]` в
 * `crates/render/src/chart/layout.rs`). Текст и сетка листа серые, поэтому
 * насыщенная заливка — признак нарисованной диаграммы, а не просто непустого
 * кадра.
 */
const SERIES_BLUE = [0x4e, 0x79, 0xa7] as const;

test('рисует пять диаграмм на холсте и в PDF вектором', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'charts-five-kinds.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });
  await expect(page.locator('#cmds')).not.toHaveText('0');

  // Холст: заливки серий видны в кадре. Диаграммы занимают область от
  // столбца F, и первая попадает в исходный вид сверху.
  const blue = await colorShare(page, SERIES_BLUE);
  expect(blue, 'в кадре нет заливок палитры диаграмм').toBeGreaterThan(0.002);

  const { bytes, name } = await downloadPdf(page, '#export-pdf');
  expect(name).toBe('charts-five-kinds.pdf');
  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });

  const probe = probePdf(bytes);
  // Клип диаграммы — `q … W n … Q`; пять диаграмм — пять клипов, как и в
  // нативном тесте.
  let blocks = 0;
  for (const [page, ops] of probe.pages.entries()) {
    for (const { start, end } of clips(ops)) {
      blocks += 1;
      const block = ops.slice(start, end);
      const where = `клип диаграммы на странице ${page + 1}`;
      expect(block.filter(buildsPath).length, `${where}: путь не строится`).toBeGreaterThanOrEqual(2);
      expect(block.filter(paintsPath).length, `${where}: путь не закрашен`).toBeGreaterThanOrEqual(1);
      expect(block, `${where}: внутри XObject вместо вектора`).not.toContain('Do');
    }
  }
  expect(blocks, 'число клипов диаграмм').toBe(5);

  expect(errors).toEqual([]);
});
