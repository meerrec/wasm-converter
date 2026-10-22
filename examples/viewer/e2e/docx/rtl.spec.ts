// RTL в настоящем браузере: направление runs, логический порядок и bidiVisual.

import { expect, test } from '@playwright/test';

import { boot } from '../helpers/boot.js';
import { openFixture } from '../helpers/docx_frame.js';

test.describe('DOCX RTL', () => {
  test('арабский и иврит идут справа налево', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    for (const fixture of ['rtl/arabic_basic.docx', 'rtl/hebrew_basic.docx']) {
      await openFixture(page, fixture);
      const layer = await page.evaluate(async () => {
        const bridge = window.docConverter;
        if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
        return bridge.viewer.getTextLayer(0);
      });
      const rtl = layer.runs.filter((run) => run.direction === 'rtl');
      expect(rtl.length, fixture).toBeGreaterThan(0);
      expect(rtl.every((run) => run.bidiLevel % 2 === 1), fixture).toBe(true);
      // DOM-порядок логический: readingOrder монотонен.
      const orders = layer.runs.map((run) => run.readingOrder);
      expect([...orders].sort((a, b) => a - b)).toEqual(orders);
    }
  });

  test('в смешанном абзаце есть и LTR-, и RTL-уровни', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'rtl/mixed_direction.docx');
    const layer = await page.evaluate(async () => {
      const bridge = window.docConverter;
      if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
      return bridge.viewer.getTextLayer(0);
    });
    expect(layer.runs.some((run) => run.bidiLevel % 2 === 1)).toBe(true);
    expect(layer.runs.some((run) => run.bidiLevel % 2 === 0)).toBe(true);
    await expect(page.locator('.dc-text-line[dir="rtl"]').first()).toBeAttached();
  });

  test('w:bidiVisual зеркалит колонки, оставляя логический col', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'rtl/rtl_table.docx');
    const layer = await page.evaluate(async () => {
      const bridge = window.docConverter;
      if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
      return bridge.viewer.getTextLayer(0);
    });
    const first = layer.runs.find((run) => run.table?.row === 0 && run.table.col === 0);
    const second = layer.runs.find((run) => run.table?.row === 0 && run.table.col === 1);
    expect(first).toBeDefined();
    expect(second).toBeDefined();
    expect(first?.bbox.x).toBeGreaterThan(second?.bbox.x ?? Number.POSITIVE_INFINITY);
  });
});
