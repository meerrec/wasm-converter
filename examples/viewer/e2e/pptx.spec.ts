// Сквозная проверка просмотрщика PPTX в настоящем браузере.
//
// Страница `#/pptx` открывает фикстуру сама, поэтому проверяются факты
// просмотра: счётчик слайдов, навигация и зум без ошибок воркера.

import { expect, test } from '@playwright/test';
import { boot } from './helpers/boot.js';

test('открывает презентацию и листает слайды', async ({ page }) => {
  const errors = await boot(page, '/pptx');

  await expect(page.locator('#slide-info')).toHaveText('1 / 2');
  await page.locator('#slide-next').click();
  await expect(page.locator('#slide-info')).toHaveText('2 / 2');
  await page.locator('#slide-prev').click();
  await expect(page.locator('#slide-info')).toHaveText('1 / 2');

  // Зум меняется и не роняет воркер.
  await page.locator('#zoom').selectOption('2');
  await expect(page.locator('#slide-info')).toHaveText('1 / 2');

  expect(errors).toEqual([]);
});

test('экспорт текста работает', async ({ page }) => {
  await boot(page, '/pptx');
  await expect(page.locator('#slide-info')).toHaveText('1 / 2');

  // Скачивание перехватывается: клик по «Текст» отдаёт файл markdown.
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.locator('#export-text').click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/\.md$/);
});
