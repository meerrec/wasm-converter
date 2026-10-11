// Скачивание файла кнопкой панели: общее для спеков об экспорте.
//
// Проверки формата живут здесь же: спекам остаётся имя файла и содержимое,
// а «байты вообще похожи на свой формат» проверяется один раз.

import { readFile } from 'node:fs/promises';
import { expect, type Page } from '@playwright/test';

/** Скачанный файл: байты и имя, которое предложил браузер. */
export interface DownloadedFile {
  bytes: Buffer;
  name: string;
}

/** Скачанный текст: байты плюс строка, декодированная из них как UTF-8. */
export interface DownloadedText extends DownloadedFile {
  text: string;
}

/** Нажать кнопку и дождаться скачивания: подписка ставится до клика. */
async function clickAndDownload(page: Page, selector: string): Promise<DownloadedFile> {
  // Скачивание может начаться сразу после клика, поэтому подписка — заранее.
  const downloadPromise = page.waitForEvent('download');
  await page.click(selector);
  const download = await downloadPromise;
  const path = await download.path();
  expect(path).not.toBeNull();
  return { bytes: await readFile(path!), name: download.suggestedFilename() };
}

/** Скачать PDF: файл непустой и начинается с сигнатуры формата. */
export async function downloadPdf(page: Page, selector: string): Promise<DownloadedFile> {
  const file = await clickAndDownload(page, selector);
  expect(file.bytes.byteLength).toBeGreaterThan(500);
  expect(file.bytes.subarray(0, 5).toString('latin1')).toBe('%PDF-');
  return file;
}

/** Скачать текстовый формат: байты обязаны быть корректным UTF-8. */
export async function downloadText(page: Page, selector: string): Promise<DownloadedText> {
  const file = await clickAndDownload(page, selector);
  // `fatal: true` — битый UTF-8 должен валить тест, а не молча превращаться
  // в U+FFFD: иначе «экспорт есть» прошло бы и на испорченной кодировке.
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(file.bytes);
  } catch (e) {
    throw new Error(
      `${file.name} не декодируется как UTF-8: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  return { ...file, text };
}
