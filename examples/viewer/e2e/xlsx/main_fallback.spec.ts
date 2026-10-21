// Сквозная проверка XLSX-просмотрщика без OffscreenCanvas.
//
// Браузеру отключают `transferControlToOffscreen` до загрузки страницы: режим
// `auto` обязан молча перейти на main, нарисовать кадр локальным painter'ом и
// не оставить ошибок. Ждём те же признаки, что и в обычной спеке: ответ
// просмотрщика, команды кадра и ненулевую долю «чернил».

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameStats } from '../helpers/png.js';

test('рисует таблицу на main-потоке, когда OffscreenCanvas недоступен', async ({ page }) => {
  // Удаляем именно ту возможность, которой пользуется воркер-путь, — так
  // страница выглядит для старого браузера.
  await page.addInitScript(() => {
    delete (
      HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }
    ).transferControlToOffscreen;
  });

  const errors = await boot(page, '/xlsx');

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  // Кадр дошёл до painter'а на main: команды и картинка на месте.
  await expect(page.locator('#cmds')).not.toHaveText('0');

  const stats = await frameStats(page);
  expect(stats.width).toBeGreaterThan(200);
  expect(stats.height).toBeGreaterThan(200);
  expect(stats.diffRatio).toBeGreaterThan(0.02);

  expect(errors).toEqual([]);
});
