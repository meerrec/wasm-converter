// Сквозная проверка DOCX-просмотрщика без OffscreenCanvas.
//
// `transferControlToOffscreen` удалён до загрузки страницы: режим `auto`
// обязан перейти на main и нарисовать страницу локальным painter'ом. Признаки
// те же, что у обычного пути: ответ просмотрщика, команды кадра и заметная
// доля непрозрачных пикселей листа.

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameStats } from '../helpers/png.js';

test('рисует страницу на main-потоке, когда OffscreenCanvas недоступен', async ({ page }) => {
  await page.addInitScript(() => {
    delete (
      HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }
    ).transferControlToOffscreen;
  });

  const errors = await boot(page, '/docx');

  await page.selectOption('#fixture', 'simple/one_paragraph.docx');
  // Раскладка готова: документ открылся и painter получил кадр.
  await expect(page.locator('#cmds')).not.toHaveText('0', { timeout: 30_000 });

  const stats = await frameStats(page);
  expect(stats.width).toBeGreaterThan(200);
  expect(stats.height).toBeGreaterThan(200);
  // Лист занимает заметную часть кадра: рисовал именно painter, а не пустой холст.
  expect(stats.pageRatio).toBeGreaterThan(0.2);
  expect(stats.diffRatio).toBeGreaterThan(0.02);

  expect(errors).toEqual([]);
});
