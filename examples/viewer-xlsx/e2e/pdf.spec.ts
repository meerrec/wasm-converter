// Сквозная проверка экспорта в PDF: кнопка в панели примера отдаёт файл
// на скачивание.
//
// Проверяем не «клик сработал», а что воркер действительно собрал PDF:
// файл скачался, начинается с сигнатуры %PDF- и не пустой.

import { readFile } from 'node:fs/promises';
import { expect, test, type Page } from '@playwright/test';

/** Открыть пример и дождаться, пока вьюер поднимется. */
async function boot(page: Page): Promise<string[]> {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });

  await page.goto('/');
  await page.waitForFunction(() => window.docConverter !== undefined);
  return errors;
}

test('экспортирует открытый лист в PDF', async ({ page, browserName }) => {
  const errors = await boot(page);

  // Кнопка оживает только вместе с книгой.
  await expect(page.locator('#export-pdf')).toBeDisabled();

  await page.selectOption('#fixture', 'text-cyrillic-wrap.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });
  await expect(page.locator('#export-pdf')).toBeEnabled();

  // Подписываемся до клика: скачивание может начаться сразу же.
  const downloadPromise = page.waitForEvent('download');
  await page.click('#export-pdf');
  const download = await downloadPromise;

  // Имя файла — из подписи открытого документа, расширение заменено на .pdf.
  expect(download.suggestedFilename()).toBe('text-cyrillic-wrap.pdf');

  const path = await download.path();
  expect(path).not.toBeNull();
  const bytes = await readFile(path!);

  expect(bytes.byteLength).toBeGreaterThan(500);
  expect(bytes.subarray(0, 5).toString('latin1')).toBe('%PDF-');

  if (browserName === 'chromium') {
    // Страховка от «экспорт зациклился и раздул файл».
    expect(bytes.byteLength).toBeLessThan(5 * 1024 * 1024);
  }

  await expect(page.locator('#status')).toContainText('PDF готов', { timeout: 30_000 });
  // После экспорта кнопка снова рабочая: можно выгрузить ещё раз.
  await expect(page.locator('#export-pdf')).toBeEnabled();

  expect(errors).toEqual([]);
});
