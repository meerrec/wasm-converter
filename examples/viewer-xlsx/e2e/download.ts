// Скачивание PDF кнопкой панели: общее для спеков о новых артефактах.

import { readFile } from 'node:fs/promises';
import { expect, type Page } from '@playwright/test';

/** Скачанный файл: байты и имя, которое предложил браузер. */
export interface DownloadedPdf {
  bytes: Buffer;
  name: string;
}

/** Нажать кнопку и дождаться скачивания: подписка ставится до клика. */
export async function downloadPdf(page: Page, selector: string): Promise<DownloadedPdf> {
  const downloadPromise = page.waitForEvent('download');
  await page.click(selector);
  const download = await downloadPromise;
  const path = await download.path();
  expect(path).not.toBeNull();
  const bytes = await readFile(path!);

  expect(bytes.byteLength).toBeGreaterThan(500);
  expect(bytes.subarray(0, 5).toString('latin1')).toBe('%PDF-');
  return { bytes, name: download.suggestedFilename() };
}
