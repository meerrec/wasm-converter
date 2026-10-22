// Сквозная проверка просмотрщика в настоящем браузере.
//
// Проверяем не «страница открылась», а что кадр действительно нарисован:
// воркер отдаёт PNG, браузер его декодирует, и по пикселям видно, что на
// холсте есть текст и сетка, а не ровная заливка.

import { expect, test } from '@playwright/test';
import { boot } from '../helpers/boot.js';
import { frameStats, pixelAt } from '../helpers/png.js';

test('рисует таблицу: текст, числа и сетка', async ({ page }) => {
  const errors = await boot(page, '/xlsx');

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  // Кадр собран: команды дошли до painter'а.
  await expect(page.locator('#cmds')).not.toHaveText('0');

  const stats = await frameStats(page);
  expect(stats.width).toBeGreaterThan(200);
  expect(stats.height).toBeGreaterThan(200);
  // Сетка и текст: пиксели отличаются от фона, и цветов заметно больше двух.
  expect(stats.diffRatio).toBeGreaterThan(0.02);
  expect(stats.colors).toBeGreaterThan(8);

  expect(errors).toEqual([]);
});

test('переключает листы и прокручивает лист', async ({ page }) => {
  const errors = await boot(page, '/xlsx');

  await page.selectOption('#fixture', 'sheets-three.xlsx');
  await expect(page.locator('#status')).toContainText('3 лист', { timeout: 30_000 });
  await expect(page.locator('#sheet option')).toHaveCount(3);

  const first = await frameStats(page);
  await page.selectOption('#sheet', '2');
  await expect(page.locator('#cmds')).not.toHaveText('0');

  // Разные листы — разные картинки.
  await page.waitForTimeout(200);
  const third = await frameStats(page);
  expect(third.diffRatio).toBeGreaterThan(0);
  expect(third.diffRatio === first.diffRatio && third.colors === first.colors).toBe(false);

  expect(errors).toEqual([]);
});

test('закреплённые строки остаются на месте при прокрутке', async ({ page }) => {
  // Окно намеренно низкое: лист из десяти строк должен не помещаться целиком,
  // иначе прокручивать нечего. Ширина — с запасом, иначе панель инструментов
  // переносится на несколько строк и съедает всю высоту. Высота тоже с запасом
  // на эту панель: при 240 у вьюера остаётся полоса в 28 px, в которой сдвиг
  // строк не виден вовсе и кадр до и после прокрутки совпадает побайтово.
  await page.setViewportSize({ width: 900, height: 320 });
  const errors = await boot(page, '/xlsx');

  await page.selectOption('#fixture', 'layout-frozen-rows.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const before = await frameStats(page);
  const scrollable = await page.evaluate(() => {
    const scroller = document.querySelector('#viewer > div') as HTMLElement;
    const canScroll = scroller.scrollHeight > scroller.clientHeight;
    scroller.scrollTop = 200;
    return { canScroll, scrollTop: scroller.scrollTop };
  });
  expect(scrollable.canScroll, 'лист должен не помещаться в окно').toBe(true);
  expect(scrollable.scrollTop).toBeGreaterThan(0);
  await expect(page.locator('#frame')).not.toHaveText('0');
  await page.waitForTimeout(300);
  const after = await frameStats(page);

  // Картинка изменилась, но не пропала: закреплённая строка с заголовком
  // осталась, а под ней поехали данные. Смену кадра ловим отпечатком: при
  // вертикальном сдвиге одинаковых фигур гистограмма цветов не меняется.
  expect(after.diffRatio).toBeGreaterThan(0.005);
  expect(after.signature, 'прокрутка меняет кадр').not.toBe(before.signature);

  expect(errors).toEqual([]);
});

test('зовёт ячейку по имени под курсором', async ({ page }) => {
  const errors = await boot(page, '/xlsx');

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const box = await page.locator('#viewer > div').boundingBox();
  expect(box).not.toBeNull();
  // Точка внутри первой ячейки данных: заголовок столбцов 20 px, строк 44 px.
  await page.mouse.move(box!.x + 50, box!.y + 30);
  await expect(page.locator('#cell')).toHaveText('A1');

  expect(errors).toEqual([]);
});

test('без сетки и заголовков кадр проще', async ({ page }) => {
  const errors = await boot(page, '/xlsx');

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  // Левый верхний угол занят полосой заголовков — она серого цвета.
  await expect(page.locator('#cmds')).not.toHaveText('0');
  const withHeaders = await pixelAt(page, 2, 2);
  expect(withHeaders).toEqual([245, 245, 245]);

  const withChrome = await frameStats(page);

  await page.uncheck('#grid');
  await page.uncheck('#headers');
  await page.waitForFunction(
    (before) => document.querySelector('#cmds')?.textContent !== before,
    String(withChrome.colors),
  );
  await page.waitForTimeout(200);

  // Заголовков нет — угол стал фоном листа, и лишних цветов поубавилось.
  const bare = await pixelAt(page, 2, 2);
  expect(bare).toEqual([255, 255, 255]);

  expect(errors).toEqual([]);
});

test('переключатель fitToPage доступен до и после открытия книги', async ({ page }) => {
  const errors = await boot(page, '/xlsx');

  // До открытия книги экспорт недоступен — гасим и переключатель.
  await expect(page.locator('#fit-to-page')).toBeDisabled();

  await page.selectOption('#fixture', 'content-table.xlsx');
  await expect(page.locator('#status')).toContainText('1 лист', { timeout: 30_000 });

  const fit = page.locator('#fit-to-page');
  await expect(fit).toBeEnabled();
  await expect(fit).not.toBeChecked();
  await fit.check();
  await expect(fit).toBeChecked();

  expect(errors).toEqual([]);
});
