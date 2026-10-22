// Pixel-diff: text-layer не должен менять canvas.
//
// Сравниваем один и тот же кадр с a11y off/on в одном браузере и прогоне:
// кросс-браузерные эталоны не используем, растеризация шрифтов у них разная.
// Допуск 0.1 % в постановке; здесь сравнение строже — бит-в-бит.

import { expect, test } from '@playwright/test';

import { boot } from '../helpers/boot.js';
import { openFixture } from '../helpers/docx_frame.js';

async function png(page: import('@playwright/test').Page): Promise<number[]> {
  return page.evaluate(async () => {
    const bridge = window.docConverter;
    if (bridge?.kind !== 'docx') throw new Error('нет DOCX-вьюера');
    const bytes = await bridge.viewer.exportPng();
    return [...bytes];
  });
}

test('a11y-слой не меняет ни одного пикселя canvas', async ({ page }) => {
  await boot(page, '/docx');
  await openFixture(page, 'a11y/a11y_audit.docx');
  const withoutA11y = await png(page);

  // Второе открытие — в отдельной вкладке: перезагрузка того же URL с
  // `?a11y=1` в WebKit иногда не поднимает приложение.
  const other = await page.context().newPage();
  try {
    await boot(other, '/docx', { a11y: true });
    await openFixture(other, 'a11y/a11y_audit.docx');
    await expect(other.locator('.dc-text-layer')).toHaveCount(1);
    const withA11y = await png(other);

    expect(withA11y.length).toBe(withoutA11y.length);
    expect(withA11y).toEqual(withoutA11y);
  } finally {
    await other.close();
  }
});
