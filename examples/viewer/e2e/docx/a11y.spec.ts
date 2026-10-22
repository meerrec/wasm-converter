// Доступность DOCX: DOM-слой, ARIA-дерево и порядок чтения.
//
// Проверки идут через настоящий вьюер с `a11y: true`: смысл спринта в том,
// чтобы текст документа был доступен без canvas, а не только в юнит-тестах
// плоского TextLayer.

import { expect, test } from '@playwright/test';

import { boot } from '../helpers/boot.js';
import { openFixture } from '../helpers/docx_frame.js';

test.describe('DOCX a11y', () => {
  test('строит семантическое дерево и скрывает canvas', async ({ page }) => {
    const errors = await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'a11y/a11y_audit.docx');

    await expect(page.locator('.dc-text-layer')).toHaveCount(1);
    await expect(page.locator('.dc-text-layer')).toHaveAttribute('role', 'document');
    await expect(page.locator('canvas')).toHaveAttribute('aria-hidden', 'true');
    await expect(page.locator('.dc-text-visual')).toHaveAttribute('aria-hidden', 'true');

    const semantic = page.locator('.dc-text-semantic');
    const headings = semantic.locator('[role="heading"]');
    await expect(headings.first()).toHaveAttribute('aria-level', '1');
    // Фикстура объявляет 4 H1 и 12 H2; заголовками могут стать и подписи
    // разделов, поэтому проверяем нижнюю границу, а не точное число.
    expect(await headings.count()).toBeGreaterThanOrEqual(16);
    await expect(semantic.locator('[role="paragraph"]').first()).not.toBeEmpty();
    await expect(semantic.locator('[role="table"]')).toHaveCount(3);
    await expect(semantic.locator('[role="columnheader"]').first()).toBeAttached();

    // Визуальный слой виртуализирован, но для видимой страницы spans есть.
    expect(await page.locator('.dc-text-line').count()).toBeGreaterThan(0);
    expect(errors).toEqual([]);
  });

  test('таблицы несут row/col и порядок ячеек', async ({ page }) => {
    await boot(page, '/docx', { a11y: true });
    await openFixture(page, 'a11y/a11y_audit.docx');

    const semantic = page.locator('.dc-text-semantic');
    const rows = semantic.locator('[role="table"]').first().locator('[role="row"]');
    await expect(rows.first()).toHaveAttribute('aria-rowindex', '1');
    const firstRow = rows.first();
    await expect(firstRow.locator('[role="columnheader"]').first()).toHaveAttribute(
      'aria-colindex',
      '1',
    );
    await expect(firstRow.locator('[role="columnheader"]').nth(1)).toHaveAttribute(
      'aria-colindex',
      '2',
    );
  });
});
