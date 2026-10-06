// Гиперссылки листа доезжают до PDF аннотациями: внешние ссылки становятся
// действиями `/URI` (в том числе `mailto:`), внутренняя — `/GoTo` без URI,
// потому что ведёт внутрь документа, а не в сеть.
//
// В `layout-links.xlsx` ровно четыре ссылки: две внешние https, одна почтовая
// и одна внутренняя (`#Лист2!A1`), поэтому проверяются конкретные числа:
// на одной случайно уцелевшей аннотации такой тест проходить не должен.

import { expect, test } from '@playwright/test';
import { boot } from './canvas.js';
import { downloadPdf } from './download.js';
import { linkAnnotations, probePdf } from './pdf-probe.js';

test('переносит гиперссылки листа в PDF аннотациями', async ({ page }) => {
  const errors = await boot(page);

  await page.selectOption('#fixture', 'layout-links.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const { bytes, name } = await downloadPdf(page, '#export-pdf');
  expect(name).toBe('layout-links.pdf');
  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });

  const links = linkAnnotations(probePdf(bytes).text);
  expect(links, 'всего аннотаций-ссылок').toHaveLength(4);
  expect(links.filter((link) => link.uri?.startsWith('http')).length, 'внешних https-ссылок').toBe(2);
  expect(links.filter((link) => link.uri?.startsWith('mailto:')).length, 'почтовых ссылок').toBe(1);
  // У внутренней ссылки цели за пределами документа нет: `uri === null`.
  expect(links.filter((link) => link.uri === null).length, 'внутренних ссылок').toBe(1);

  expect(errors).toEqual([]);
});
