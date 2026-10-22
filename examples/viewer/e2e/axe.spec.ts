// axe-core на доступных представлениях: DOCX-слой и минимальная обвязка XLSX.
//
// Гейт жёсткий: любое нарушение — провал с перечнем id/таргетов, чтобы не
// разбираться по одному скриншоту. Набор курируемый: по одной-две фикстуры
// каждой категории (заголовки, списки, таблицы, RTL, поля, сноски, картинки).
// Полный корпус — 109 DOCX — гоняется отдельным ночным прогоном: пакетный axe
// на нём занимает десятки минут и не должен блокировать каждый PR.

import { expect, test } from '@playwright/test';

import { runAxe } from './helpers/axe.js';
import { boot } from './helpers/boot.js';

const DOCX = [
  'a11y/a11y_audit.docx',
  'basic/paragraph_alignment.docx',
  'basic/multiple_runs.docx',
  'formatting/heading_1.docx',
  'formatting/heading_3.docx',
  'formatting/bold.docx',
  'numbering/decimal_basic.docx',
  'numbering/bullet_symbols.docx',
  'numbering/multilevel.docx',
  'tables/header_repeat.docx',
  'tables/grid_span.docx',
  'tables/nested.docx',
  'rtl/arabic_basic.docx',
  'rtl/hebrew_basic.docx',
  'rtl/mixed_direction.docx',
  'rtl/rtl_table.docx',
  'fields/hyperlink_field.docx',
  'notes/footnote_basic.docx',
];

const XLSX = [
  'content-table.xlsx',
  'content-markup.xlsx',
  'content-mixed-types.xlsx',
  'charts-five-kinds.xlsx',
  'comments-legacy.xlsx',
  'edge-merged-large.xlsx',
  'formats-date-iso.xlsx',
  'images-png.xlsx',
  'sheets-three.xlsx',
  'styles-bold-italic.xlsx',
  'values-integers.xlsx',
];

test.describe('axe', () => {
  test.setTimeout(300_000);

  test('DOCX: 0 violations на репрезентативном наборе', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    for (const fixture of DOCX) {
      await page.selectOption('#fixture', fixture);
      await page.waitForFunction(() => {
        const bridge = window.docConverter;
        return bridge?.kind === 'docx' && bridge.viewer.pageCount > 0;
      });
      // Семантическое дерево строится после открытия асинхронно.
      await page.waitForTimeout(150);
      const results = await runAxe(page, '#viewer');
      expect(
        results.violations,
        `${fixture}: ${results.violations.map((v) => v.id).join(', ')}`,
      ).toEqual([]);
    }
  });

  test('XLSX: 0 violations на репрезентативном наборе', async ({ page }) => {
    await boot(page, '/xlsx', { a11y: true });
    for (const fixture of XLSX) {
      await page.selectOption('#fixture', fixture);
      await expect(page.locator('#status')).toContainText('лист', { timeout: 30_000 });
      const results = await runAxe(page, '#viewer');
      expect(
        results.violations,
        `${fixture}: ${results.violations.map((v) => v.id).join(', ')}`,
      ).toEqual([]);
    }
  });
});
