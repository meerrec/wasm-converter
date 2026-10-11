// Двухстраничный документ: счётчик страниц и переход между ними.
//
// Фикстура `basic/sections.docx` состоит из двух секций — портретной и
// ландшафтной A4, — поэтому страницы отличаются не только текстом, но и
// размером листа: по ширине нарисованной области видно, что кадр сменился.

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameId, openFixture, pageCount, waitForFrame } from '../helpers/docx_frame.js';
import { frameStats } from '../helpers/png.js';

test('кнопки переключают страницу и возвращают назад', async ({ page }) => {
  const errors = await boot(page, '/docx');

  const first = await openFixture(page, 'basic/sections.docx');
  expect(await pageCount(page)).toBe(2);
  await expect(page.locator('#page-info')).toHaveText('1 / 2');

  const before = await frameId(page);
  await page.click('#page-next');
  await waitForFrame(page, before);
  await expect(page.locator('#page-info')).toHaveText('2 / 2');

  const second = await frameStats(page);
  expect(second.inkBox).not.toBeNull();
  expect(second.signature).not.toBe(first.signature);
  // Ландшафтный лист шире портретного — и на холсте он шире.
  expect(first.pageBox).not.toBeNull();
  expect(second.pageBox).not.toBeNull();
  expect(second.pageBox!.width).toBeGreaterThan(first.pageBox!.width * 1.2);

  // Назад: возвращается тот же кадр, что был до перехода.
  const beforeBack = await frameId(page);
  await page.click('#page-prev');
  await waitForFrame(page, beforeBack);
  await expect(page.locator('#page-info')).toHaveText('1 / 2');

  const back = await frameStats(page);
  expect(back.signature).toBe(first.signature);

  expect(errors).toEqual([]);
});

test('прокрутка колесом переключает страницу', async ({ page }) => {
  const errors = await boot(page, '/docx');

  const first = await openFixture(page, 'basic/sections.docx');
  expect(await pageCount(page)).toBe(2);

  // Одного события мало: Firefox за одно событие колеса проходит меньше, чем
  // Chromium, поэтому докучиваем до второй страницы.
  const info = () =>
    page.evaluate(() => document.querySelector('#page-info')?.textContent ?? '');
  await page.mouse.move(400, 300);
  for (let i = 0; i < 8 && (await info()) !== '2 / 2'; i += 1) {
    await page.mouse.wheel(0, 600);
    await page.waitForTimeout(100);
  }
  await expect(page.locator('#page-info')).toHaveText('2 / 2');

  const second = await frameStats(page);
  expect(second.signature).not.toBe(first.signature);
  // Вторая секция — ландшафт: лист на холсте заметно шире портретного.
  expect(first.pageBox).not.toBeNull();
  expect(second.pageBox).not.toBeNull();
  expect(second.pageBox!.width).toBeGreaterThan(first.pageBox!.width * 1.2);
  // Лист белый, а не залитый чёрным после перерисовки.
  expect(second.lightRatio).toBeGreaterThan(0.9);

  expect(errors).toEqual([]);
});
