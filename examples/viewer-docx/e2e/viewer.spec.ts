// Сквозная проверка просмотрщика DOCX в настоящем браузере.
//
// Проверяем не «страница открылась», а что на холсте действительно белый лист
// с текстом: воркер отдаёт PNG, браузер его декодирует, тест смотрит пиксели.

import { expect, test } from '@playwright/test';
import { boot, openFixture, pageCount } from './canvas.js';

test('открывает документ и рисует страницу с текстом', async ({ page }) => {
  const errors = await boot(page);

  const stats = await openFixture(page, 'simple/one_paragraph.docx');
  expect(await pageCount(page)).toBe(1);
  await expect(page.locator('#page-info')).toHaveText('1 / 1');

  // Лист не занимает холст целиком: по ширине окно шире страницы, по высоте
  // страница выше окна, поэтому прозрачный фон по краям остаётся.
  expect(stats.pageRatio).toBeGreaterThan(0.2);
  expect(stats.pageRatio).toBeLessThan(0.95);
  // Поле листа белое, текст на нём тёмный — и того и другого в меру.
  expect(stats.lightRatio).toBeGreaterThan(0.9);
  expect(stats.inkRatio).toBeGreaterThan(0.0005);
  expect(stats.inkRatio).toBeLessThan(0.2);
  expect(stats.inkBox).not.toBeNull();

  expect(errors).toEqual([]);
});
