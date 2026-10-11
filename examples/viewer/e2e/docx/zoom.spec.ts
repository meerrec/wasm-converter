// Зум: нарисованное должно расти и уменьшаться вместе с множителем.

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameId, openFixture, waitForFrame } from '../helpers/docx_frame.js';
import { frameStats } from '../helpers/png.js';

test('зум меняет размер нарисованного', async ({ page }) => {
  const errors = await boot(page, '/docx');

  const at100 = await openFixture(page, 'simple/one_paragraph.docx');
  const ink100 = at100.inkBox;
  expect(ink100).not.toBeNull();
  expect(ink100!.width).toBeGreaterThan(10);

  const before = await frameId(page);
  // В объединённом примере значения списка масштаба — подписи как в Office
  // («200 %»), а не множители: множитель выводится из подписи в `DocxPage.tsx`.
  await page.selectOption('#zoom', '200 %');
  await waitForFrame(page, before);

  const at200 = await frameStats(page);
  const ink200 = at200.inkBox;
  expect(ink200).not.toBeNull();
  // Тот же текст занимает вдвое больше пикселей — и по ширине, и по высоте.
  expect(ink200!.width / ink100!.width).toBeGreaterThan(1.6);
  expect(ink200!.height / ink100!.height).toBeGreaterThan(1.6);
  // Лист вырос вместе с текстом: верхнее поле страницы тоже удвоилось.
  expect(ink200!.y).toBeGreaterThan(ink100!.y * 1.6);

  // И обратно: на половине зума текста становится меньше, чем на целом.
  const beforeHalf = await frameId(page);
  await page.selectOption('#zoom', '50 %');
  await waitForFrame(page, beforeHalf);

  const at50 = await frameStats(page);
  expect(at50.inkBox).not.toBeNull();
  expect(at50.inkBox!.width).toBeLessThan(ink100!.width);

  expect(errors).toEqual([]);
});
