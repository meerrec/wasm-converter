import { expect, test, type Page } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { colorShare } from '../helpers/png.js';

const PRIMARY = process.platform === 'darwin' ? 'Meta' : 'Control';

async function openBook(page: Page, fixture = 'content-table.xlsx'): Promise<string[]> {
  const errors = await boot(page, '/xlsx');
  await page.selectOption('#fixture', fixture);
  await expect(page.locator('#status')).toContainText('лист', { timeout: 30_000 });
  await expect(page.locator('#cell')).toHaveValue('A1');
  return errors;
}

async function selection(page: Page) {
  return page.evaluate(() => {
    const bridge = window.docConverter;
    if (bridge?.kind !== 'xlsx') throw new Error('ожидался XLSX-вьюер');
    return bridge.viewer.getSelection();
  });
}

async function canvasBox(page: Page) {
  const box = await page.locator('#viewer canvas').boundingBox();
  if (!box) throw new Error('нет canvas');
  return box;
}

test('drag, Ctrl+click и Shift+Arrow меняют диапазоны и рисуют акцент', async ({ page }) => {
  const errors = await openBook(page);
  const box = await canvasBox(page);

  await page.mouse.move(box.x + 50, box.y + 30);
  await page.mouse.down();
  await page.mouse.move(box.x + 130, box.y + 55, { steps: 5 });
  await page.mouse.up();
  await expect(page.locator('#cell')).toHaveValue('A1:B2');
  expect((await selection(page))?.ranges[0]).toEqual({
    start: { row: 0, col: 0 },
    end: { row: 1, col: 1 },
  });
  expect(await colorShare(page, [33, 115, 70], 12)).toBeGreaterThan(0);

  await page.keyboard.down(PRIMARY);
  await page.mouse.click(box.x + 250, box.y + 75);
  await page.keyboard.up(PRIMARY);
  await expect.poll(async () => (await selection(page))?.ranges.length).toBe(2);

  await page.keyboard.press('Shift+ArrowRight');
  await expect.poll(async () => (await selection(page))?.ranges[1]?.end.col).toBe(4);
  await expect(page.locator('#viewer > div')).toHaveAttribute('role', 'grid');
  await expect(page.locator('#viewer > div')).toHaveAttribute('aria-multiselectable', 'true');

  expect(errors).toEqual([]);
});

test('заголовки и повторный Ctrl+A выделяют строку и весь лист', async ({ page }) => {
  const errors = await openBook(page);
  const box = await canvasBox(page);

  await page.mouse.click(box.x + 20, box.y + 30);
  await expect.poll(async () => (await selection(page))?.ranges[0]?.end)
    .toEqual({ row: 0, col: 16_383 });
  let current = await selection(page);
  expect(current?.ranges[0]?.start).toEqual({ row: 0, col: 0 });

  await page.keyboard.press(`${PRIMARY}+a`);
  current = await selection(page);
  expect(current?.ranges[0]?.end).not.toEqual({ row: 1_048_575, col: 16_383 });

  await page.keyboard.press(`${PRIMARY}+a`);
  await expect.poll(async () => (await selection(page))?.ranges[0]?.end)
    .toEqual({ row: 1_048_575, col: 16_383 });
  current = await selection(page);

  await page.mouse.click(box.x + 10, box.y + 10);
  current = await selection(page);
  expect(current?.active).toEqual({ row: 0, col: 0 });
  expect(current?.ranges[0]?.end).toEqual({ row: 1_048_575, col: 16_383 });

  expect(errors).toEqual([]);
});

test('Name Box достигает XFD1048576 и Ctrl+Home возвращает A1', async ({ page }) => {
  const errors = await openBook(page);

  await page.locator('#cell').fill('XFD1048576');
  await page.locator('#cell').press('Enter');
  await expect(page.locator('#cell')).toHaveValue('XFD1048576');
  expect((await selection(page))?.active).toEqual({ row: 1_048_575, col: 16_383 });
  await expect.poll(async () => page.locator('#viewer > div').evaluate((element) => element.scrollLeft))
    .toBeGreaterThan(0);
  await expect.poll(async () => page.locator('#viewer > div').evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
  const scroll = await page.locator('#viewer > div').evaluate((element) => ({
    width: element.scrollWidth,
    height: element.scrollHeight,
  }));
  expect(scroll.width).toBeLessThanOrEqual(8_000_800);
  expect(scroll.height).toBeLessThanOrEqual(8_000_600);

  await page.locator('#viewer > div').focus();
  await page.keyboard.press(`${PRIMARY}+Home`);

  await expect(page.locator('#cell')).toHaveValue('A1');
  expect((await selection(page))?.active).toEqual({ row: 0, col: 0 });

  await page.keyboard.press('PageDown');
  await expect.poll(async () => (await selection(page))?.active.row).toBeGreaterThan(0);
  await page.keyboard.press('PageUp');
  await expect(page.locator('#cell')).toHaveValue('A1');

  expect(errors).toEqual([]);
});

test('каждый лист сохраняет своё выделение', async ({ page }) => {
  const errors = await openBook(page, 'sheets-three.xlsx');
  const box = await canvasBox(page);

  await page.mouse.click(box.x + 130, box.y + 55);
  await expect(page.locator('#cell')).toHaveValue('B2');
  await page.selectOption('#sheet', '1');
  await expect(page.locator('#cell')).toHaveValue('A1');
  await page.selectOption('#sheet', '2');
  await expect(page.locator('#cell')).toHaveValue('A1');
  await page.selectOption('#sheet', '0');
  await expect(page.locator('#cell')).toHaveValue('B2');

  expect(errors).toEqual([]);
});
