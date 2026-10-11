// Регрессия: страница чернела со второго кадра.
//
// Кэш стилей painter'а расходился с состоянием контекста: `restore()` на конце
// кадра возвращал кисти цвет из `save()`, а кэш продолжал считать, что заливка
// уже выставлена. На следующем кадре белая заливка листа пропускалась, и лист
// заливался чёрным — цветом, который контекст вернул из `save()`.
//
// Проверяем именно повторную отрисовку и без изменения размера холста: resize
// сбрасывает состояние painter'а и замаскировал бы регрессию.

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameId, openFixture, waitForFrame } from '../helpers/docx_frame.js';
import { frameStats } from '../helpers/png.js';

test('кадр остаётся белым после повторной отрисовки', async ({ page }) => {
  const errors = await boot(page, '/docx');

  const first = await openFixture(page, 'simple/one_paragraph.docx');
  expect(first.lightRatio).toBeGreaterThan(0.9);

  // Прокрутка просит новый кадр: между кадрами состояние painter'а сохраняется
  // (сбрасывает его только resize), поэтому второй кадр рисуется тем же
  // контекстом, что и первый.
  const before = await frameId(page);
  await page.evaluate(() => window.docConverter?.viewer.scrollTo(0, 0));
  await waitForFrame(page, before);

  const second = await frameStats(page);
  // Лист по-прежнему белый с тёмным текстом, а не чёрный с чёрным.
  expect(second.lightRatio).toBeGreaterThan(0.9);
  expect(second.inkRatio).toBeGreaterThan(0.0005);
  expect(second.inkRatio).toBeLessThan(0.2);

  // Ещё кадр: регрессия проявлялась на каждом кадре после первого.
  const beforeThird = await frameId(page);
  await page.evaluate(() => window.docConverter?.viewer.scrollTo(0, 0));
  await waitForFrame(page, beforeThird);

  const third = await frameStats(page);
  expect(third.lightRatio).toBeGreaterThan(0.9);
  // Кадр тот же самый: перерисовка не меняет нарисованное.
  expect(third.signature).toBe(second.signature);

  expect(errors).toEqual([]);
});
