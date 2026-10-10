// Двухстраничный документ: счётчик страниц и переход между ними.
//
// Фикстура `basic/sections.docx` состоит из двух секций — портретной и
// ландшафтной A4, — поэтому страницы отличаются не только текстом, но и
// размером листа: по ширине нарисованной области видно, что кадр сменился.

import { expect, test } from '@playwright/test';
import { boot, frameId, frameStats, openFixture, pageCount, waitForFrame } from './canvas.js';

test('кнопки переключают страницу и возвращают назад', async ({ page, browserName }) => {
  // В WebKit `scrollTop` округляется вниз, а верх второй страницы лежит на
  // 1122.53 + 16 зазора = 1138.53: `showPage` ставит прокрутку на 1138, и
  // `pageAt` в `docx_viewer.ts` считает текущей прежнюю страницу — кадр
  // остаётся пустым. Это баг `@doc-converter/core`, а не примера: он описан в
  // отчёте слайса, чинить его здесь нельзя. Прокрутку колесом (соседний тест)
  // он не задевает — там прокрутка уходит за границу, а не встаёт на неё.
  test.skip(browserName === 'webkit', 'showPage: scrollTop округляется ниже верха страницы');

  const errors = await boot(page);

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
  const errors = await boot(page);

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
