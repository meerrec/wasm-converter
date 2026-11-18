// Временный сквозной смоук OOXML-редактора: команда ленты меняет пакет,
// «Сохранить» отдаёт настоящий файл. Файл удаляется после прогона.

import { readFile } from 'node:fs/promises';
import JSZip from 'jszip';
import { expect, test } from '@playwright/test';
import { boot } from './helpers/boot.js';

async function downloadBytes(page: import('@playwright/test').Page, selector: string): Promise<Buffer> {
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.locator(selector).click(),
  ]);
  const path = await download.path();
  if (path === null) throw new Error('браузер не отдал путь загрузки');
  return readFile(path);
}

test('XLSX: полужирный применяется и сохраняется в XLSX', async ({ page }) => {
  const errors = await boot(page, '/xlsx');
  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const viewer = await page.locator('#viewer').boundingBox();
  if (viewer === null) throw new Error('нет холста');
  await page.mouse.move(viewer.x + viewer.width / 2, viewer.y + viewer.height / 2);
  await expect(page.locator('#cell')).not.toHaveText('—');

  await page.getByLabel('Полужирный').click();
  await expect(page.locator('#status')).not.toContainText('не выполнена', { timeout: 15_000 });

  await page.getByRole('tab', { name: 'Файл' }).click();
  const bytes = await downloadBytes(page, '#save');
  const zip = await JSZip.loadAsync(bytes);
  const styles = await zip.file('xl/styles.xml')?.async('string');
  expect(styles).toContain('<b/');
  expect(errors).toEqual([]);
});

test('PPTX: новый слайд сохраняется в PPTX', async ({ page }) => {
  const errors = await boot(page, '/pptx');
  await expect(page.locator('#slide-info')).toHaveText('1 / 2', { timeout: 30_000 });

  await page.getByRole('button', { name: 'Создать слайд' }).click();
  await expect(page.locator('#slide-info')).toHaveText('3 / 3', { timeout: 15_000 });

  await page.getByRole('tab', { name: 'Файл' }).click();
  const bytes = await downloadBytes(page, '#save');
  const zip = await JSZip.loadAsync(bytes);
  const slides = Object.keys(zip.files).filter((name) => /^ppt\/slides\/slide\d+\.xml$/.test(name));
  expect(slides).toHaveLength(3);
  expect(errors).toEqual([]);
});
